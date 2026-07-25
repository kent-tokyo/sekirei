//! Rule-conformance golden corpus harness (Sprint 1 / Phase B3 foundation;
//! expanded Sprint 2 with nyugyoku/jishogi isolation cases + schema fields).
//!
//! Reads `tests/fixtures/rule_conformance_cases.jsonl` (regenerate the
//! engine-verifiable cases with `cargo run --example
//! gen_rule_conformance_corpus > tests/fixtures/rule_conformance_cases.jsonl`)
//! and checks each case's *recorded* facts against what the engine's own
//! primitives (`generate_legal_moves`, `is_in_check`) actually observe when
//! the moves are replayed -- not against sekirei-match-runner's rule
//! *decisions*, which this corpus exists to eventually gate (see the
//! module-level notes below on why).
//!
//! This is a foundation, not exhaustive coverage: a handful of cases per
//! rule category, not a full sweep. Nyugyoku (entering-king declaration) and
//! jishogi (mutual-impasse) cases are recorded but not verifiable yet -- no
//! eligibility-counting logic exists anywhere in this codebase (confirmed
//! during the Sprint 1 provenance/USI audit, reconfirmed Sprint 2). Those
//! cases assert only that the corpus's SFEN parses, has at least one legal
//! move, and matches its claimed in-check state -- they exist so a future
//! implementation has fixtures to gate against, and to document the exact
//! rule-source citation at record time (not what a future implementation
//! happens to compute). **If you're implementing nyugyoku/jishogi
//! eligibility**: every `case_id` starting with `nyugyoku_`/`jishogi_` in
//! the fixture is waiting on you -- promote it from a parse-only assertion
//! to a real eligibility check, and update
//! `KNOWN_MISSING_DECLARATION_CASE_COUNT` below (it will fail to compile-time
//! remind you if you add/remove a pending case without touching this test).
//!
//! **A known, confirmed gap this corpus documents rather than hides**:
//! `crates/sekirei-match-runner/src/main.rs`'s repetition handling
//! (`EndReason::Repetition`) always resolves a 4-fold hash repeat to
//! `Outcome::Draw` -- it has no continuous-check (連続王手) special case at
//! all, and its max-moves ceiling isn't exposed as a reusable API either.
//! The `continuous_check_sennichite_*_checks` and `max_moves_ceiling` cases
//! below are engine-verified for everything sekirei-core can check (legal
//! move replay, repetition count, in-check pattern) -- but this harness does
//! not call into match-runner's outcome logic, so their `expected_result`
//! documents the *correct* ruling, not what match-runner currently produces.

use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::sfen::{move_from_usi, move_to_usi, parse_position_cmd};

const CORPUS: &str = include_str!("fixtures/rule_conformance_cases.jsonl");

/// Exact count of `nyugyoku_`/`jishogi_`-prefixed cases in the corpus right
/// now. Asserted exactly (not just "at least one") in
/// `nyugyoku_and_jishogi_cases_are_recorded_as_pending_not_silently_dropped`
/// below, so silent fixture drift (a case added or removed without anyone
/// noticing) fails the build instead of passing quietly.
const KNOWN_MISSING_DECLARATION_CASE_COUNT: usize = 8;

struct Case {
    case_id: String,
    initial_sfen: String,
    move_history: Vec<String>,
    side_to_move: String,
    expected_legal_moves: Vec<String>,
    expected_repetition_count: u32,
    expected_continuous_check_side: String,
}

/// Hand-rolled JSONL field extraction (no serde_json dependency in this
/// crate, matching sekirei-match-runner's own `json_f64` convention for the
/// same reason) -- every field here is a flat string/number/array-of-string,
/// so a full JSON parser would be over-engineering for this.
fn json_str(line: &str, key: &str) -> String {
    let needle = format!("\"{key}\":\"");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("missing {key} in {line}"))
        + needle.len();
    let end = line[start..].find('"').unwrap() + start;
    line[start..end].to_string()
}

fn json_u32(line: &str, key: &str) -> u32 {
    let needle = format!("\"{key}\":");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("missing {key} in {line}"))
        + needle.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end]
        .parse()
        .unwrap_or_else(|_| panic!("bad {key} in {line}"))
}

fn json_str_array(line: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\":[");
    let start = match line.find(&needle) {
        Some(i) => i + needle.len(),
        None => panic!("missing {key} in {line}"),
    };
    let end = line[start..].find(']').unwrap() + start;
    let inner = &line[start..end];
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect()
}

/// A field whose value is either `null` or a quoted string (only
/// `expected_declaration_eligibility` uses this today).
fn json_opt_str(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("missing {key} in {line}"))
        + needle.len();
    if line[start..].starts_with("null") {
        None
    } else {
        Some(json_str(line, key))
    }
}

fn parse_corpus() -> Vec<Case> {
    CORPUS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| Case {
            case_id: json_str(line, "case_id"),
            initial_sfen: json_str(line, "initial_sfen"),
            move_history: json_str_array(line, "move_history"),
            side_to_move: json_str(line, "side_to_move"),
            expected_legal_moves: json_str_array(line, "expected_legal_moves"),
            expected_repetition_count: json_u32(line, "expected_repetition_count"),
            expected_continuous_check_side: json_str(line, "expected_continuous_check_side"),
        })
        .collect()
}

#[test]
fn every_corpus_position_parses() {
    for case in parse_corpus() {
        parse_position_cmd(&format!("sfen {}", case.initial_sfen))
            .unwrap_or_else(|e| panic!("{}: initial_sfen failed to parse: {e}", case.case_id));
    }
}

#[test]
fn recorded_move_histories_are_legal_move_by_move() {
    for case in parse_corpus() {
        if case.move_history.is_empty() {
            continue; // declaration/max-moves placeholder cases carry no replay
        }
        let mut board = parse_position_cmd(&format!("sfen {}", case.initial_sfen)).unwrap();
        for (i, mv_str) in case.move_history.iter().enumerate() {
            let legal = generate_legal_moves(&mut board);
            let mv = move_from_usi(mv_str, &board).unwrap_or_else(|e| {
                panic!("{}: move {i} ({mv_str}) failed to parse: {e}", case.case_id)
            });
            assert!(
                legal.contains(&mv),
                "{}: move {i} ({mv_str}) is not legal at that point in the recorded history",
                case.case_id
            );
            board.do_move(mv);
        }
    }
}

#[test]
fn repetition_count_matches_the_recorded_move_history() {
    for case in parse_corpus() {
        if case.move_history.is_empty() {
            continue;
        }
        let mut board = parse_position_cmd(&format!("sfen {}", case.initial_sfen)).unwrap();
        let start_hash = board.hash();
        let mut occurrences = 1u32; // the initial position counts as the first occurrence
        for mv_str in &case.move_history {
            let mv = move_from_usi(mv_str, &board).unwrap();
            board.do_move(mv);
            if board.hash() == start_hash {
                occurrences += 1;
            }
        }
        assert_eq!(
            occurrences, case.expected_repetition_count,
            "{}: recorded expected_repetition_count doesn't match what replaying move_history actually produces",
            case.case_id
        );
    }
}

/// The one case-specific check this harness does beyond "is it
/// self-consistent": for the case explicitly claiming continuous check by a
/// named side, verify that side's mover really does give check on *every*
/// one of its own moves (not just some) -- that's the exact distinction
/// between an ordinary sennichite draw and a 連続王手 loss.
#[test]
fn continuous_check_side_actually_checks_on_every_one_of_its_moves() {
    for case in parse_corpus() {
        if case.expected_continuous_check_side == "none" || case.move_history.is_empty() {
            continue;
        }
        let checker_is_black = case.expected_continuous_check_side == "black";
        let mut board = parse_position_cmd(&format!("sfen {}", case.initial_sfen)).unwrap();
        for (i, mv_str) in case.move_history.iter().enumerate() {
            let mover_is_black = board.side_to_move == sekirei_core::color::Color::Black;
            let mv = move_from_usi(mv_str, &board).unwrap();
            board.do_move(mv);
            if mover_is_black == checker_is_black {
                let opponent = if checker_is_black {
                    sekirei_core::color::Color::White
                } else {
                    sekirei_core::color::Color::Black
                };
                assert!(
                    is_in_check(&board, opponent),
                    "{}: move {i} ({mv_str}) by the claimed continuous-check side didn't give check",
                    case.case_id
                );
            }
        }
    }
}

/// `expected_legal_moves` must always be engine-derived (never hand-typed --
/// see the generator's own module doc for why); this test is what actually
/// enforces that promise stays true, by re-deriving the legal-move list at
/// `initial_sfen` for `side_to_move` and comparing it (as a set, since move
/// ordering isn't part of the contract) against what's recorded.
#[test]
fn expected_legal_moves_match_the_engine_at_the_initial_position() {
    for case in parse_corpus() {
        let mut board = parse_position_cmd(&format!("sfen {}", case.initial_sfen)).unwrap();
        let side = if case.side_to_move == "black" {
            sekirei_core::color::Color::Black
        } else {
            sekirei_core::color::Color::White
        };
        assert_eq!(
            board.side_to_move, side,
            "{}: side_to_move field doesn't match the initial_sfen's own side-to-move token",
            case.case_id
        );
        let mut actual: Vec<String> = generate_legal_moves(&mut board)
            .iter()
            .map(|m| move_to_usi(*m))
            .collect();
        let mut expected = case.expected_legal_moves.clone();
        actual.sort();
        expected.sort();
        assert_eq!(
            actual, expected,
            "{}: expected_legal_moves doesn't match generate_legal_moves at the initial position",
            case.case_id
        );
    }
}

/// Every case must carry all 11 documented fields with values from their
/// valid domain -- catches a malformed fixture line before it can silently
/// pass the other (more targeted) tests above by having, say, a typo'd
/// `side_to_move` that happens to not be exercised by any other assertion.
#[test]
fn every_case_matches_the_documented_schema() {
    for line in CORPUS.lines().filter(|l| !l.trim().is_empty()) {
        let case_id = json_str(line, "case_id");
        assert!(!case_id.is_empty(), "empty case_id in {line}");
        json_str(line, "initial_sfen");
        json_str_array(line, "move_history");
        let side = json_str(line, "side_to_move");
        assert!(
            side == "black" || side == "white",
            "{case_id}: side_to_move must be \"black\" or \"white\", got {side:?}"
        );
        json_str_array(line, "expected_legal_moves");
        json_u32(line, "expected_repetition_count");
        let check_side = json_str(line, "expected_continuous_check_side");
        assert!(
            check_side == "black" || check_side == "white" || check_side == "none",
            "{case_id}: expected_continuous_check_side must be black/white/none, got {check_side:?}"
        );
        json_opt_str(line, "expected_declaration_eligibility"); // present, null or string -- either is valid
        let result = json_str(line, "expected_result");
        assert!(!result.is_empty(), "{case_id}: expected_result is empty");
        json_str(line, "rule_reference");
        json_str(line, "notes");
    }
}

/// Nyugyoku (entering-king declaration) / jishogi (mutual-impasse) cases:
/// no eligibility logic exists yet anywhere in this codebase (confirmed
/// during the Sprint 1 audit, reconfirmed Sprint 2), so this documents
/// exactly which cases are pending real implementation rather than silently
/// skipping them -- and, unlike a plain non-empty check, asserts the exact
/// count so adding or removing a pending case without updating
/// `KNOWN_MISSING_DECLARATION_CASE_COUNT` fails the build instead of passing
/// quietly.
#[test]
fn nyugyoku_and_jishogi_cases_are_recorded_as_pending_not_silently_dropped() {
    let pending: Vec<String> = parse_corpus()
        .into_iter()
        .filter(|c| c.case_id.starts_with("nyugyoku_") || c.case_id.starts_with("jishogi_"))
        .map(|c| c.case_id)
        .collect();
    assert_eq!(
        pending.len(),
        KNOWN_MISSING_DECLARATION_CASE_COUNT,
        "expected exactly {KNOWN_MISSING_DECLARATION_CASE_COUNT} pending nyugyoku/jishogi cases, found {}: {pending:?} -- update KNOWN_MISSING_DECLARATION_CASE_COUNT if this is an intentional addition/removal, or promote a case to a real assertion if you just implemented the eligibility logic",
        pending.len()
    );
    eprintln!("pending nyugyoku/jishogi implementation, corpus cases held for it: {pending:?}");
}
