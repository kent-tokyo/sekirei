//! Synthetic builder -> existing loader -> legal decision contract.
#![cfg(feature = "cyclic-book")]

#[allow(dead_code)]
#[path = "../examples/build_cyclic_book.rs"]
mod builder;
#[path = "../src/cyclic_book.rs"]
mod loader;

use std::fs;
use std::path::Path;

use sekirei_core::{
    board::Board,
    book_graph::BookGraphDecision,
    sfen::{board_to_sfen, move_from_usi, move_to_usi},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const ORIGIN: &str = "4k4/9/4P4/9/9/9/9/9/4K4 b G 1";

fn fixture() -> Value {
    let root = Board::from_sfen(ORIGIN).unwrap();
    let mut mate = root.clone();
    mate.do_move(move_from_usi("G*5b", &root).unwrap());
    json!({
        "schema": "sekirei.cyclic-book-positions", "version": 1,
        "producer": "synthetic-builder-contract", "source": "synthetic",
        "source_declaration": "Composed two-position fixture for a legality test; no match data.",
        "verification": {"max_depth": 4, "node_limit": 1000},
        "positions": [ORIGIN, board_to_sfen(&mate)]
    })
}

fn input(dir: &Path, value: &Value) -> std::path::PathBuf {
    let path = dir.join("positions.json");
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn generated_bundle_loads_and_selects_a_legal_mate_with_bound_hashes() {
    let dir = TempDir::new().unwrap();
    let input = input(dir.path(), &fixture());
    let output = dir.path().join("bundle");
    builder::build(&input, &output).unwrap();
    builder::verify(&input, &output).unwrap();
    let artifact = read_json(&output.join("book.json"));
    assert!(
        artifact["nodes"][0]["successors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["target"].is_null())
    );
    let proof = read_json(&output.join("provenance.json"));
    assert_eq!(proof["schema"], "sekirei.cyclic-book-provenance");
    assert_eq!(
        proof["input_sha256"],
        format!("{:x}", Sha256::digest(fs::read(&input).unwrap()))
    );
    assert_eq!(
        proof["artifact_sha256"],
        format!(
            "{:x}",
            Sha256::digest(fs::read(output.join("book.json")).unwrap())
        )
    );
    assert_eq!(proof["ownership_verified"], false);
    let loaded = loader::CyclicBook::load(output.join("book.json").to_str().unwrap()).unwrap();
    let root = Board::from_sfen(ORIGIN).unwrap();
    let BookGraphDecision::ProvenMate {
        best_move,
        proof_plies,
    } = loaded.decision(Some(&format!("sfen {ORIGIN}")), &root)
    else {
        panic!("mate proof expected")
    };
    assert_eq!(move_to_usi(best_move), "G*5b");
    assert_eq!(proof_plies, 1);
    assert!(move_from_usi(&move_to_usi(best_move), &root).is_ok());
}

#[test]
fn deterministic_builds_preserve_unknown_boundaries_without_expansion() {
    let dir = TempDir::new().unwrap();
    let mut value = fixture();
    value["positions"].as_array_mut().unwrap().truncate(1);
    let input = input(dir.path(), &value);
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    builder::build(&input, &first).unwrap();
    builder::build(&input, &second).unwrap();
    for name in ["book.json", "provenance.json"] {
        assert_eq!(
            fs::read(first.join(name)).unwrap(),
            fs::read(second.join(name)).unwrap()
        );
    }
    let artifact = read_json(&first.join("book.json"));
    assert_eq!(artifact["nodes"].as_array().unwrap().len(), 1);
    assert!(
        artifact["nodes"][0]["successors"]
            .as_array()
            .unwrap()
            .iter()
            .all(|edge| edge["target"].is_null())
    );
    let loaded = loader::CyclicBook::load(first.join("book.json").to_str().unwrap()).unwrap();
    assert!(matches!(
        loaded.decision(
            Some(&format!("sfen {ORIGIN}")),
            &Board::from_sfen(ORIGIN).unwrap()
        ),
        BookGraphDecision::Fallback(_)
    ));
}

#[test]
fn input_artifact_and_provenance_tampering_are_detected() {
    let dir = TempDir::new().unwrap();
    let input = input(dir.path(), &fixture());
    let output = dir.path().join("bundle");
    builder::build(&input, &output).unwrap();
    let original = fs::read(output.join("book.json")).unwrap();
    let mut bad = read_json(&output.join("book.json"));
    let old = bad["nodes"][0]["successors"][0]["gave_check"]
        .as_bool()
        .unwrap();
    bad["nodes"][0]["successors"][0]["gave_check"] = json!(!old);
    fs::write(output.join("book.json"), serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(builder::verify(&input, &output).is_err());
    assert!(loader::CyclicBook::load(output.join("book.json").to_str().unwrap()).is_err());
    fs::write(output.join("book.json"), original).unwrap();
    let mut provenance = read_json(&output.join("provenance.json"));
    provenance["ownership_verified"] = json!(true);
    fs::write(
        output.join("provenance.json"),
        serde_json::to_vec(&provenance).unwrap(),
    )
    .unwrap();
    assert!(builder::verify(&input, &output).is_err());

    let second = dir.path().join("second");
    builder::build(&input, &second).unwrap();
    let mut changed = fixture();
    changed["source_declaration"] = json!("Changed declaration");
    fs::write(&input, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(builder::verify(&input, &second).is_err());
}

#[test]
fn existing_output_and_failed_inputs_are_never_overwritten() {
    let dir = TempDir::new().unwrap();
    let input = input(dir.path(), &fixture());
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("sentinel"), "keep").unwrap();
    assert!(builder::build(&input, &existing).is_err());
    assert_eq!(
        fs::read_to_string(existing.join("sentinel")).unwrap(),
        "keep"
    );
    assert!(!existing.join("book.json").exists());
    let bundle = dir.path().join("bundle");
    builder::build(&input, &bundle).unwrap();
    let before = fs::read(bundle.join("book.json")).unwrap();
    assert!(builder::build(&input, &bundle).is_err());
    assert_eq!(fs::read(bundle.join("book.json")).unwrap(), before);
}

#[test]
fn version_source_duplicate_and_resource_limits_fail_before_output_creation() {
    for mutation in 0..9 {
        let dir = TempDir::new().unwrap();
        let mut bad = fixture();
        match mutation {
            0 => bad["version"] = json!(2),
            1 => bad["source"] = json!("external"),
            2 => bad["ownership_verified"] = json!(true),
            3 => bad["verification"]["max_depth"] = json!(17),
            4 => bad["verification"]["node_limit"] = json!(10001),
            5 => bad["positions"] = json!(vec![ORIGIN; 257]),
            6 => bad["positions"] = json!([ORIGIN, ORIGIN.replace(" G 1", " G 99")]),
            7 => bad["positions"] = json!(["x".repeat(257)]),
            8 => bad["positions"] = json!(["4k4/9/9/9/9/9/9/9/9 b - 1"]),
            _ => unreachable!(),
        }
        let input = input(dir.path(), &bad);
        let output = dir.path().join("bundle");
        assert!(
            builder::build(&input, &output).is_err(),
            "mutation {mutation}"
        );
        assert!(!output.exists());
    }
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("large.json");
    fs::write(&input, vec![b' '; 1_048_577]).unwrap();
    let output = dir.path().join("bundle");
    assert!(builder::build(&input, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn generated_legal_edge_count_is_bounded_before_publication() {
    let rank = |file: u8, king: char| {
        format!(
            "{}{}{}",
            if file < 9 {
                (9 - file).to_string()
            } else {
                String::new()
            },
            king,
            if file > 1 {
                (file - 1).to_string()
            } else {
                String::new()
            }
        )
    };
    let positions = (1..=9)
        .flat_map(|white| {
            (1..=9).map(move |black| {
                format!(
                    "{}/9/9/9/9/9/9/9/{} b 2R2B4G4S4N4L18P 1",
                    rank(white, 'k'),
                    rank(black, 'K')
                )
            })
        })
        .collect::<Vec<_>>();
    let mut value = fixture();
    value["positions"] = json!(positions);
    let dir = TempDir::new().unwrap();
    let input = input(dir.path(), &value);
    let output = dir.path().join("bundle");
    let error = builder::build(&input, &output).unwrap_err();
    assert!(error.contains("16384 legal successors"), "{error}");
    assert!(!output.exists());
}

#[test]
fn owned_source_is_preserved_as_an_unverified_declaration() {
    let dir = TempDir::new().unwrap();
    let mut value = fixture();
    value["source"] = json!("sekirei_owned");
    let input = input(dir.path(), &value);
    let output = dir.path().join("bundle");
    builder::build(&input, &output).unwrap();
    let provenance = read_json(&output.join("provenance.json"));
    assert_eq!(provenance["source"], "sekirei_owned");
    assert_eq!(provenance["ownership_verified"], false);
    assert!(
        builder::run(&["--help".into()])
            .unwrap()
            .contains("build|verify")
    );
    assert!(builder::run(&["build".into()]).is_err());
}
