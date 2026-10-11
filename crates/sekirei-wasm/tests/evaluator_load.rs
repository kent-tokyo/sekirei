//! Process-isolated provenance test for browser-style evaluator bytes.

use sekirei_core::nnue::{NnueWeights, weights_to_bytes};
use sekirei_core::sfen::STARTPOS_SFEN;
use sekirei_wasm::{analyze_position, computer_move, load_evaluator};
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn loaded_evaluator_provenance_is_frozen_into_results() {
    let bytes = weights_to_bytes(&NnueWeights::default_lcg()).unwrap();
    let expected_hash = sha256_hex(&bytes);
    let loaded = load_evaluator(&bytes).expect("valid in-memory SEKIRW must load");

    assert_eq!(loaded.format(), "SEKIRW01");
    assert_eq!(loaded.byte_count(), bytes.len() as u32);
    assert_eq!(loaded.weight_sha256(), expected_hash);
    assert_eq!(loaded.evaluator_id(), "sekirei-nnue");
    assert_eq!(loaded.evaluator_version(), "SEKIRW01");

    let analysis = analyze_position(STARTPOS_SFEN, 1, 10_000).unwrap();
    assert_eq!(analysis.evaluator_id(), "sekirei-nnue");
    assert_eq!(analysis.evaluator_version(), "SEKIRW01");
    assert_eq!(
        analysis.evaluator_weight_sha256(),
        Some(expected_hash.clone())
    );

    let reply = computer_move(STARTPOS_SFEN, 1, 10_000).unwrap();
    assert_eq!(reply.evaluator_id(), "sekirei-nnue");
    assert_eq!(reply.evaluator_version(), "SEKIRW01");
    assert_eq!(reply.evaluator_weight_sha256(), Some(expected_hash));
}
