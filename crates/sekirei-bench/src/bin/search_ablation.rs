//! search_ablation — fair A/B/C/D/E comparison of PVS/YBW/speculation.
//!
//! Arms (single source of truth: `ARMS` below):
//!   A seq-AB           PVS off, YBW off, speculation off
//!   B seq-PVS          PVS on,  YBW off, speculation off
//!   C PVS+YBW          PVS on,  YBW on,  speculation off
//!   D PVS+YBW+spec     PVS on,  YBW on,  speculation on
//!   E PVS+spec         PVS on,  YBW off, speculation on
//!
//! Two tuning profiles (see `Profile`):
//!   production  — killer/history/countermove ordering as shipped
//!   controlled  — killer/history/countermove ordering disabled (via
//!                 `SearchTuning`, internal-only, no USI exposure), to
//!                 isolate YBW/PVS/speculation's own search-tree effect
//!                 from a separate, unrelated, pre-existing non-determinism:
//!                 those tables are shared (via atomics) across YBW
//!                 siblings, so real concurrency can race move ordering and
//!                 change the searched value across identical repeated
//!                 runs. `controlled` is NOT a strength-measurement
//!                 condition -- it exists purely to get a cleaner read on
//!                 search efficiency. See Commit 5's report for the
//!                 nondeterminism-audit numbers this is based on.
//!
//! Every process invocation loads NNUE weights exactly once (or falls back
//! to the engine's built-in default), so "same NNUE weights" holds
//! automatically across every arm/profile/thread-count measured by that
//! invocation. Every individual measurement gets a fresh `Tt` (and,
//! because `Searcher`/`SpeculativeSearcher` always construct fresh killer/
//! history/countermove tables per `search()` call, those are fresh too).
//!
//! Usage:
//!   cargo run -p sekirei-bench --release --bin search_ablation -- smoke
//!   cargo run -p sekirei-bench --release --bin search_ablation -- audit
//!   cargo run -p sekirei-bench --release --bin search_ablation -- fixed-depth
//!   cargo run -p sekirei-bench --release --bin search_ablation -- fixed-time
//! Run with `--help` for the full option list.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::mv::Move;
use sekirei_core::nnue;
use sekirei_core::search::bench_api::SearchTuning;
use sekirei_core::search::{
    SearchConfig, Searcher, SpecSearchInfo, SpeculativeSearcher, YbwSearchStats,
};
use sekirei_core::sfen::move_to_usi;
use sekirei_core::tt::Tt;

const SCHEMA_VERSION: &str = "1";
const DEFAULT_CORPUS: &str = "benchmarks/corpora/search_ablation_v1.tsv";
const CORPUS_VERSION: &str = "search_ablation_v1";

// ============================================================
// Small deterministic helpers (no external crates: no `rand`, no hash crate)
// ============================================================

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn hex_hash(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a(bytes))
}

fn xorshift64(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// Deterministic Fisher-Yates shuffle, seeded, so a (position, iteration)
/// unit's arm execution order varies (avoiding systematic CPU-thermal/cache
/// bias from always running A,B,C,D,E in the same order) while remaining
/// fully reproducible from the recorded seed.
fn shuffled_arm_order(seed: u64) -> ([usize; 5], u64) {
    let mut order = [0usize, 1, 2, 3, 4];
    let mut s = seed;
    for i in (1..order.len()).rev() {
        let j = (xorshift64(&mut s) as usize) % (i + 1);
        order.swap(i, j);
    }
    (order, seed)
}

fn position_shuffle_seed(base_seed: u64, position_id: &str, iteration: u64) -> u64 {
    base_seed ^ fnv1a(position_id.as_bytes()) ^ iteration.wrapping_mul(0x9E3779B97F4A7C15)
}

// ============================================================
// Arms (single source of truth)
// ============================================================

#[derive(Clone, Copy)]
struct Arm {
    id: &'static str,
    label: &'static str,
    use_pvs: bool,
    use_ybw: bool,
    use_speculation: bool,
}

const ARMS: [Arm; 5] = [
    Arm {
        id: "A",
        label: "seq-AB",
        use_pvs: false,
        use_ybw: false,
        use_speculation: false,
    },
    Arm {
        id: "B",
        label: "seq-PVS",
        use_pvs: true,
        use_ybw: false,
        use_speculation: false,
    },
    Arm {
        id: "C",
        label: "PVS+YBW",
        use_pvs: true,
        use_ybw: true,
        use_speculation: false,
    },
    Arm {
        id: "D",
        label: "PVS+YBW+spec",
        use_pvs: true,
        use_ybw: true,
        use_speculation: true,
    },
    Arm {
        id: "E",
        label: "PVS+spec",
        use_pvs: true,
        use_ybw: false,
        use_speculation: true,
    },
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Profile {
    Production,
    Controlled,
}

impl Profile {
    /// `ybw_early_cancel` is a second, independent dimension (used by the
    /// `cancel-ablation` phase to isolate Commit 3's actual node/time
    /// savings); every other phase always passes `true` (shipped behavior).
    fn tuning(self, ybw_early_cancel: bool) -> SearchTuning {
        SearchTuning {
            heuristic_move_ordering: self == Profile::Production,
            ybw_early_cancel,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Profile::Production => "production",
            Profile::Controlled => "controlled",
        }
    }
}

// ============================================================
// Position corpus
// ============================================================

#[derive(Clone)]
struct Position {
    id: String,
    category: String,
    sfen: String,
}

fn load_corpus(path: &Path) -> std::io::Result<(Vec<Position>, String)> {
    let content = fs::read_to_string(path)?;
    let corpus_hash = hex_hash(content.as_bytes());
    let mut positions = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() != 3 {
            eprintln!("preflight: skipping malformed corpus line: {line:?}");
            continue;
        }
        positions.push(Position {
            id: parts[0].to_string(),
            category: parts[1].to_string(),
            sfen: parts[2].to_string(),
        });
    }
    Ok((positions, corpus_hash))
}

/// Reject invalid SFEN or already-terminal (checkmate/stalemate) positions.
/// Returns (accepted, rejection reasons for reporting).
fn preflight(positions: Vec<Position>) -> (Vec<Position>, Vec<String>) {
    let mut ok = Vec::new();
    let mut rejected = Vec::new();
    for p in positions {
        match Board::from_sfen(&p.sfen) {
            Err(e) => rejected.push(format!("{}: invalid SFEN ({e})", p.id)),
            Ok(mut b) => {
                if generate_legal_moves(&mut b).is_empty() {
                    rejected.push(format!(
                        "{}: no legal moves (terminal/already mated) -- unsuitable as a search-start position",
                        p.id
                    ));
                } else {
                    ok.push(p);
                }
            }
        }
    }
    (ok, rejected)
}

// ============================================================
// One search invocation, unified across Searcher/SpeculativeSearcher
// ============================================================

struct RunOutcome {
    best_move: Option<Move>,
    score: i32,
    completed_depth: u32,
    main_nodes: u64,
    spec_nodes: u64,
    elapsed: Duration,
    hashfull: u32,
    bestmove_changes: u32,
    pv: Vec<Move>,
    ybw: YbwSearchStats,
    spec_tasks_started: u64,
    spec_tasks_completed: u64,
    spec_tasks_cancelled: u64,
    board_unchanged: bool,
    hash_unchanged: bool,
    accumulator_unchanged: bool,
    bestmove_legal: bool,
    pv_legal: bool,
    pv_head_matches_bestmove: bool,
}

fn spec_info_fields(info: &SpecSearchInfo) -> (u64, u64, u64) {
    (
        info.spec_tasks_started,
        info.spec_tasks_completed,
        info.spec_tasks_cancelled,
    )
}

/// Run one arm once, at one profile/thread-count, against one position, for
/// either a fixed depth (`time_limit = None`, with a generous safety cap so
/// a pathological position can't hang the whole sweep) or a fixed time
/// budget (`max_depth` set high so the search is always time- not depth-
/// bounded). Verifies every fairness/correctness invariant inline.
fn run_one(
    sfen: &str,
    arm: &Arm,
    tuning: SearchTuning,
    hash_mb: usize,
    max_depth: u32,
    time_limit: Option<Duration>,
    safety_cap: Option<Duration>,
) -> RunOutcome {
    let mut board = Board::from_sfen(sfen).expect("preflight already validated this SFEN");
    let board_before = board.clone();
    let hash_before = board.hash();
    let acc_before = board.acc.clone();
    let legal_before = generate_legal_moves(&mut board.clone());

    let config = SearchConfig {
        max_depth,
        time_limit: time_limit.or(safety_cap),
        soft_limit: None,
        multi_pv: 1,
        use_ybw: arm.use_ybw,
        use_speculation: arm.use_speculation,
        spec_top_n: SearchConfig::default().spec_top_n,
        ybw_max_siblings: SearchConfig::default().ybw_max_siblings,
        use_pvs: arm.use_pvs,
    };

    let t0 = Instant::now();
    let (
        best_move,
        score,
        completed_depth,
        main_nodes,
        spec_nodes,
        hashfull,
        bestmove_changes,
        pv,
        ybw,
        spec_tasks_started,
        spec_tasks_completed,
        spec_tasks_cancelled,
    ) = if arm.use_speculation {
        let info =
            SpeculativeSearcher::new(Tt::new(hash_mb)).search_tuned(&mut board, config, tuning);
        let (started, completed, cancelled) = spec_info_fields(&info);
        (
            info.best_move,
            info.score,
            info.depth,
            info.nodes,
            info.spec_nodes,
            info.hashfull,
            info.bestmove_changes,
            info.pv,
            info.ybw,
            started,
            completed,
            cancelled,
        )
    } else {
        let info = Searcher::new(Tt::new(hash_mb)).search_tuned(&mut board, config, tuning);
        (
            info.best_move,
            info.score,
            info.depth,
            info.nodes,
            0,
            info.hashfull,
            info.bestmove_changes,
            info.pv,
            info.ybw,
            0,
            0,
            0,
        )
    };
    let elapsed = t0.elapsed();

    let hash_unchanged = board.hash() == hash_before;
    let accumulator_unchanged = board.acc == acc_before;
    let board_unchanged = hash_unchanged && accumulator_unchanged;

    let bestmove_legal = match best_move {
        Some(mv) => legal_before.contains(&mv),
        None => legal_before.is_empty(),
    };
    let pv_head_matches_bestmove = match best_move {
        Some(mv) => pv.first() == Some(&mv),
        None => pv.is_empty(),
    };
    let pv_legal = {
        let mut replay = board_before.clone();
        let mut ok = true;
        for &mv in &pv {
            let legal = generate_legal_moves(&mut replay);
            if !legal.contains(&mv) {
                ok = false;
                break;
            }
            replay.do_move(mv);
        }
        ok
    };

    RunOutcome {
        best_move,
        score,
        completed_depth,
        main_nodes,
        spec_nodes,
        elapsed,
        hashfull,
        bestmove_changes,
        pv,
        ybw,
        spec_tasks_started,
        spec_tasks_completed,
        spec_tasks_cancelled,
        board_unchanged,
        hash_unchanged,
        accumulator_unchanged,
        bestmove_legal,
        pv_legal,
        pv_head_matches_bestmove,
    }
}

// ============================================================
// JSONL record
// ============================================================

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

fn opt_u32_json(v: Option<u32>) -> String {
    match v {
        Some(v) => v.to_string(),
        None => "null".to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
struct RecordMeta<'a> {
    git_commit: &'a str,
    binary_fingerprint: &'a str,
    weights_path: &'a str,
    weights_hash: &'a str,
    corpus_version: &'a str,
    corpus_hash: &'a str,
    position_id: &'a str,
    sfen_hash: &'a str,
    arm: &'a str,
    profile: &'a str,
    ybw_early_cancel: bool,
    threads: usize,
    repetition: u64,
    shuffle_seed: u64,
    mode: &'a str,
    requested_depth: Option<u32>,
    time_limit_ms: Option<u64>,
}

fn write_record(
    out: &mut File,
    meta: &RecordMeta,
    outcome: &RunOutcome,
    time_overrun_ns: Option<i64>,
) {
    let bestmove_str = outcome
        .best_move
        .map(move_to_usi)
        .unwrap_or_else(|| "none".to_string());
    let pv_str = outcome
        .pv
        .iter()
        .copied()
        .map(move_to_usi)
        .collect::<Vec<_>>()
        .join(" ");
    let total_nodes = outcome.main_nodes + outcome.spec_nodes;
    let nps = if outcome.elapsed.as_secs_f64() > 0.0 {
        (total_nodes as f64 / outcome.elapsed.as_secs_f64()) as u64
    } else {
        0
    };

    let line = format!(
        "{{\"schema_version\":\"{sv}\",\"git_commit\":\"{gc}\",\"binary_fingerprint\":\"{bf}\",\
         \"weights_path\":\"{wp}\",\"weights_hash\":\"{wh}\",\"corpus_version\":\"{cv}\",\
         \"corpus_hash\":\"{ch}\",\
         \"position_id\":\"{pid}\",\"sfen_hash\":\"{sh}\",\"arm\":\"{arm}\",\"profile\":\"{prof}\",\
         \"ybw_early_cancel\":{yec_cond},\
         \"threads\":{threads},\"repetition\":{rep},\"shuffle_seed\":{seed},\"mode\":\"{mode}\",\
         \"requested_depth\":{rd},\"time_limit_ms\":{tl},\"completed_depth\":{cd},\
         \"score\":{score},\"bestmove\":\"{bm}\",\"pv\":\"{pv}\",\"elapsed_ns\":{elapsed_ns},\
         \"main_nodes\":{mn},\"spec_nodes\":{sn},\"total_nodes\":{tn},\"nps\":{nps},\
         \"hashfull\":{hf},\"bestmove_changes\":{bc},\"time_overrun_ns\":{tor},\
         \"ybw_splits\":{ys},\"ybw_probes_started\":{yps},\"ybw_probes_completed\":{ypc},\
         \"ybw_probes_cancelled\":{ypx},\"ybw_direct_cutoffs\":{ydc},\
         \"ybw_full_researches\":{yfr},\"ybw_cancelled_nodes\":{ycn},\
         \"ybw_cancel_checks_hit\":{ycch},\"ybw_tasks_skipped_before_start\":{ytsb},\
         \"ybw_recursive_aborts\":{yra},\
         \"spec_tasks_started\":{sts},\"spec_tasks_completed\":{stc},\
         \"spec_tasks_cancelled\":{stx},\"board_unchanged\":{bu},\"hash_unchanged\":{hu},\
         \"accumulator_unchanged\":{au},\"pv_legal\":{pl},\"bestmove_legal\":{bl},\
         \"pv_head_matches_bestmove\":{phm}}}",
        sv = SCHEMA_VERSION,
        gc = json_escape(meta.git_commit),
        bf = json_escape(meta.binary_fingerprint),
        wp = json_escape(meta.weights_path),
        wh = json_escape(meta.weights_hash),
        cv = json_escape(meta.corpus_version),
        ch = json_escape(meta.corpus_hash),
        pid = json_escape(meta.position_id),
        sh = json_escape(meta.sfen_hash),
        arm = json_escape(meta.arm),
        prof = json_escape(meta.profile),
        yec_cond = meta.ybw_early_cancel,
        threads = meta.threads,
        rep = meta.repetition,
        seed = meta.shuffle_seed,
        mode = json_escape(meta.mode),
        rd = opt_u32_json(meta.requested_depth),
        tl = meta
            .time_limit_ms
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string()),
        cd = outcome.completed_depth,
        score = outcome.score,
        bm = json_escape(&bestmove_str),
        pv = json_escape(&pv_str),
        elapsed_ns = outcome.elapsed.as_nanos(),
        mn = outcome.main_nodes,
        sn = outcome.spec_nodes,
        tn = total_nodes,
        nps = nps,
        hf = outcome.hashfull,
        bc = outcome.bestmove_changes,
        tor = time_overrun_ns
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string()),
        ys = outcome.ybw.splits,
        yps = outcome.ybw.probes_started,
        ypc = outcome.ybw.probes_completed,
        ypx = outcome.ybw.probes_cancelled,
        ydc = outcome.ybw.direct_cutoffs,
        yfr = outcome.ybw.full_researches,
        ycn = outcome.ybw.cancelled_nodes,
        ycch = outcome.ybw.cancel_checks_hit,
        ytsb = outcome.ybw.tasks_skipped_before_start,
        yra = outcome.ybw.recursive_aborts,
        sts = outcome.spec_tasks_started,
        stc = outcome.spec_tasks_completed,
        stx = outcome.spec_tasks_cancelled,
        bu = outcome.board_unchanged,
        hu = outcome.hash_unchanged,
        au = outcome.accumulator_unchanged,
        pl = outcome.pv_legal,
        bl = outcome.bestmove_legal,
        phm = outcome.pv_head_matches_bestmove,
    );
    writeln!(out, "{line}").expect("write JSONL record");
}

// ============================================================
// Environment fingerprints
// ============================================================

fn git_commit() -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn binary_fingerprint() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| fs::read(p).ok())
        .map(|bytes| hex_hash(&bytes))
        .unwrap_or_else(|| "unknown".to_string())
}

fn load_weights_and_fingerprint(weights_path: Option<&str>) -> (String, String) {
    match weights_path {
        Some(path) => {
            let bytes = fs::read(path).unwrap_or_else(|e| {
                eprintln!("fatal: failed to read weights file {path}: {e}");
                std::process::exit(1);
            });
            let hash = hex_hash(&bytes);
            if let Err(e) = nnue::load_weights(Path::new(path)) {
                eprintln!("fatal: failed to load weights file {path}: {e}");
                std::process::exit(1);
            }
            (path.to_string(), hash)
        }
        None => ("default_lcg".to_string(), "default_lcg".to_string()),
    }
}

// ============================================================
// Stats
// ============================================================

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

struct Stats {
    median: f64,
    mean: f64,
    stddev: f64,
    p25: f64,
    p75: f64,
    p5: f64,
    p95: f64,
    cv: f64,
}

fn compute_stats(values: &[f64]) -> Stats {
    if values.is_empty() {
        return Stats {
            median: 0.0,
            mean: 0.0,
            stddev: 0.0,
            p25: 0.0,
            p75: 0.0,
            p5: 0.0,
            p95: 0.0,
            cv: 0.0,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    let stddev = variance.sqrt();
    Stats {
        median: percentile(&sorted, 0.5),
        mean,
        stddev,
        p25: percentile(&sorted, 0.25),
        p75: percentile(&sorted, 0.75),
        p5: percentile(&sorted, 0.05),
        p95: percentile(&sorted, 0.95),
        cv: if mean != 0.0 {
            stddev / mean.abs()
        } else {
            0.0
        },
    }
}

fn median_of(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    percentile(&sorted, 0.5)
}

fn mean_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// Percentile bootstrap 95% CI for `statistic` over `values`, seeded so
/// results are reproducible from the run's own seed rather than any real
/// randomness.
fn bootstrap_ci95(
    values: &[f64],
    statistic: fn(&[f64]) -> f64,
    resamples: usize,
    seed: u64,
) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let mut s = seed | 1; // avoid a zero state, which xorshift64 can't escape
    let mut stats: Vec<f64> = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let resample: Vec<f64> = (0..values.len())
            .map(|_| values[(xorshift64(&mut s) as usize) % values.len()])
            .collect();
        stats.push(statistic(&resample));
    }
    stats.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (percentile(&stats, 0.025), percentile(&stats, 0.975))
}

fn mode_agreement_rate<T: Eq + std::hash::Hash + Clone>(values: &[T]) -> f64 {
    if values.is_empty() {
        return 1.0;
    }
    let mut counts: HashMap<T, usize> = HashMap::new();
    for v in values {
        *counts.entry(v.clone()).or_insert(0) += 1;
    }
    let max_count = counts.values().copied().max().unwrap_or(0);
    max_count as f64 / values.len() as f64
}

// ============================================================
// CLI
// ============================================================

struct Cli {
    phase: String,
    corpus: PathBuf,
    out: Option<PathBuf>,
    threads: Vec<usize>,
    weights: Option<String>,
    hash_mb: usize,
    seed: u64,
    warmup: u64,
    depths: Vec<u32>,
    reps_t1: u64,
    reps_tn: u64,
    time_ms: u64,
    position: Option<String>,
    smoke_positions: usize,
    baseline_check: bool,
    arms: Vec<String>,
    profiles: Vec<String>,
}

fn print_help() {
    println!(
        "search_ablation <phase> [options]\n\n\
         phases: smoke | audit | fixed-depth | fixed-time\n\n\
         options:\n\
         \x20\x20--corpus PATH         (default: {DEFAULT_CORPUS})\n\
         \x20\x20--out PATH            (default: results/<ts>_search_ablation_<phase>.jsonl)\n\
         \x20\x20--threads 1,2,4       (default: 1,2,4)\n\
         \x20\x20--weights PATH        (default: built-in default_lcg)\n\
         \x20\x20--hash-mb N           (default: 16)\n\
         \x20\x20--seed N              (default: 20260722)\n\
         \x20\x20--warmup N            (default: 1)\n\
         \x20\x20--depths 3,4,5,6      (fixed-depth / smoke; default varies by phase)\n\
         \x20\x20--reps-t1 N           (default: 3)\n\
         \x20\x20--reps-tn N           (default: 10, Threads>1)\n\
         \x20\x20--time-ms N           (fixed-time; default: 3000)\n\
         \x20\x20--position ID         (audit; default: first midgame position)\n\
         \x20\x20--smoke-positions N   (smoke; default: 5)\n\
         \x20\x20--baseline-check      (cancel-ablation; compares on-vs-on instead of \n\
         \x20\x20                       on-vs-off, to measure disagreement from the known,\n\
         \x20\x20                       pre-existing killer/history/countermove race alone,\n\
         \x20\x20                       independent of ybw_early_cancel)\n\
         \x20\x20--arms A,B,C,D,E      (fixed-depth/fixed-time; default: all 5. Filters which\n\
         \x20\x20                       arms are dispatched -- does not change how any single\n\
         \x20\x20                       measurement is taken, only which combinations run.)\n\
         \x20\x20--profiles production,controlled  (fixed-depth; default: both. Same caveat\n\
         \x20\x20                       as --arms -- a pure dispatch filter.)\n"
    );
}

fn parse_cli() -> Cli {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        print_help();
        std::process::exit(if args.len() < 2 { 1 } else { 0 });
    }
    let phase = args[1].clone();
    let mut corpus = PathBuf::from(DEFAULT_CORPUS);
    let mut out = None;
    let mut threads = vec![1, 2, 4];
    let mut weights = None;
    let mut hash_mb = 16usize;
    let mut seed = 20260722u64;
    let mut warmup = 1u64;
    let mut depths = Vec::new();
    let mut reps_t1 = 3u64;
    let mut reps_tn = 10u64;
    let mut time_ms = 3000u64;
    let mut position = None;
    let mut smoke_positions = 5usize;
    let mut baseline_check = false;
    let mut arms: Vec<String> = ARMS.iter().map(|a| a.id.to_string()).collect();
    let mut profiles: Vec<String> = vec!["production".to_string(), "controlled".to_string()];

    let mut i = 2;
    while i < args.len() {
        let flag = args[i].as_str();
        let mut next = || {
            i += 1;
            args.get(i)
                .unwrap_or_else(|| {
                    eprintln!("fatal: {flag} requires a value");
                    std::process::exit(1);
                })
                .clone()
        };
        match flag {
            "--corpus" => corpus = PathBuf::from(next()),
            "--out" => out = Some(PathBuf::from(next())),
            "--threads" => threads = next().split(',').map(|s| s.parse().unwrap()).collect(),
            "--weights" => weights = Some(next()),
            "--hash-mb" => hash_mb = next().parse().unwrap(),
            "--seed" => seed = next().parse().unwrap(),
            "--warmup" => warmup = next().parse().unwrap(),
            "--depths" => depths = next().split(',').map(|s| s.parse().unwrap()).collect(),
            "--reps-t1" => reps_t1 = next().parse().unwrap(),
            "--reps-tn" => reps_tn = next().parse().unwrap(),
            "--time-ms" => time_ms = next().parse().unwrap(),
            "--position" => position = Some(next()),
            "--smoke-positions" => smoke_positions = next().parse().unwrap(),
            "--baseline-check" => baseline_check = true,
            "--arms" => arms = next().split(',').map(|s| s.to_string()).collect(),
            "--profiles" => profiles = next().split(',').map(|s| s.to_string()).collect(),
            _ => {
                eprintln!("fatal: unknown option {flag}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    Cli {
        phase,
        corpus,
        out,
        threads,
        weights,
        hash_mb,
        seed,
        warmup,
        depths,
        reps_t1,
        reps_tn,
        time_ms,
        position,
        smoke_positions,
        baseline_check,
        arms,
        profiles,
    }
}

fn default_out_path(phase: &str) -> PathBuf {
    let ts = fs::metadata(".")
        .and_then(|_| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| std::io::Error::other("time"))
        })
        .map(|d| d.as_secs())
        .unwrap_or(0);
    fs::create_dir_all("results").ok();
    PathBuf::from(format!("results/{ts}_search_ablation_{phase}.jsonl"))
}

fn open_out(path: &Path) -> File {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|e| {
            eprintln!("fatal: failed to open output file {}: {e}", path.display());
            std::process::exit(1);
        })
}

// ============================================================
// main
// ============================================================

fn main() {
    let cli = parse_cli();
    println!("arms:");
    for arm in &ARMS {
        println!(
            "  {} ({}): pvs={} ybw={} speculation={}",
            arm.id, arm.label, arm.use_pvs, arm.use_ybw, arm.use_speculation
        );
    }
    let (raw_positions, corpus_hash) = load_corpus(&cli.corpus).unwrap_or_else(|e| {
        eprintln!("fatal: failed to read corpus {}: {e}", cli.corpus.display());
        std::process::exit(1);
    });
    let (positions, rejected) = preflight(raw_positions);
    if !rejected.is_empty() {
        eprintln!("preflight rejected {} position(s):", rejected.len());
        for r in &rejected {
            eprintln!("  {r}");
        }
    }
    println!(
        "corpus: {} positions accepted, {} rejected (hash {corpus_hash})",
        positions.len(),
        rejected.len()
    );

    let (weights_path, weights_hash) = load_weights_and_fingerprint(cli.weights.as_deref());
    let git = git_commit();
    let binfp = binary_fingerprint();
    println!("git_commit={git} binary_fingerprint={binfp} weights={weights_path} ({weights_hash})");

    let out_path = cli
        .out
        .clone()
        .unwrap_or_else(|| default_out_path(&cli.phase));
    println!("writing JSONL to {}", out_path.display());
    let mut out = open_out(&out_path);

    let env = Env {
        git,
        binfp,
        weights_path,
        weights_hash,
        corpus_hash,
    };

    match cli.phase.as_str() {
        "smoke" => phase_smoke(&cli, &positions, &env, &mut out),
        "audit" => phase_audit(&cli, &positions, &env, &mut out),
        "fixed-depth" => phase_fixed_depth(&cli, &positions, &env, &mut out),
        "fixed-time" => phase_fixed_time(&cli, &positions, &env, &mut out),
        "cancel-ablation" => phase_cancel_ablation(&cli, &positions, &env, &mut out),
        other => {
            eprintln!(
                "fatal: unknown phase {other:?} (expected smoke|audit|fixed-depth|fixed-time|cancel-ablation)"
            );
            std::process::exit(1);
        }
    }
}

struct Env {
    git: String,
    binfp: String,
    weights_path: String,
    weights_hash: String,
    corpus_hash: String,
}

fn install_pool<F: FnOnce() + Send>(threads: usize, f: F) {
    if threads == 0 {
        eprintln!("fatal: --threads entries must be >= 1");
        std::process::exit(1);
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("build local rayon pool")
        .install(f);
}

// ============================================================
// Phase 1: correctness smoke
// ============================================================

fn phase_smoke(cli: &Cli, positions: &[Position], env: &Env, out: &mut File) {
    let depths = if cli.depths.is_empty() {
        vec![3, 4]
    } else {
        cli.depths.clone()
    };
    let reps = 2u64;
    let sample = pick_diverse_sample(positions, cli.smoke_positions);
    println!(
        "=== Phase 1: correctness smoke -- {} positions, threads {:?}, depths {:?}, {reps} reps ===",
        sample.len(),
        cli.threads,
        depths
    );

    let mut crash_free = true;
    let mut illegal_found = 0u64;
    let mut pv_illegal_found = 0u64;
    let mut board_mutated_found = 0u64;
    let mut ybw_off_but_active = 0u64;
    let mut spec_off_but_active = 0u64;

    for &threads in &cli.threads {
        install_pool(threads, || {
            for pos in &sample {
                for &depth in &depths {
                    for rep in 0..reps {
                        for &arm_idx in
                            &shuffled_arm_order(position_shuffle_seed(cli.seed, &pos.id, rep)).0
                        {
                            let arm = &ARMS[arm_idx];
                            let outcome = run_one(
                                &pos.sfen,
                                arm,
                                Profile::Production.tuning(true),
                                cli.hash_mb,
                                depth,
                                None,
                                Some(Duration::from_secs(30)),
                            );
                            if !outcome.bestmove_legal {
                                illegal_found += 1;
                                eprintln!(
                                    "SMOKE FAIL: {} arm={} pos={} depth={depth}: illegal bestmove",
                                    pos.id, arm.id, pos.id
                                );
                            }
                            if !outcome.pv_legal {
                                pv_illegal_found += 1;
                                eprintln!(
                                    "SMOKE FAIL: {} arm={} pos={} depth={depth}: illegal PV",
                                    pos.id, arm.id, pos.id
                                );
                            }
                            if !outcome.board_unchanged {
                                board_mutated_found += 1;
                                eprintln!(
                                    "SMOKE FAIL: {} arm={} pos={} depth={depth}: board/hash/acc mutated",
                                    pos.id, arm.id, pos.id
                                );
                            }
                            if !arm.use_ybw && outcome.ybw.splits > 0 {
                                ybw_off_but_active += 1;
                                eprintln!(
                                    "SMOKE FAIL: arm={} has use_ybw=false but ybw.splits={}",
                                    arm.id, outcome.ybw.splits
                                );
                            }
                            if !arm.use_speculation && outcome.spec_tasks_started > 0 {
                                spec_off_but_active += 1;
                                eprintln!(
                                    "SMOKE FAIL: arm={} has use_speculation=false but spec_tasks_started={}",
                                    arm.id, outcome.spec_tasks_started
                                );
                            }
                            let meta = RecordMeta {
                                git_commit: &env.git,
                                binary_fingerprint: &env.binfp,
                                weights_path: &env.weights_path,
                                weights_hash: &env.weights_hash,
                                corpus_version: CORPUS_VERSION,
                                corpus_hash: &env.corpus_hash,
                                position_id: &pos.id,
                                sfen_hash: &hex_hash(pos.sfen.as_bytes()),
                                arm: arm.id,
                                profile: Profile::Production.name(),
                                ybw_early_cancel: true,
                                threads,
                                repetition: rep,
                                shuffle_seed: position_shuffle_seed(cli.seed, &pos.id, rep),
                                mode: "smoke",
                                requested_depth: Some(depth),
                                time_limit_ms: None,
                            };
                            write_record(out, &meta, &outcome, None);
                        }
                    }
                }
            }
        });
    }
    out.flush().ok();

    println!("--- smoke summary ---");
    println!("crash_free: {crash_free}");
    println!("illegal_bestmove_count: {illegal_found}");
    println!("illegal_pv_count: {pv_illegal_found}");
    println!("board_mutated_count: {board_mutated_found}");
    println!("ybw_off_but_active_count: {ybw_off_but_active}");
    println!("spec_off_but_active_count: {spec_off_but_active}");
    crash_free &= illegal_found == 0
        && pv_illegal_found == 0
        && board_mutated_found == 0
        && ybw_off_but_active == 0
        && spec_off_but_active == 0;
    println!(
        "SMOKE {}",
        if crash_free {
            "PASSED"
        } else {
            "FAILED -- see SMOKE FAIL lines above"
        }
    );
}

fn pick_diverse_sample(positions: &[Position], n: usize) -> Vec<Position> {
    let mut seen_categories = std::collections::HashSet::new();
    let mut sample = Vec::new();
    for p in positions {
        if sample.len() >= n {
            break;
        }
        if seen_categories.insert(p.category.clone()) {
            sample.push(p.clone());
        }
    }
    // Top up with more positions (any category) if fewer categories than n.
    for p in positions {
        if sample.len() >= n {
            break;
        }
        if !sample.iter().any(|s| s.id == p.id) {
            sample.push(p.clone());
        }
    }
    sample
}

// ============================================================
// Phase 2: nondeterminism audit
// ============================================================

fn phase_audit(cli: &Cli, positions: &[Position], env: &Env, out: &mut File) {
    let pos = cli
        .position
        .as_ref()
        .and_then(|id| positions.iter().find(|p| &p.id == id).cloned())
        .or_else(|| positions.iter().find(|p| p.category == "midgame").cloned())
        .unwrap_or_else(|| {
            eprintln!("fatal: no position available for audit");
            std::process::exit(1);
        });
    let depth = cli.depths.first().copied().unwrap_or(4);
    println!(
        "=== Phase 2: nondeterminism audit -- position={} depth={depth} ===",
        pos.id
    );

    for &(threads, reps) in &[(1usize, 5u64), (2, 20), (4, 20)] {
        if !cli.threads.contains(&threads) {
            continue;
        }
        for profile in [Profile::Production, Profile::Controlled] {
            let mut scores = Vec::new();
            let mut bestmoves = Vec::new();
            let mut nodes = Vec::new();
            let mut elapsed_ns = Vec::new();
            let mut depths_seen = Vec::new();
            let mut cutoffs_total = 0u64;
            let mut cancelled_total = 0u64;

            install_pool(threads, || {
                for rep in 0..reps {
                    // Arm C (PVS+YBW) is the one whose cancellation activity
                    // this audit cares about isolating from production
                    // heuristics.
                    let arm = &ARMS[2];
                    let outcome = run_one(
                        &pos.sfen,
                        arm,
                        profile.tuning(true),
                        cli.hash_mb,
                        depth,
                        None,
                        Some(Duration::from_secs(30)),
                    );
                    scores.push(outcome.score);
                    bestmoves.push(outcome.best_move.map(move_to_usi));
                    nodes.push((outcome.main_nodes + outcome.spec_nodes) as f64);
                    elapsed_ns.push(outcome.elapsed.as_nanos() as f64);
                    depths_seen.push(outcome.completed_depth);
                    cutoffs_total += outcome.ybw.direct_cutoffs;
                    cancelled_total += outcome.ybw.probes_cancelled;

                    let meta = RecordMeta {
                        git_commit: &env.git,
                        binary_fingerprint: &env.binfp,
                        weights_path: &env.weights_path,
                        weights_hash: &env.weights_hash,
                        corpus_version: CORPUS_VERSION,
                        corpus_hash: &env.corpus_hash,
                        position_id: &pos.id,
                        sfen_hash: &hex_hash(pos.sfen.as_bytes()),
                        arm: arm.id,
                        profile: profile.name(),
                        ybw_early_cancel: true,
                        threads,
                        repetition: rep,
                        shuffle_seed: cli.seed,
                        mode: "audit",
                        requested_depth: Some(depth),
                        time_limit_ms: None,
                    };
                    write_record(out, &meta, &outcome, None);
                }
            });

            let score_agree = mode_agreement_rate(&scores);
            let bm_agree = mode_agreement_rate(&bestmoves);
            let node_stats = compute_stats(&nodes);
            let elapsed_stats = compute_stats(&elapsed_ns);
            let mut depth_counts: HashMap<u32, u64> = HashMap::new();
            for d in &depths_seen {
                *depth_counts.entry(*d).or_insert(0) += 1;
            }
            println!(
                "threads={threads} profile={} reps={reps}: score_agreement={:.2} bestmove_agreement={:.2} \
                 depth_dist={:?} ybw_direct_cutoffs={cutoffs_total} ybw_probes_cancelled={cancelled_total}",
                profile.name(),
                score_agree,
                bm_agree,
                depth_counts,
            );
            println!(
                "  total_nodes: median={:.0} mean={:.0} stddev={:.0} p5={:.0} p25={:.0} p75={:.0} \
                 p95={:.0} cv={:.4}",
                node_stats.median,
                node_stats.mean,
                node_stats.stddev,
                node_stats.p5,
                node_stats.p25,
                node_stats.p75,
                node_stats.p95,
                node_stats.cv,
            );
            println!(
                "  elapsed_ns: median={:.0} mean={:.0} stddev={:.0} p5={:.0} p25={:.0} p75={:.0} \
                 p95={:.0} cv={:.4}",
                elapsed_stats.median,
                elapsed_stats.mean,
                elapsed_stats.stddev,
                elapsed_stats.p5,
                elapsed_stats.p25,
                elapsed_stats.p75,
                elapsed_stats.p95,
                elapsed_stats.cv,
            );
        }
    }
    out.flush().ok();
    println!(
        "--- engineering thresholds (informational, not pass/fail gates): total_nodes CV <= ~0.05, \
         bestmove agreement >= ~0.90, arm ranking not drastically reversed between production/controlled ---"
    );
}

// ============================================================
// Phase 3: full fixed-depth
// ============================================================

fn phase_fixed_depth(cli: &Cli, positions: &[Position], env: &Env, out: &mut File) {
    let depths = if cli.depths.is_empty() {
        vec![4, 5]
    } else {
        cli.depths.clone()
    };
    println!(
        "=== Phase 3: full fixed-depth -- {} positions, depths {:?}, threads {:?} ===",
        positions.len(),
        depths,
        cli.threads
    );
    for &threads in &cli.threads {
        let reps = if threads == 1 {
            cli.reps_t1
        } else {
            cli.reps_tn
        };
        install_pool(threads, || {
            for pos in positions {
                for &depth in &depths {
                    for profile in [Profile::Production, Profile::Controlled] {
                        if !cli.profiles.iter().any(|p| p == profile.name()) {
                            continue;
                        }
                        for rep in 0..(cli.warmup + reps) {
                            let is_warmup = rep < cli.warmup;
                            let arm_order =
                                shuffled_arm_order(position_shuffle_seed(cli.seed, &pos.id, rep)).0;
                            for &arm_idx in &arm_order {
                                let arm = &ARMS[arm_idx];
                                if !cli.arms.iter().any(|a| a == arm.id) {
                                    continue;
                                }
                                let outcome = run_one(
                                    &pos.sfen,
                                    arm,
                                    profile.tuning(true),
                                    cli.hash_mb,
                                    depth,
                                    None,
                                    Some(Duration::from_secs(30)),
                                );
                                if is_warmup {
                                    continue; // warm-up: discarded, not written
                                }
                                let meta = RecordMeta {
                                    git_commit: &env.git,
                                    binary_fingerprint: &env.binfp,
                                    weights_path: &env.weights_path,
                                    weights_hash: &env.weights_hash,
                                    corpus_version: CORPUS_VERSION,
                                    corpus_hash: &env.corpus_hash,
                                    position_id: &pos.id,
                                    sfen_hash: &hex_hash(pos.sfen.as_bytes()),
                                    arm: arm.id,
                                    profile: profile.name(),
                                    ybw_early_cancel: true,
                                    threads,
                                    repetition: rep - cli.warmup,
                                    shuffle_seed: position_shuffle_seed(cli.seed, &pos.id, rep),
                                    mode: "fixed-depth",
                                    requested_depth: Some(depth),
                                    time_limit_ms: None,
                                };
                                write_record(out, &meta, &outcome, None);
                            }
                        }
                    }
                }
                out.flush().ok();
            }
        });
        println!("threads={threads} (reps={reps}) done");
    }
}

// ============================================================
// Phase 4: full fixed-time (production only)
// ============================================================

fn phase_fixed_time(cli: &Cli, positions: &[Position], env: &Env, out: &mut File) {
    let time_limit = Duration::from_millis(cli.time_ms);
    println!(
        "=== Phase 4: full fixed-time -- {} positions, time_ms={}, threads {:?} (production only) ===",
        positions.len(),
        cli.time_ms,
        cli.threads
    );
    for &threads in &cli.threads {
        let reps = if threads == 1 {
            cli.reps_t1
        } else {
            cli.reps_tn
        };
        install_pool(threads, || {
            for pos in positions {
                for rep in 0..(cli.warmup + reps) {
                    let is_warmup = rep < cli.warmup;
                    let arm_order =
                        shuffled_arm_order(position_shuffle_seed(cli.seed, &pos.id, rep)).0;
                    for &arm_idx in &arm_order {
                        let arm = &ARMS[arm_idx];
                        if !cli.arms.iter().any(|a| a == arm.id) {
                            continue;
                        }
                        let outcome = run_one(
                            &pos.sfen,
                            arm,
                            Profile::Production.tuning(true),
                            cli.hash_mb,
                            64, // time-bounded, not depth-bounded
                            Some(time_limit),
                            None,
                        );
                        if is_warmup {
                            continue;
                        }
                        let overrun_ns =
                            outcome.elapsed.as_nanos() as i64 - time_limit.as_nanos() as i64;
                        let meta = RecordMeta {
                            git_commit: &env.git,
                            binary_fingerprint: &env.binfp,
                            weights_path: &env.weights_path,
                            weights_hash: &env.weights_hash,
                            corpus_version: CORPUS_VERSION,
                            corpus_hash: &env.corpus_hash,
                            position_id: &pos.id,
                            sfen_hash: &hex_hash(pos.sfen.as_bytes()),
                            arm: arm.id,
                            profile: Profile::Production.name(),
                            ybw_early_cancel: true,
                            threads,
                            repetition: rep - cli.warmup,
                            shuffle_seed: position_shuffle_seed(cli.seed, &pos.id, rep),
                            mode: "fixed-time",
                            requested_depth: None,
                            time_limit_ms: Some(cli.time_ms),
                        };
                        write_record(out, &meta, &outcome, Some(overrun_ns));
                    }
                }
                out.flush().ok();
            }
        });
        println!("threads={threads} (reps={reps}) done");
    }
}

// ============================================================
// Phase: YBW early-cancellation ablation (arms C/D only)
// ============================================================

struct PairedDeltaReport {
    n: usize,
    median_delta: f64,
    mean_delta: f64,
    ci95: (f64, f64),
    improved: usize,
    worsened: usize,
    tied: usize,
}

/// `deltas[i]` should be a *relative* change, e.g. `(on - off) / off`, so a
/// negative value means "on" used less of that resource than "off".
fn paired_delta_report(deltas: &[f64], seed: u64) -> PairedDeltaReport {
    let improved = deltas.iter().filter(|&&d| d < -1e-9).count();
    let worsened = deltas.iter().filter(|&&d| d > 1e-9).count();
    let tied = deltas.len().saturating_sub(improved + worsened);
    PairedDeltaReport {
        n: deltas.len(),
        median_delta: median_of(deltas),
        mean_delta: mean_of(deltas),
        ci95: bootstrap_ci95(deltas, median_of, 2000, seed),
        improved,
        worsened,
        tied,
    }
}

impl std::fmt::Display for PairedDeltaReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "n={} median_delta={:.4} mean_delta={:.4} ci95=[{:.4},{:.4}] improved={} worsened={} tied={}",
            self.n,
            self.median_delta,
            self.mean_delta,
            self.ci95.0,
            self.ci95.1,
            self.improved,
            self.worsened,
            self.tied,
        )
    }
}

/// Isolates Commit 3's actual node/time savings via a paired on/off
/// comparison of `ybw_early_cancel`, same position/depth/threads/arm/fresh
/// state, arms C and D only, production profile, Threads>=2 (cancellation
/// has no concurrent sibling to save work from at Threads=1). Per the
/// user's own framing: `ybw_direct_cutoffs` only counts how often the
/// cutoff *condition* was met -- it is NOT the reduction effect itself,
/// which is what the total_nodes/elapsed paired deltas below measure
/// directly.
fn phase_cancel_ablation(cli: &Cli, positions: &[Position], env: &Env, out: &mut File) {
    let depths = if cli.depths.is_empty() {
        vec![5, 6]
    } else {
        cli.depths.clone()
    };
    let n_positions = if cli.smoke_positions == 5 {
        8
    } else {
        cli.smoke_positions
    };
    let sample = pick_diverse_sample(positions, n_positions);
    let reps = cli.reps_tn;
    let threads_list: Vec<usize> = cli.threads.iter().copied().filter(|&t| t >= 2).collect();
    if threads_list.is_empty() {
        eprintln!(
            "fatal: cancel-ablation needs at least one --threads value >= 2 (cancellation has \
             no concurrent sibling to save work from at Threads=1)"
        );
        std::process::exit(1);
    }
    println!(
        "=== Phase: ybw_early_cancel ablation -- {} positions, depths {:?}, threads {:?}, \
         arms C/D, production only, {reps} reps ===",
        sample.len(),
        depths,
        threads_list
    );

    for &threads in &threads_list {
        for &arm_idx in &[2usize, 3usize] {
            let arm = &ARMS[arm_idx];
            let mut node_deltas = Vec::new();
            let mut elapsed_deltas = Vec::new();
            let mut score_matches = 0usize;
            let mut bestmove_matches = 0usize;
            let mut total = 0usize;
            let mut total_direct_cutoffs = 0u64;
            let mut total_cancel_checks_hit = 0u64;
            let mut total_tasks_skipped = 0u64;
            let mut total_recursive_aborts = 0u64;
            let mut total_cancelled_nodes = 0u64;
            let mut pv_legal_all = true;
            let mut state_unchanged_all = true;

            install_pool(threads, || {
                for pos in &sample {
                    for &depth in &depths {
                        for rep in 0..reps {
                            let pair_seed = position_shuffle_seed(cli.seed, &pos.id, rep)
                                ^ (arm_idx as u64) << 16
                                ^ (depth as u64) << 8
                                ^ threads as u64;
                            let mut order_rng = pair_seed;
                            let on_first = xorshift64(&mut order_rng).is_multiple_of(2);

                            // `--baseline-check`: both calls use `true` (on), measuring
                            // disagreement from the known, pre-existing killer/history/
                            // countermove race alone -- a control for the on-vs-off
                            // comparison below, not a real ablation of anything.
                            let run = |early_cancel: bool| {
                                let effective = if cli.baseline_check {
                                    true
                                } else {
                                    early_cancel
                                };
                                run_one(
                                    &pos.sfen,
                                    arm,
                                    Profile::Production.tuning(effective),
                                    cli.hash_mb,
                                    depth,
                                    None,
                                    Some(Duration::from_secs(30)),
                                )
                            };
                            let (on_outcome, off_outcome) = if on_first {
                                let on = run(true);
                                let off = run(false);
                                (on, off)
                            } else {
                                let off = run(false);
                                let on = run(true);
                                (on, off)
                            };

                            for (early_cancel, outcome) in
                                [(true, &on_outcome), (false, &off_outcome)]
                            {
                                let meta = RecordMeta {
                                    git_commit: &env.git,
                                    binary_fingerprint: &env.binfp,
                                    weights_path: &env.weights_path,
                                    weights_hash: &env.weights_hash,
                                    corpus_version: CORPUS_VERSION,
                                    corpus_hash: &env.corpus_hash,
                                    position_id: &pos.id,
                                    sfen_hash: &hex_hash(pos.sfen.as_bytes()),
                                    arm: arm.id,
                                    profile: Profile::Production.name(),
                                    ybw_early_cancel: early_cancel,
                                    threads,
                                    repetition: rep,
                                    shuffle_seed: pair_seed,
                                    mode: "cancel-ablation",
                                    requested_depth: Some(depth),
                                    time_limit_ms: None,
                                };
                                write_record(out, &meta, outcome, None);
                            }

                            let on_nodes = (on_outcome.main_nodes + on_outcome.spec_nodes) as f64;
                            let off_nodes =
                                (off_outcome.main_nodes + off_outcome.spec_nodes) as f64;
                            if off_nodes > 0.0 {
                                node_deltas.push((on_nodes - off_nodes) / off_nodes);
                            }
                            let on_elapsed = on_outcome.elapsed.as_nanos() as f64;
                            let off_elapsed = off_outcome.elapsed.as_nanos() as f64;
                            if off_elapsed > 0.0 {
                                elapsed_deltas.push((on_elapsed - off_elapsed) / off_elapsed);
                            }
                            total += 1;
                            if on_outcome.score == off_outcome.score {
                                score_matches += 1;
                            }
                            if on_outcome.best_move == off_outcome.best_move {
                                bestmove_matches += 1;
                            }
                            total_direct_cutoffs += on_outcome.ybw.direct_cutoffs;
                            total_cancel_checks_hit += on_outcome.ybw.cancel_checks_hit;
                            total_tasks_skipped += on_outcome.ybw.tasks_skipped_before_start;
                            total_recursive_aborts += on_outcome.ybw.recursive_aborts;
                            total_cancelled_nodes += on_outcome.ybw.cancelled_nodes;
                            pv_legal_all &= on_outcome.pv_legal && off_outcome.pv_legal;
                            state_unchanged_all &=
                                on_outcome.board_unchanged && off_outcome.board_unchanged;
                        }
                    }
                }
            });
            out.flush().ok();

            let node_report = paired_delta_report(&node_deltas, cli.seed ^ (arm_idx as u64));
            let elapsed_report =
                paired_delta_report(&elapsed_deltas, cli.seed ^ (arm_idx as u64) ^ 0xABCD);
            let score_agree = if total > 0 {
                score_matches as f64 / total as f64
            } else {
                1.0
            };
            let bm_agree = if total > 0 {
                bestmove_matches as f64 / total as f64
            } else {
                1.0
            };

            println!(
                "--- arm={} threads={threads}: direct_cutoffs={total_direct_cutoffs} \
                 cancel_checks_hit={total_cancel_checks_hit} tasks_skipped_before_start={total_tasks_skipped} \
                 recursive_aborts={total_recursive_aborts} cancelled_nodes={total_cancelled_nodes}",
                arm.id
            );
            println!("  total_nodes delta (on vs off, relative): {node_report}");
            println!("  elapsed delta (on vs off, relative):     {elapsed_report}");
            println!(
                "  score_agreement={score_agree:.2} bestmove_agreement={bm_agree:.2} \
                 pv_legal_all={pv_legal_all} state_unchanged_all={state_unchanged_all}"
            );

            let verdict = if !pv_legal_all || !state_unchanged_all {
                "PV/STATE INVARIANT VIOLATED -- stop and fix correctness before trusting any performance number"
            } else if score_agree < 0.95 {
                "LOW score agreement -- before concluding this is a cancellation bug, re-run with \
                 --baseline-check (on-vs-on) at the same scope: arms using speculation are known \
                 to have their own pre-existing, documented non-determinism (shared killer/history/\
                 countermove tables, speculative TT writes) independent of this feature -- compare \
                 against that baseline rate, don't assume this number alone means a new defect"
            } else if total_cancel_checks_hit == 0 {
                "cutoffs proven but cancellation never observed at this depth/branching -- correct, but effectiveness unconfirmed here (try deeper positions)"
            } else if node_report.median_delta < -0.01 || elapsed_report.median_delta < -0.01 {
                "cancellation observed AND nodes/elapsed improved -- early cancellation is effective here"
            } else {
                "cancellation observed but no measurable nodes/elapsed improvement -- token-check overhead or search-order change may be offsetting the savings at this granularity"
            };
            println!("  verdict: {verdict}");
        }
    }
}
