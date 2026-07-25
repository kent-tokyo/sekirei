//! Minimal repro for the `nnue::WEIGHTS` process-global `OnceLock` losing
//! independence between two "engine instances" in one process (Sprint 2
//! search_ablation P0 follow-up; see
//! `docs/experiments/search_ablation_multiweight_repro.md`).
//!
//! Writes two distinct (trivially, deliberately different) weight files to a
//! temp dir, calls `nnue::load_weights` on each in turn, and reports whether
//! the second load actually took effect. Expected (per `nnue.rs:198-206`'s
//! `OnceLock::set` semantics): it does not -- the second call logs "already
//! loaded; ignoring duplicate load" and `weights()` keeps serving the first
//! file's values for the rest of the process's life.
//!
//! Deliberately an example, not a `#[test]`: this demonstrates a known
//! current limitation, it does not guard a regression -- the permanent
//! regression test (two independent `Engine` instances) is deferred to the
//! EvalFile-reload implementation itself (see the design doc's test plan).
//!
//! Usage: cargo run -p sekirei-core --example repro_multiweight_onelock

use sekirei_core::nnue::{self, L1, NnueWeights};

fn weights_with_ft_bias(bias_value: i16) -> NnueWeights {
    let mut w = NnueWeights::default_lcg();
    w.ft_bias = [bias_value; L1];
    w
}

fn main() {
    let dir = std::env::temp_dir().join("sekirei_repro_multiweight_onelock");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path_a = dir.join("weights_a.bin");
    let path_b = dir.join("weights_b.bin");

    nnue::save_weights(&weights_with_ft_bias(7), &path_a).expect("write weights A");
    nnue::save_weights(&weights_with_ft_bias(-7), &path_b).expect("write weights B");

    println!("loading weights A (ft_bias[0] should become 7)...");
    nnue::load_weights(&path_a).expect("load A");
    let after_a = nnue::weights().ft_bias[0];
    println!("  weights().ft_bias[0] = {after_a}");
    assert_eq!(after_a, 7, "first load should always take effect");

    println!(
        "loading weights B into the SAME process (ft_bias[0] should become -7 if independent)..."
    );
    nnue::load_weights(&path_b)
        .expect("load B (call succeeds; the question is whether it took effect)");
    let after_b = nnue::weights().ft_bias[0];
    println!("  weights().ft_bias[0] = {after_b}");

    if after_b == -7 {
        println!(
            "RESULT: second load took effect -- WEIGHTS is no longer a single-shot OnceLock (bug fixed)."
        );
    } else if after_b == 7 {
        println!(
            "RESULT: second load was silently ignored (still serving weights A's value). \
             This is the documented `OnceLock` limitation -- see the 'already loaded; ignoring \
             duplicate load' line nnue.rs:203 logs to stderr above. Confirms: within one process, \
             a second `load_weights()` call cannot give a second 'engine instance' independent weights."
        );
    } else {
        println!(
            "RESULT: unexpected value {after_b} -- investigate before trusting this repro further."
        );
    }
}
