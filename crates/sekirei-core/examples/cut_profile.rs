//! Move-ordering profile: where the beta cutoffs of the main search fall in
//! the move order, for fixed-depth searches of a position list.
//!
//! ```text
//! cut_profile <nn.bin> <positions.txt> [count] [depth] [fv_scale] [NAME=value ...]
//! ```
//!
//! `positions.txt` holds USI `position ...` lines. Every position gets one
//! fresh single-thread search to `depth`. `NAME=value` sets search parameters
//! (tune builds). Prints the share of cutoffs by the position of the cutoff
//! move (quiet and other cutoff moves), fail-low nodes and total nodes.
//! Diagnostics only.

use sekirei_core::halfkp;
use sekirei_core::nnue::load_evaluator;
use sekirei_core::search::{CUT_BUCKETS, SearchConfig, SearchDiagnostics, Searcher, params};
use sekirei_core::sfen::parse_position_cmd_with_history;
use sekirei_core::tt::Tt;
use std::path::Path;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    load_evaluator(Path::new(&args[0])).expect("load evaluator");
    let text = std::fs::read_to_string(&args[1]).expect("positions");
    let count: usize = args.get(2).map_or(40, |s| s.parse().unwrap());
    let depth: u32 = args.get(3).map_or(10, |s| s.parse().unwrap());
    if let Some(scale) = args.get(4) {
        halfkp::set_fv_scale(scale.parse().unwrap()).unwrap();
    }
    for kv in args.iter().skip(5) {
        let (k, v) = kv.split_once('=').expect("NAME=value");
        let k = k.strip_prefix("T_").unwrap_or(k);
        assert!(
            params::set(k, v.parse().unwrap()),
            "unknown or fixed parameter {k}"
        );
    }
    let positions: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("position "))
        .take(count)
        .collect();
    let mut cuts = [[0u64; CUT_BUCKETS]; 4];
    let mut loops = [0u64; 2];
    let (mut pv_calls, mut lw, mut sw) = (0u64, [0u64; 2], [0u64; 2]);
    let mut rs = [0u64; 3];
    let mut ex = [0u64; 8];
    let mut chk = [0u64; 2];
    let mut table = [[0u64; 9]; 17];
    let mut fail_low = 0u64;
    let mut ttq = [[0u64; 18]; 2];
    let mut ttp = [[0u64; 4]; 17];
    let mut v2n = [[0u64; 4]; 17];
    let mut mk = [[0u64; 8]; 16];
    let mut nodes = 0u64;
    let mut ab = 0u64;
    for body in &positions {
        let (mut board, history) = parse_position_cmd_with_history(body).expect("parse");
        let diagnostics = Arc::new(SearchDiagnostics::new());
        let searcher = Searcher::with_diagnostics(
            Tt::new(
                std::env::var("CP_HASH")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(64),
            ),
            diagnostics.clone(),
        );
        let info = searcher.search_with_history(
            &mut board,
            SearchConfig {
                max_depth: depth,
                ..SearchConfig::default()
            },
            &history,
        );
        let t = diagnostics.depth_table();
        for d in 0..17 {
            for k in 0..9 {
                table[d][k] += t[d][k];
            }
        }
        let c2 = diagnostics.searched_check_counts();
        chk[0] += c2[0];
        chk[1] += c2[1];
        let e8 = diagnostics.exit_counts();
        for i in 0..8 {
            ex[i] += e8[i];
        }
        let r3 = diagnostics.research_counts();
        for i in 0..3 {
            rs[i] += r3[i];
        }
        let (pc, l2, s2) = diagnostics.window_counts();
        pv_calls += pc;
        for i in 0..2 {
            lw[i] += l2[i];
            sw[i] += s2[i];
        }
        let k = diagnostics.move_kind_counts();
        for r in 0..16 {
            for i in 0..8 {
                mk[r][i] += k[r][i];
            }
        }
        let vn = diagnostics.v2_node_counts();
        for r in 0..17 {
            for i in 0..4 {
                v2n[r][i] += vn[r][i];
            }
        }
        let tp = diagnostics.tt_probe_counts();
        for r in 0..17 {
            for i in 0..4 {
                ttp[r][i] += tp[r][i];
            }
        }
        let tq = diagnostics.tt_quiet_cut_counts();
        for r in 0..2 {
            for i in 0..18 {
                ttq[r][i] += tq[r][i];
            }
        }
        let (c, f, l) = diagnostics.cut_histogram();
        loops[0] += l[0];
        loops[1] += l[1];
        for r in 0..4 {
            for b in 0..CUT_BUCKETS {
                cuts[r][b] += c[r][b];
            }
        }
        fail_low += f;
        nodes += info.nodes;
        ab += diagnostics.snapshot().alpha_beta_calls;
    }
    let total: u64 = cuts.iter().flatten().sum();
    let labels = [
        "1", "2", "3", "4", "5-6", "7-8", "9-12", "13-16", "17-32", "33+",
    ];
    println!(
        "positions {} depth {depth} nodes/pos {} ab/pos {} cutoffs {total} fail-low nodes {fail_low} loop nodes without/with TT move {}/{}",
        positions.len(),
        nodes / positions.len() as u64,
        ab / positions.len() as u64,
        loops[0],
        loops[1]
    );
    let per = |a: u64, b: u64| a as f64 / b.max(1) as f64;
    println!(
        "pv calls/pos {}  loop nodes null/pv {}/{}  moves searched per loop node null {:.2} pv {:.2}",
        pv_calls / positions.len() as u64,
        lw[0],
        lw[1],
        per(sw[0], lw[0]),
        per(sw[1], lw[1])
    );
    println!(
        "reduced probes {}  full-depth re-searches {} ({:.1}%)  full-window re-searches {}",
        rs[0],
        rs[1],
        100.0 * per(rs[1], rs[0]),
        rs[2]
    );
    let np = positions.len() as u64;
    println!(
        "later moves searched that give check: null {} of {} ({:.1}%), pv {} of {} ({:.1}%)",
        chk[0],
        sw[0] - lw[0],
        100.0 * per(chk[0], sw[0] - lw[0]),
        chk[1],
        sw[1] - lw[1],
        100.0 * per(chk[1], sw[1] - lw[1])
    );
    println!(
        "exits/pos: depth0 {} tt {} mate1 {} rfp {} razor {} probcut {} null-cut {} null-failed {}",
        ex[0] / np,
        ex[1] / np,
        ex[2] / np,
        ex[3] / np,
        ex[4] / np,
        ex[5] / np,
        ex[6] / np,
        ex[7] / np
    );
    println!(" r     calls    TT razor   RFP   NMP    PC mate1  loop ch/node ch/loop");
    for (d, row) in table.iter().enumerate().skip(1) {
        let n = row[0];
        if n == 0 {
            continue;
        }
        let f = |k: usize| 100.0 * row[k] as f64 / n as f64;
        println!(
            "{d:2} {n:9} {:5.1} {:5.1} {:5.1} {:5.1} {:5.1} {:5.1} {:5.1} {:7.2} {:7.2}",
            f(1),
            f(2),
            f(3),
            f(4),
            f(5),
            f(6),
            f(7),
            row[8] as f64 / n as f64,
            row[8] as f64 / row[7].max(1) as f64
        );
    }
    // Shares within each row: where the cutoff move stood.
    let names = ["no TT, quiet", "no TT, other", "TT, quiet", "TT, other"];
    for (r, name) in names.iter().enumerate() {
        let row: u64 = cuts[r].iter().sum();
        let cells: Vec<String> = labels
            .iter()
            .zip(&cuts[r])
            .map(|(l, &v)| format!("{l}:{:.1}", 100.0 * v as f64 / row.max(1) as f64))
            .collect();
        println!("{name:>13} {row:8} {}", cells.join(" "));
    }
    let first: u64 = (0..4).map(|r| cuts[r][0]).sum();
    let late: u64 = (0..4)
        .map(|r| (6..CUT_BUCKETS).map(|b| cuts[r][b]).sum::<u64>())
        .sum();
    let pct = |v: u64| 100.0 * v as f64 / total.max(1) as f64;
    println!("first move {:.2}%  move 9+ {:.2}%", pct(first), pct(late));
    if mk.iter().any(|row| row[0] > 0) {
        println!(
            "d chk    first     capt    promo  chkdrop  chkmove    qdrop    qmove  checks%(later)"
        );
        for d in 1..8 {
            for ic in 0..2 {
                let row = &mk[d * 2 + ic];
                let later: u64 = row[1..7].iter().sum();
                println!(
                    "{d} {ic}   {}  {:5.1}",
                    row[..7]
                        .iter()
                        .map(|v| format!("{v:8}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                    100.0 * (row[3] + row[4]) as f64 / later.max(1) as f64
                );
            }
        }
    }
    if v2n.iter().any(|row| row[0] > 0) {
        println!(" r  v2nodes  ttpv%  eval>=beta%  incheck%");
        for (r, row) in v2n.iter().enumerate().skip(1) {
            if row[0] > 0 {
                let q = |i: usize| 100.0 * row[i] as f64 / row[0] as f64;
                println!("{r:2} {:8} {:6.1} {:11.1} {:9.1}", row[0], q(1), q(2), q(3));
            }
        }
    }
    println!(" r   probes   hit%  dep>=d%  dep>=d-1%");
    for (r, row) in ttp.iter().enumerate().skip(1) {
        if row[0] > 0 {
            let q = |i: usize| 100.0 * row[i] as f64 / row[0] as f64;
            println!("{r:2} {:8} {:6.1} {:7.1} {:8.1}", row[0], q(1), q(2), q(3));
        }
    }
    println!(
        "quiet TT move: share of its cutoffs made by the TT move, by bound and node depth - entry depth"
    );
    for (b, name) in ["exact", "lower", "upper"].iter().enumerate() {
        let cells: Vec<String> = ["<=0", "1", "2", "3", "4-5", "6+"]
            .iter()
            .enumerate()
            .map(|(g, lab)| {
                let (a, z) = (ttq[0][b * 6 + g], ttq[1][b * 6 + g]);
                format!(
                    "{lab}:{}/{:.1}%",
                    a + z,
                    100.0 * a as f64 / (a + z).max(1) as f64
                )
            })
            .collect();
        println!("  {name:5} {}", cells.join(" "));
    }
}
