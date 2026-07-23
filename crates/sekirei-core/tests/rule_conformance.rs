//! Rule-conformance golden corpus harness (Sprint 1 / Phase B3 foundation).
//!
//! Reads `tests/fixtures/rule_conformance_cases.jsonl` (regenerate the
//! engine-verifiable cases with `cargo run --release --example
//! gen_rule_conformance_corpus`) and checks each case's *recorded* facts
//! against what the engine's own primitives (`generate_legal_moves`,
//! `is_in_check`) actually observe when the moves are replayed -- not
//! against sekirei-match-runner's rule *decisions*, which this corpus
//! exists to eventually gate (see the module-level notes below on why).
//!
//! This is a foundation, not exhaustive coverage: a handful of cases per
//! rule category, not a full sweep. Declaration-win (nyugyoku) and
//! mutual-impasse (jishogi) cases are recorded but not verifiable yet --
//! no eligibility-counting logic exists anywhere in this codebase (confirmed
//! during the Sprint 1 provenance/USI audit). Those cases assert only that
//! the corpus's SFEN parses; they exist so a future implementation has
//! fixtures to gate against, and to document the exact rule-source citation
//! at record time (not what a future implementation happens to compute).
//!
//! **A known, confirmed gap this corpus documents rather than hides**:
//! `crates/sekirei-match-runner/src/main.rs`'s repetition handling
//! (`EndReason::Repetition`) always resolves a 4-fold hash repeat to
//! `Outcome::Draw` -- it has no continuous-check (連続王手) special case at
//! all. The `continuous_check_sennichite_black_checks` case below is
//! engine-verified to actually BE a continuous-check repetition (every
//! black move gives check, the position recurs 4 times) -- but this test
//! does not call into match-runner's outcome logic, so it does not (yet)
//! fail to reflect that gap. Closing the gap is separate implementation
//! work; this harness's job is to hold the fixture that work will be
//! graded against.

use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::sfen::{move_from_usi, parse_position_cmd};

const CORPUS: &str = include_str!("fixtures/rule_conformance_cases.jsonl");

struct Case {
    case_id: String,
    initial_sfen: String,
    move_history: Vec<String>,
    repetition_count: u32,
    continuous_check_side: String,
    expected_result: String,
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

fn parse_corpus() -> Vec<Case> {
    CORPUS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| Case {
            case_id: json_str(line, "case_id"),
            initial_sfen: json_str(line, "initial_sfen"),
            move_history: json_str_array(line, "move_history"),
            repetition_count: json_u32(line, "repetition_count"),
            continuous_check_side: json_str(line, "continuous_check_side"),
            expected_result: json_str(line, "expected_result"),
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
            occurrences, case.repetition_count,
            "{}: recorded repetition_count doesn't match what replaying move_history actually produces",
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
        if case.continuous_check_side == "none" || case.move_history.is_empty() {
            continue;
        }
        let checker_is_black = case.continuous_check_side == "black";
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

/// Declaration-win/mutual-impasse cases: no eligibility logic exists yet
/// anywhere in this codebase (confirmed during the Sprint 1 audit), so this
/// only documents which cases are pending real implementation rather than
/// silently skipping them.
#[test]
fn declaration_cases_are_recorded_as_pending_not_silently_dropped() {
    let pending: Vec<String> = parse_corpus()
        .into_iter()
        .filter(|c| {
            c.expected_result.contains("declaration") || c.expected_result.contains("ambiguous")
        })
        .map(|c| c.case_id)
        .collect();
    assert!(
        !pending.is_empty(),
        "expected at least one declaration-rule placeholder case in the corpus"
    );
    eprintln!("pending declaration-rule implementation, corpus cases held for it: {pending:?}");
}
