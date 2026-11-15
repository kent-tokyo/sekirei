//! Bounded existence proof and legal-defense replay for the fixed NM01b position.

use std::time::{Duration, Instant};

use sekirei_core::board::Board;
use sekirei_core::mate::{prove_mate_existence, MatePositionPolicy, MateProofLimits};
use sekirei_core::sfen::move_to_usi;

fn main() {
    const SFEN: &str =
        "l+P5nl/2+S3k2/p2+R1gspp/4ppp2/7N1/4PPP2/P+p3G1PP/3+p2K2/L7+r b BGPbg2s2nl2p 81";
    let board = Board::from_sfen(SFEN).expect("NM01b SFEN must be legal");
    let proof = prove_mate_existence(
        &board,
        MateProofLimits {
            search_nodes: 500_000,
            max_ply: 25,
            deadline: Some(Instant::now() + Duration::from_secs(3)),
            extraction_nodes: 100_000,
            verification_nodes: 100_000,
        },
        MatePositionPolicy::RequireBothKings,
    );
    let line = proof.tree.as_ref().map(|tree| {
        tree.representative_line()
            .into_iter()
            .map(move_to_usi)
            .collect::<Vec<_>>()
    });
    println!(
        "verified={} invalid={:?} stop={:?} search_nodes={} extraction_nodes={} verification_nodes={} longest={:?} line={:?}",
        proof.tree.is_some(),
        proof.invalid_reason,
        proof.stop_reason,
        proof.search_nodes,
        proof.extraction_nodes,
        proof.verification_nodes,
        proof.tree.as_ref().map(|tree| tree.longest_line_ply()),
        line,
    );
}
