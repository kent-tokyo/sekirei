//! Node profile per iterative-deepening depth.
//!
//! ```text
//! depth_profile <nn.bin> <positions.txt> [count] [max_depth] [fv_scale]
//! ```
//!
//! `positions.txt` holds USI `position ...` lines. For every position a fresh
//! single-thread search runs to `max_depth` once per depth, and the mean
//! alpha-beta calls, quiescence calls and total nodes per depth are printed,
//! with the ratio to the previous depth (effective branching factor).
//! Diagnostics only.

use sekirei_core::halfkp;
use sekirei_core::nnue::load_evaluator;
use sekirei_core::search::{SearchConfig, SearchDiagnostics, Searcher};
use sekirei_core::sfen::parse_position_cmd_with_history;
use sekirei_core::tt::Tt;
use std::path::Path;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    load_evaluator(Path::new(&args[0])).expect("load evaluator");
    let text = std::fs::read_to_string(&args[1]).expect("positions");
    let count: usize = args.get(2).map_or(20, |s| s.parse().unwrap());
    let max_depth: u32 = args.get(3).map_or(8, |s| s.parse().unwrap());
    if let Some(scale) = args.get(4) {
        halfkp::set_fv_scale(scale.parse().unwrap()).unwrap();
    }
    let positions: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("position "))
        .take(count)
        .collect();
    let mut sums = vec![[0f64; 3]; max_depth as usize + 1];
    for body in &positions {
        for depth in 1..=max_depth {
            let (mut board, history) = parse_position_cmd_with_history(body).expect("parse");
            let diagnostics = Arc::new(SearchDiagnostics::new());
            let searcher = Searcher::with_diagnostics(Tt::new(64), diagnostics.clone());
            let info = searcher.search_with_history(
                &mut board,
                SearchConfig {
                    max_depth: depth,
                    ..SearchConfig::default()
                },
                &history,
            );
            let snap = diagnostics.snapshot();
            let s = &mut sums[depth as usize];
            s[0] += snap.alpha_beta_calls as f64;
            s[1] += snap.quiescence_calls as f64;
            s[2] += info.nodes as f64;
        }
    }
    let n = positions.len() as f64;
    let mut prev = 0f64;
    for (depth, sum) in sums.iter().enumerate().skip(1) {
        let [ab, qs, nodes] = sum.map(|v| v / n);
        let ebf = if prev > 0.0 { nodes / prev } else { 0.0 };
        println!("depth {depth:2} ab {ab:10.0} qs {qs:10.0} nodes {nodes:10.0} ebf {ebf:5.2}");
        prev = nodes;
    }
}
