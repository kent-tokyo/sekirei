//! One-off calibration harness, not wired into any CLI: does veridict's
//! `SprtVariant::Trinomial` + `paired_by_id=true` correctly absorb intra-pair
//! (same-position, opposite-color) correlation, without reproducing the
//! CI-inflation bug that hit the `metrics::elo`/Wilson path (see
//! `tasks/lessons.md`, 2026-07-09)? See the 2026-07-12 gate-B-prep entry for
//! the request this answers.
//!
//! Run manually: `cargo run --release -p sekirei-match-runner --example verify_pairing`
//!
//! Part 1: replays the real v012-vs-v010 400-game set (already known to have
//! within-pair correlation ~0) through Wald/unpaired, Trinomial/unpaired, and
//! Trinomial/paired — the informative result is "paired doesn't blow up",
//! i.e. mechanism robustness, not correlation absorption (v012 has none to
//! absorb). Also validates this file's fast pre-netted-pair shortcut (used in
//! Part 2 to keep 1000-replicate Monte Carlo runs fast) against veridict's
//! real `paired_by_id=true` code path on that same real data.
//!
//! Part 2: synthetic sequential Monte Carlo (checked every 13 pairs, mirrors
//! roughly one gateA-sized sprint; stops at the first SPRT bound crossed or
//! at the pre-registered 800-pair/1600-game cap) across a small grid of
//! (intra-pair correlation rho, true win probability) scenarios, comparing
//! Wald+unpaired's and Trinomial+paired's false-accept-H1 rate under H0 and
//! power under H1.

use std::collections::HashMap;
use std::fs;

use veridict::input::Record;
use veridict::sprt::{SprtConfig, SprtVariant, run as sprt_run};
use veridict::stats::sprt::{bounds, llr_delta, score_from_elo};

// ponytail: duplicated from sekirei-match-runner's own `main.rs` Lcg rather
// than imported -- this crate has no [lib] target (bin-only), so an
// `examples/` binary can't reach anything in `main.rs`. Same MMIX constants.
struct Lcg(u64);
impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    /// Uniform f64 in [0, 1).
    fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn bernoulli(&mut self, p: f64) -> bool {
        self.uniform() < p
    }
}

/// Pulls `"key":"value"` out of a flat single-line JSON object -- avoids
/// pulling in serde_json just for this one fixed, self-generated shape
/// (`{"id":"...","result":"..."}`, written by this same match-runner).
fn json_str_field<'a>(line: &'a str, key: &str) -> &'a str {
    let needle = format!("\"{key}\":\"");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("no {key} field in: {line}"))
        + needle.len();
    let rest = &line[start..];
    let end = rest
        .find('"')
        .unwrap_or_else(|| panic!("unterminated {key} field in: {line}"));
    &rest[..end]
}

fn rec(id: Option<String>, result: &str) -> (usize, Record) {
    (
        0,
        Record {
            id,
            baseline: None,
            candidate: None,
            result: Some(result.to_string()),
            baseline_status: None,
            candidate_status: None,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict3 {
    Pass,
    Fail,
    Inconclusive,
}

fn to_verdict3(v: veridict::Verdict) -> Verdict3 {
    match v {
        veridict::Verdict::Pass => Verdict3::Pass,
        veridict::Verdict::Fail => Verdict3::Fail,
        veridict::Verdict::Inconclusive => Verdict3::Inconclusive,
    }
}

// ---- Part 1: real v012 data ----

fn part1_v012() {
    println!("=== Part 1: v012-vs-v010 real data (400 games, correlation ~0) ===");
    let path = "sprint_gate_runs/v012_vs_v010_20260708/combined.jsonl";
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));

    // Original ids look like "sprintNN_gameMMMM". Reconstruct the removed
    // pair_id() scheme: within each sprint's local game sequence (gpp=4),
    // consecutive pairs of 2 games (1-2, 3-4, ...) are exactly one
    // same-position/opposite-color pair each (gpp=4's B,W,B,W color pattern
    // makes every consecutive 2-block same-position-opposite-color, and 4 is
    // evenly divisible by 2, so this never straddles a position boundary).
    let mut unpaired: Vec<(usize, Record)> = Vec::new();
    let mut paired: Vec<(usize, Record)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let id = json_str_field(line, "id").to_string();
        let result = json_str_field(line, "result").to_string();
        let (sprint, local_game) = {
            let (s, g) = id.split_once("_game").expect("id shape sprintNN_gameMMMM");
            (s.to_string(), g.parse::<u64>().expect("game number"))
        };
        let pair_idx = (local_game - 1) / 2;
        unpaired.push(rec(Some(id.clone()), &result));
        paired.push(rec(Some(format!("{sprint}_pair{pair_idx}")), &result));
        let _ = i;
    }
    println!("loaded {} games", unpaired.len());

    let config = SprtConfig::new(0.0, 20.0, 0.05, 0.05).unwrap();

    let wald_unpaired = sprt_run(
        unpaired.iter().cloned().map(Ok),
        &config,
        SprtVariant::Wald,
        false,
    )
    .unwrap();
    let trinomial_unpaired = sprt_run(
        unpaired.iter().cloned().map(Ok),
        &config,
        SprtVariant::Trinomial,
        false,
    )
    .unwrap();
    let trinomial_paired = sprt_run(
        paired.iter().cloned().map(Ok),
        &config,
        SprtVariant::Trinomial,
        true,
    )
    .unwrap();

    for (label, r) in [
        (
            "Wald + unpaired (current production default)",
            &wald_unpaired,
        ),
        ("Trinomial + unpaired", &trinomial_unpaired),
        ("Trinomial + paired", &trinomial_paired),
    ] {
        println!(
            "{label}: llr={:.3} verdict={:?} w/d/l={}/{}/{} drawelo={:?}",
            r.llr, r.verdict, r.candidate_wins, r.draws, r.baseline_wins, r.drawelo
        );
    }
    println!(
        "sanity: trinomial_paired split-pair rate = {}/{} (2026-07-09 entry reported 96/200 = 48%)",
        trinomial_paired.draws,
        trinomial_paired.candidate_wins + trinomial_paired.baseline_wins + trinomial_paired.draws
    );

    // Equivalence check: Part 2's Monte Carlo uses a fast shortcut (net each
    // pair to a single pre-computed outcome, feed it with paired_by_id=false
    // instead of feeding 2 same-id records with paired_by_id=true) to avoid
    // materializing up to 1600 records per check across 1000s of replicates.
    // Validate that shortcut here against the real paired_by_id=true path,
    // on real data, before trusting it in Part 2.
    let mut pair_groups: HashMap<String, Vec<&str>> = HashMap::new();
    for (_, r) in &paired {
        pair_groups
            .entry(r.id.clone().unwrap())
            .or_default()
            .push(r.result.as_deref().unwrap());
    }
    let mut net_records: Vec<(usize, Record)> = Vec::new();
    for outcomes in pair_groups.values() {
        assert_eq!(outcomes.len(), 2, "expected exactly 2 games per pair");
        let points = |o: &str| match o {
            "candidate_win" => 1.0,
            "draw" => 0.5,
            "baseline_win" => 0.0,
            other => panic!("unexpected outcome {other}"),
        };
        let total = points(outcomes[0]) + points(outcomes[1]);
        let net = if total > 1.0 {
            "candidate_win"
        } else if total < 1.0 {
            "baseline_win"
        } else {
            "draw"
        };
        net_records.push(rec(None, net));
    }
    let trinomial_prenetted = sprt_run(
        net_records.iter().cloned().map(Ok),
        &config,
        SprtVariant::Trinomial,
        false,
    )
    .unwrap();
    assert_eq!(
        trinomial_prenetted.candidate_wins,
        trinomial_paired.candidate_wins
    );
    assert_eq!(
        trinomial_prenetted.baseline_wins,
        trinomial_paired.baseline_wins
    );
    assert_eq!(trinomial_prenetted.draws, trinomial_paired.draws);
    assert!((trinomial_prenetted.llr - trinomial_paired.llr).abs() < 1e-9);
    println!(
        "equivalence check OK: pre-netted shortcut matches real paired_by_id=true (llr diff={:.2e})",
        (trinomial_prenetted.llr - trinomial_paired.llr).abs()
    );
    println!();
}

// ---- Part 2: synthetic Monte Carlo ----

fn cap_pairs() -> u64 {
    std::env::var("CAP_PAIRS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(800)
}
fn check_every_pairs() -> u64 {
    std::env::var("CHECK_EVERY_PAIRS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(13)
}
fn n_sims() -> u32 {
    std::env::var("N_SIMS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
}

struct ScenarioResult {
    pass: u32,
    fail: u32,
    inconclusive: u32,
}
impl ScenarioResult {
    fn new() -> Self {
        Self {
            pass: 0,
            fail: 0,
            inconclusive: 0,
        }
    }
    fn record(&mut self, v: Verdict3) {
        match v {
            Verdict3::Pass => self.pass += 1,
            Verdict3::Fail => self.fail += 1,
            Verdict3::Inconclusive => self.inconclusive += 1,
        }
    }
    fn n(&self) -> u32 {
        self.pass + self.fail + self.inconclusive
    }
}

/// Runs one simulated Gate-B-shaped match to completion (bound crossed or
/// cap reached) and returns both arms' verdicts on the *same* realization
/// (same games), so the two arms are directly comparable per replicate.
///
/// Correlation model: each position's true candidate win probability is
/// `true_p +/- delta` (coin flip, delta = sqrt(rho * true_p * (1-true_p))),
/// so both games at that position are Bernoulli draws from the *same*
/// per-position probability -- this is the exact, distribution-agnostic
/// intraclass-correlation identity (Cov(X1,X2 | shared p_i) = Var(p_i),
/// Var(X_game) = true_p*(1-true_p), so Cov/Var = rho by construction),
/// not tied to any particular shape for the per-position distribution.
/// Games are decisive-only (no synthetic per-game draws) -- matches gateA's
/// real end-reason distribution (395/396 decisive) and is the regime where
/// pairing losslessly collapses the pentanomial {LL,LD,split,WD,WW} down to
/// the trinomial {LL,split,WW} the Trinomial variant models exactly.
fn simulate_one(
    rng: &mut Lcg,
    true_p: f64,
    rho: f64,
    config: &SprtConfig,
    cap_pairs: u64,
    check_every_pairs: u64,
) -> (Verdict3, Verdict3) {
    let delta = (rho * true_p * (1.0 - true_p)).sqrt();
    let bounds_ = bounds(config.alpha, config.beta);
    let p0 = score_from_elo(config.elo0);
    let p1 = score_from_elo(config.elo1);

    let (mut cand_games, mut base_games) = (0u64, 0u64);
    let mut wald_verdict = Verdict3::Inconclusive;
    let mut trinomial_verdict = Verdict3::Inconclusive;
    let mut pair_records: Vec<(usize, Record)> = Vec::with_capacity(cap_pairs as usize);

    for pair_idx in 0..cap_pairs {
        let p_i = if rng.bernoulli(0.5) {
            (true_p + delta).clamp(0.001, 0.999)
        } else {
            (true_p - delta).clamp(0.001, 0.999)
        };
        let g1 = rng.bernoulli(p_i);
        let g2 = rng.bernoulli(p_i);
        if g1 {
            cand_games += 1;
        } else {
            base_games += 1;
        }
        if g2 {
            cand_games += 1;
        } else {
            base_games += 1;
        }
        let net = match (g1, g2) {
            (true, true) => "candidate_win",
            (false, false) => "baseline_win",
            _ => "draw",
        };
        pair_records.push(rec(None, net));

        let is_last = pair_idx + 1 == cap_pairs;
        if (pair_idx + 1) % check_every_pairs != 0 && !is_last {
            continue;
        }

        if wald_verdict == Verdict3::Inconclusive {
            let llr = cand_games as f64 * llr_delta(true, p0, p1)
                + base_games as f64 * llr_delta(false, p0, p1);
            wald_verdict = if llr >= bounds_.upper {
                Verdict3::Pass
            } else if llr <= bounds_.lower {
                Verdict3::Fail
            } else {
                Verdict3::Inconclusive
            };
        }
        if trinomial_verdict == Verdict3::Inconclusive {
            let report = sprt_run(
                pair_records.iter().cloned().map(Ok),
                config,
                SprtVariant::Trinomial,
                false,
            )
            .unwrap();
            trinomial_verdict = to_verdict3(report.verdict);
        }
        if wald_verdict != Verdict3::Inconclusive && trinomial_verdict != Verdict3::Inconclusive {
            break;
        }
    }
    (wald_verdict, trinomial_verdict)
}

fn part2_monte_carlo() {
    let cap_pairs = cap_pairs();
    let check_every_pairs = check_every_pairs();
    let n_sims = n_sims();
    println!(
        "=== Part 2: synthetic sequential Monte Carlo (cap={cap_pairs} pairs, check every {check_every_pairs} pairs) ==="
    );
    let config = SprtConfig::new(0.0, 20.0, 0.05, 0.05).unwrap();

    // rho=0.15 matches gateA's observed sweep-rate excess at n=4/position
    // (24.2% vs 12.5% independence prediction) -- see derivation in the
    // 2026-07-12 lessons.md entry this harness supports. rho=0.3 is the
    // user's original stress-test value.
    let scenarios: [(&str, f64, f64); 4] = [
        ("H0, rho=0 (independence sanity check)", 0.5, 0.0),
        ("H0, rho=0.15 (gateA-calibrated)", 0.5, 0.15),
        ("H0, rho=0.3 (stress)", 0.5, 0.3),
        (
            "H1 elo=+20, rho=0.15 (power check)",
            score_from_elo(20.0),
            0.15,
        ),
    ];

    for (label, true_p, rho) in scenarios {
        let mut wald = ScenarioResult::new();
        let mut trinomial = ScenarioResult::new();
        let mut rng = Lcg(0x5EED_0000 ^ (label.len() as u64));
        for _ in 0..n_sims {
            let (w, t) = simulate_one(&mut rng, true_p, rho, &config, cap_pairs, check_every_pairs);
            wald.record(w);
            trinomial.record(t);
        }
        println!("--- {label} (n={n_sims}) ---");
        println!(
            "  Wald+unpaired:      PASS={:>4} ({:>5.1}%)  FAIL={:>4}  INCONCLUSIVE={:>4}",
            wald.pass,
            100.0 * wald.pass as f64 / wald.n() as f64,
            wald.fail,
            wald.inconclusive
        );
        println!(
            "  Trinomial+paired:   PASS={:>4} ({:>5.1}%)  FAIL={:>4}  INCONCLUSIVE={:>4}",
            trinomial.pass,
            100.0 * trinomial.pass as f64 / trinomial.n() as f64,
            trinomial.fail,
            trinomial.inconclusive
        );
    }
}

// ---- Part 3: elo1 x cap sizing grid, Trinomial+paired only ----
//
// Trinomial+paired is fixed by the 2026-07-12 verification above; this part
// answers a different question -- given that choice, how does the
// pre-registered cap trade off against elo1 in terms of PASS/FAIL/
// INCONCLUSIVE rates and how many pairs a decisive run actually consumes?
// SPRT's whole point is that clear-cut cases resolve fast, so the cap should
// mostly function as a rarely-paid worst-case insurance policy, not a
// typical cost -- mean/p90 pairs-consumed is how that gets checked, not
// asserted.

/// Like `simulate_one`'s Trinomial arm, but returns how many pairs were
/// actually consumed before a bound was crossed (or the cap, if never
/// decisive) -- `simulate_one` only reports the verdict.
fn simulate_trinomial_paired(
    rng: &mut Lcg,
    true_p: f64,
    rho: f64,
    config: &SprtConfig,
    cap_pairs: u64,
    check_every_pairs: u64,
) -> (Verdict3, u64) {
    let delta = (rho * true_p * (1.0 - true_p)).sqrt();
    let mut pair_records: Vec<(usize, Record)> = Vec::with_capacity(cap_pairs as usize);

    for pair_idx in 0..cap_pairs {
        let p_i = if rng.bernoulli(0.5) {
            (true_p + delta).clamp(0.001, 0.999)
        } else {
            (true_p - delta).clamp(0.001, 0.999)
        };
        let net = match (rng.bernoulli(p_i), rng.bernoulli(p_i)) {
            (true, true) => "candidate_win",
            (false, false) => "baseline_win",
            _ => "draw",
        };
        pair_records.push(rec(None, net));

        let is_last = pair_idx + 1 == cap_pairs;
        if (pair_idx + 1) % check_every_pairs != 0 && !is_last {
            continue;
        }
        let report = sprt_run(
            pair_records.iter().cloned().map(Ok),
            config,
            SprtVariant::Trinomial,
            false,
        )
        .unwrap();
        let verdict = to_verdict3(report.verdict);
        if verdict != Verdict3::Inconclusive || is_last {
            return (verdict, pair_idx + 1);
        }
    }
    unreachable!("loop always returns by is_last")
}

fn mean(xs: &[u64]) -> f64 {
    xs.iter().sum::<u64>() as f64 / xs.len() as f64
}

fn percentile(xs: &mut [u64], p: f64) -> u64 {
    xs.sort_unstable();
    let idx = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[idx]
}

fn part3_grid() {
    let n_sims = n_sims();
    let check_every_pairs = check_every_pairs();
    println!("=== Part 3: Trinomial+paired, rho=0.15, elo1 x cap grid (n={n_sims}/cell) ===");

    let true_elos: [f64; 5] = [0.0, 10.0, 15.0, 20.0, 30.0];
    let elo1s: [f64; 2] = [15.0, 20.0];
    let caps: [u64; 3] = [800, 1600, 2400];
    let rho = 0.15;

    println!(
        "{:>9} {:>6} {:>6} | {:>7} {:>7} {:>7} | {:>9} {:>9}",
        "true_elo", "elo1", "cap", "PASS%", "FAIL%", "INC%", "mean_prs", "p90_prs"
    );
    for elo1 in elo1s {
        let config = SprtConfig::new(0.0, elo1, 0.05, 0.05).unwrap();
        for true_elo in true_elos {
            let true_p = score_from_elo(true_elo);
            for cap in caps {
                let mut result = ScenarioResult::new();
                let mut consumed: Vec<u64> = Vec::with_capacity(n_sims as usize);
                let mut rng =
                    Lcg(0xC0FF_EE00 ^ (elo1 as u64) << 32 ^ (true_elo as u64) << 16 ^ cap);
                for _ in 0..n_sims {
                    let (v, n) = simulate_trinomial_paired(
                        &mut rng,
                        true_p,
                        rho,
                        &config,
                        cap,
                        check_every_pairs,
                    );
                    result.record(v);
                    consumed.push(n);
                }
                println!(
                    "{:>9.0} {:>6.0} {:>6} | {:>6.1}% {:>6.1}% {:>6.1}% | {:>9.0} {:>9}",
                    true_elo,
                    elo1,
                    cap,
                    100.0 * result.pass as f64 / n_sims as f64,
                    100.0 * result.fail as f64 / n_sims as f64,
                    100.0 * result.inconclusive as f64 / n_sims as f64,
                    mean(&consumed),
                    percentile(&mut consumed, 0.90),
                );
            }
        }
    }
}

// ---- Part 4: cheap background check on the "dilution, not a defect" hypothesis ----
//
// At true_elo=30/rho=0.15, Trinomial+paired needed ~600-730 pairs on average
// to resolve (Part 3), well above a naive "~200-300 games" chess-engine-
// testing intuition. Hypothesis (2026-07-12 lessons.md open question):
// Wald+unpaired looks faster only because it double-counts each pair's
// shared position-level signal as if the two games were independent -- the
// same mechanism Part 2 showed inflates its false-accept rate under H0.
// This isn't a rerun of Part 2 (which fixes elo1=20); it directly compares
// games-consumed vs pairs-consumed*2 on the same true_elo=30 scenario Part
// 3 flagged, to see the "dilution" concretely in sample-size terms.
fn simulate_wald_unpaired_games(
    rng: &mut Lcg,
    true_p: f64,
    rho: f64,
    config: &SprtConfig,
    cap_games: u64,
    check_every_games: u64,
) -> (Verdict3, u64) {
    let delta = (rho * true_p * (1.0 - true_p)).sqrt();
    let bounds_ = bounds(config.alpha, config.beta);
    let p0 = score_from_elo(config.elo0);
    let p1 = score_from_elo(config.elo1);
    let (mut cand, mut base) = (0u64, 0u64);
    let mut games = 0u64;

    while games < cap_games {
        let p_i = if rng.bernoulli(0.5) {
            (true_p + delta).clamp(0.001, 0.999)
        } else {
            (true_p - delta).clamp(0.001, 0.999)
        };
        for _ in 0..2 {
            if rng.bernoulli(p_i) {
                cand += 1;
            } else {
                base += 1;
            }
            games += 1;
        }

        if !games.is_multiple_of(check_every_games) && games < cap_games {
            continue;
        }
        let llr = cand as f64 * llr_delta(true, p0, p1) + base as f64 * llr_delta(false, p0, p1);
        let verdict = if llr >= bounds_.upper {
            Verdict3::Pass
        } else if llr <= bounds_.lower {
            Verdict3::Fail
        } else {
            Verdict3::Inconclusive
        };
        if verdict != Verdict3::Inconclusive || games >= cap_games {
            return (verdict, games);
        }
    }
    unreachable!()
}

fn part4_dilution_check() {
    let n_sims = n_sims();
    println!(
        "=== Part 4: dilution-hypothesis background check (true_elo=30, rho=0.15, elo1=20) ==="
    );
    let config = SprtConfig::new(0.0, 20.0, 0.05, 0.05).unwrap();
    let true_p = score_from_elo(30.0);
    let rho = 0.15;

    let mut trinomial_pairs: Vec<u64> = Vec::with_capacity(n_sims as usize);
    let mut trinomial_result = ScenarioResult::new();
    let mut rng_t = Lcg(0xD11E_0001);
    for _ in 0..n_sims {
        let (v, n) = simulate_trinomial_paired(&mut rng_t, true_p, rho, &config, 2400, 13);
        trinomial_result.record(v);
        trinomial_pairs.push(n);
    }

    let mut wald_games: Vec<u64> = Vec::with_capacity(n_sims as usize);
    let mut wald_result = ScenarioResult::new();
    let mut rng_w = Lcg(0xD11E_0002);
    for _ in 0..n_sims {
        let (v, n) = simulate_wald_unpaired_games(&mut rng_w, true_p, rho, &config, 4800, 26);
        wald_result.record(v);
        wald_games.push(n);
    }

    println!(
        "  Trinomial+paired: PASS={:.1}%  mean_pairs={:.0} (={:.0} games-equivalent)  p90_pairs={}",
        100.0 * trinomial_result.pass as f64 / n_sims as f64,
        mean(&trinomial_pairs),
        2.0 * mean(&trinomial_pairs),
        percentile(&mut trinomial_pairs.clone(), 0.90),
    );
    println!(
        "  Wald+unpaired:    PASS={:.1}%  mean_games={:.0}                          p90_games={}",
        100.0 * wald_result.pass as f64 / n_sims as f64,
        mean(&wald_games),
        percentile(&mut wald_games.clone(), 0.90),
    );
}

fn main() {
    part1_v012();
    part2_monte_carlo();
    part3_grid();
    part4_dilution_check();
}
