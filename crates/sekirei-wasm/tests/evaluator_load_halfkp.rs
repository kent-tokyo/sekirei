//! Process-isolated provenance test for browser-style HalfKP evaluator bytes.

use sekirei_core::halfkp::HalfKpNetwork;
use sekirei_core::nnue::{NnueWeights, install_weights};
use sekirei_core::sfen::STARTPOS_SFEN;
use sekirei_wasm::{analyze_position, load_evaluator};
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn loaded_halfkp_provenance_is_frozen_into_results() {
    let bytes = HalfKpNetwork::random(23).to_bytes();
    let expected_hash = sha256_hex(&bytes);
    let loaded = load_evaluator(&bytes).expect("valid in-memory HalfKP must load");

    assert_eq!(loaded.format(), "HalfKP 256x2-32-32");
    assert_eq!(loaded.byte_count(), bytes.len() as u32);
    assert_eq!(loaded.weight_sha256(), expected_hash);
    assert_eq!(loaded.evaluator_id(), "halfkp");
    assert_eq!(loaded.evaluator_version(), "HalfKP-256x2-32-32-v0x7af32f16");

    let analysis = analyze_position(STARTPOS_SFEN, 1, 10_000).unwrap();
    assert_eq!(analysis.evaluator_id(), "halfkp");
    assert_eq!(
        analysis.evaluator_version(),
        "HalfKP-256x2-32-32-v0x7af32f16"
    );
    assert_eq!(analysis.evaluator_weight_sha256(), Some(expected_hash));

    let error = install_weights(NnueWeights::default_lcg())
        .expect_err("a loaded HalfKP evaluator must reject SEKIRW replacement");
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
}
