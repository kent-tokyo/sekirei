use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
#[cfg(test)]
use std::io::BufReader;
use std::io::{self, BufRead, Read};
use std::path::Path;

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::sfen::board_to_sfen;
use serde::Serialize;
use sha2::{Digest, Sha256};
use shogiesa_core::schema::{
    GameOutcome, Observation, PositionRecord, PositionRecordParseError, SideToMove, StabilityInfo,
    parse_json_line,
};

use crate::csa::GameResult;
use crate::dataset_reader::DatasetReader;

/// Typed shogiesa provenance retained beside the authoritative Sekirei board.
///
/// `observations` and `stability` are deliberately metadata here: Sekirei's
/// positions training path continues to use its configured internal teacher
/// search/cache. Selecting an external observation as a teacher requires a
/// separate, explicit provenance and licensing policy; merely appearing in a
/// JSONL row must never silently change the training target.
#[allow(dead_code)]
#[derive(Clone)]
pub struct PositionSample {
    pub board: Board,
    pub phase: String,
    pub side_to_move: String,
    pub ply: u32,
    pub source: String,
    pub source_kind: String,
    pub root_id: Option<String>,
    pub variation_id: Option<String>,
    pub branch_from_ply: Option<u32>,
    pub observations: Vec<Observation>,
    pub stability: Option<StabilityInfo>,
    pub game_result: GameResult,
    pub game_result_source: Option<String>,
}

#[cfg(test)]
#[derive(Default)]
pub struct PositionLoadReport {
    pub samples: Vec<PositionSample>,
    pub non_empty_lines: usize,
    pub skipped_by_reason: BTreeMap<&'static str, usize>,
    pub schema_versions: BTreeMap<u32, usize>,
    pub cp_observations: usize,
    pub mate_observations: usize,
    pub missing_stability: usize,
    pub missing_game_result: usize,
    pub unknown_game_result: usize,
    /// SHA-256 of the exact input bytes, independent of the file path.
    pub sha256: String,
}

#[cfg(test)]
impl PositionLoadReport {
    pub fn print_summary(&self, path: &Path) {
        eprintln!(
            "positions: path={path:?} lines={} accepted={} skipped={} schemas={:?} observations(cp={},mate={}) metadata_missing(stability={},game_result={}) unknown_game_result={}",
            self.non_empty_lines,
            self.samples.len(),
            self.skipped_by_reason.values().sum::<usize>(),
            self.schema_versions,
            self.cp_observations,
            self.mate_observations,
            self.missing_stability,
            self.missing_game_result,
            self.unknown_game_result,
        );
        for (reason, count) in &self.skipped_by_reason {
            eprintln!("  positions skipped: reason={reason} count={count}");
        }
    }
}

fn parse_error_reason(error: &PositionRecordParseError) -> &'static str {
    match error {
        PositionRecordParseError::Json(_) => "invalid_json_or_required_field",
        PositionRecordParseError::UnsupportedSchema(_) => "unsupported_schema_version",
        _ => "unsupported_typed_record",
    }
}

fn map_game_result(outcome: Option<GameOutcome>) -> GameResult {
    match outcome {
        Some(GameOutcome::BlackWins) => GameResult::BlackWin,
        Some(GameOutcome::WhiteWins) => GameResult::WhiteWin,
        Some(GameOutcome::Draw) => GameResult::Draw,
        Some(GameOutcome::Unknown) | None => GameResult::Unknown,
    }
}

pub(crate) fn map_record(record: PositionRecord) -> Result<PositionSample, String> {
    let board =
        Board::from_sfen(&record.sfen).map_err(|error| format!("invalid SFEN ({error})"))?;
    let expected_side = match record.tags.side_to_move {
        SideToMove::Black => Color::Black,
        SideToMove::White => Color::White,
    };
    if board.side_to_move != expected_side {
        return Err(format!(
            "tags.side_to_move={} disagrees with SFEN side {:?}",
            record.tags.side_to_move, board.side_to_move
        ));
    }

    let game_result = map_game_result(record.game_result.as_ref().map(|result| result.outcome));
    let game_result_source = record
        .game_result
        .as_ref()
        .map(|result| result.result_source.clone());

    Ok(PositionSample {
        board,
        phase: record.tags.phase.to_string(),
        side_to_move: record.tags.side_to_move.to_string(),
        ply: record.source.ply,
        source: record.source.path,
        source_kind: record.source.kind,
        root_id: record.source.root_id,
        variation_id: record.source.variation_id,
        branch_from_ply: record.source.branch_from_ply,
        observations: record.observations,
        stability: record.stability,
        game_result,
        game_result_source,
    })
}

/// Maximum retained rejected rows; rejection counts always cover the whole input.
pub const MAX_REJECTED_EXAMPLES: usize = 8;
const REJECTED_PREVIEW_CHARS: usize = 256;

/// Reproduction evidence for one rejected row, without retaining the whole row.
#[derive(Clone, Debug, Serialize)]
pub struct PositionRejectedExample {
    pub line_number: usize,
    /// Zero-based byte offset in the decoded JSONL stream.
    pub byte_offset: u64,
    /// SHA-256 includes the original line ending, when present.
    pub row_sha256: String,
    pub reason: &'static str,
    pub preview: String,
    pub preview_truncated: bool,
}

/// Complete ingestion audit, returned only after EOF and successful callbacks.
///
/// Counts describe parsing and board validation, before any downstream sample
/// caps, split, cache selection, or training filters. Scanner state retains the
/// longest decoded row, one mapped sample, and capped rejection examples;
/// compressed readers additionally allocate their bounded decoder buffers.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PositionIngestReport {
    /// SHA-256 of the physical input file, including its compression framing.
    pub sha256: String,
    pub input_bytes: u64,
    pub compression: &'static str,
    /// Exact decoded JSONL bytes; equal to the input identity for plain files.
    pub decoded_sha256: String,
    pub decoded_bytes: u64,
    /// Physical line breaks in the decoded JSONL, including blank rows.
    pub physical_lines: usize,
    pub non_empty_lines: usize,
    pub accepted: usize,
    pub skipped: usize,
    pub skipped_by_reason: BTreeMap<&'static str, usize>,
    pub schema_versions: BTreeMap<u32, usize>,
    pub cp_observations: usize,
    pub mate_observations: usize,
    pub missing_stability: usize,
    pub missing_game_result: usize,
    pub unknown_game_result: usize,
    pub rejected_examples: Vec<PositionRejectedExample>,
}

impl PositionIngestReport {
    fn record_skip(&mut self, reason: &'static str, byte_offset: u64, raw: &str) {
        self.skipped += 1;
        *self.skipped_by_reason.entry(reason).or_default() += 1;
        if self.rejected_examples.len() < MAX_REJECTED_EXAMPLES {
            let mut chars = raw.chars();
            let preview = chars.by_ref().take(REJECTED_PREVIEW_CHARS).collect();
            self.rejected_examples.push(PositionRejectedExample {
                line_number: self.physical_lines,
                byte_offset,
                row_sha256: format!("{:x}", Sha256::digest(raw.as_bytes())),
                reason,
                preview,
                preview_truncated: chars.next().is_some(),
            });
        }
    }
}

/// Read, validate and deliver each accepted position without collecting it.
///
/// The callback receives the exact decoded UTF-8 row, including any line
/// ending. It may write the row to a spool; the borrow ends before the reusable
/// buffer advances. Strict mode stops at the first invalid record. Non-strict
/// mode records bounded evidence and continues. Read/UTF-8 errors and callback
/// failures always stop the scan and never produce a complete audit report.
pub fn scan_positions(
    path: &Path,
    strict: bool,
    accept: impl FnMut(PositionSample, &str) -> Result<(), String>,
) -> Result<PositionIngestReport, String> {
    let file = File::open(path).map_err(|error| format!("cannot read {path:?}: {error}"))?;
    let mut source = InputDigest {
        reader: file,
        digest: Sha256::new(),
        bytes: 0,
    };
    let reader = DatasetReader::new(&mut source, path)
        .map_err(|error| format!("cannot decode positions {path:?}: {error}"))?;
    let compression = reader.compression();
    // Returning successfully requires the decoder to reach physical EOF and
    // verify every member/frame footer. Hash the file below buffering/decoding,
    // without reopening a possibly changed input after the scan.
    let mut report = scan_reader(reader, strict, accept)?;
    report.compression = compression;
    report.sha256 = format!("{:x}", source.digest.finalize());
    report.input_bytes = source.bytes;
    Ok(report)
}

struct InputDigest<R> {
    reader: R,
    digest: Sha256,
    bytes: u64,
}

impl<R: Read> Read for InputDigest<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read(output)?;
        self.bytes = self
            .bytes
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("positions input byte count overflow"))?;
        self.digest.update(&output[..count]);
        Ok(count)
    }
}

fn scan_reader(
    mut reader: impl BufRead,
    strict: bool,
    mut accept: impl FnMut(PositionSample, &str) -> Result<(), String>,
) -> Result<PositionIngestReport, String> {
    let mut report = PositionIngestReport {
        compression: "plain",
        ..PositionIngestReport::default()
    };
    let mut digest = Sha256::new();
    let mut raw = String::new();
    loop {
        raw.clear();
        let line_number = report.physical_lines.saturating_add(1);
        let bytes_read = reader.read_line(&mut raw).map_err(|error| {
            format!("positions line {line_number}: cannot read JSONL row: {error}")
        })?;
        if bytes_read == 0 {
            break;
        }
        let byte_offset = report.decoded_bytes;
        report.decoded_bytes = report
            .decoded_bytes
            .checked_add(bytes_read as u64)
            .ok_or_else(|| "positions input byte count overflow".to_string())?;
        report.physical_lines = line_number;
        digest.update(raw.as_bytes());
        if raw.trim().is_empty() {
            continue;
        }
        report.non_empty_lines += 1;
        let record = match parse_json_line(&raw) {
            Ok(record) => record,
            Err(error) => {
                if strict {
                    return Err(format!("positions line {line_number}: {error}"));
                }
                report.record_skip(parse_error_reason(&error), byte_offset, &raw);
                continue;
            }
        };
        let schema_version = record.schema_version;
        let cp_count = record
            .observations
            .iter()
            .filter(|observation| {
                matches!(observation.score, shogiesa_core::schema::Score::Cp { .. })
            })
            .count();
        let mate_count = record.observations.len() - cp_count;
        let missing_stability = record.stability.is_none();
        let missing_game_result = record.game_result.is_none();
        let unknown_game_result = record
            .game_result
            .as_ref()
            .is_some_and(|result| result.outcome == GameOutcome::Unknown);

        match map_record(record) {
            Ok(sample) => {
                accept(sample, &raw).map_err(|error| {
                    format!("positions line {line_number}: accepted-row callback failed: {error}")
                })?;
                report.accepted += 1;
                *report.schema_versions.entry(schema_version).or_default() += 1;
                report.cp_observations += cp_count;
                report.mate_observations += mate_count;
                report.missing_stability += usize::from(missing_stability);
                report.missing_game_result += usize::from(missing_game_result);
                report.unknown_game_result += usize::from(unknown_game_result);
            }
            Err(error) => {
                if strict {
                    return Err(format!("positions line {line_number}: {error}"));
                }
                let reason = if error.starts_with("invalid SFEN") {
                    "invalid_sfen"
                } else {
                    "side_to_move_mismatch"
                };
                report.record_skip(reason, byte_offset, &raw);
            }
        }
    }
    report.decoded_sha256 = format!("{:x}", digest.finalize());
    report.sha256.clone_from(&report.decoded_sha256);
    report.input_bytes = report.decoded_bytes;
    Ok(report)
}

/// Collect the streaming scan for existing callers that require random access.
///
/// Unlike `scan_positions`, this compatibility wrapper retains all accepted
/// samples. Its validation, rejection categories and digest use the same scan.
#[cfg(test)]
pub fn load_positions(path: &Path, strict: bool) -> Result<PositionLoadReport, String> {
    let mut samples = Vec::new();
    let ingest = scan_positions(path, strict, |sample, _raw| {
        samples.push(sample);
        Ok(())
    })?;
    let report = PositionLoadReport {
        samples,
        non_empty_lines: ingest.non_empty_lines,
        skipped_by_reason: ingest.skipped_by_reason,
        schema_versions: ingest.schema_versions,
        cp_observations: ingest.cp_observations,
        mate_observations: ingest.mate_observations,
        missing_stability: ingest.missing_stability,
        missing_game_result: ingest.missing_game_result,
        unknown_game_result: ingest.unknown_game_result,
        sha256: ingest.sha256,
    };
    report.print_summary(path);
    Ok(report)
}

/// Validate every non-empty JSONL position without silently skipping rows.
#[cfg(test)]
pub fn validate_positions(path: &Path) -> Result<usize, String> {
    load_positions(path, true).map(|report| report.samples.len())
}

/// Apply a per-source sample cap using deterministic hash ordering.
/// Selects samples via `sfen_hash(source + "\0" + sfen, seed)` — order-independent.
pub fn apply_source_cap(
    samples: Vec<PositionSample>,
    cap: usize,
    seed: u64,
) -> Vec<PositionSample> {
    if cap == 0 {
        return samples;
    }

    let mut by_source: HashMap<&str, Vec<(u64, usize)>> = HashMap::new();
    for (index, sample) in samples.iter().enumerate() {
        let sfen = board_to_sfen(&sample.board);
        let key = format!("{}\0{}", sample.source, sfen);
        let hash = sfen_hash(&key, seed);
        by_source
            .entry(&sample.source)
            .or_default()
            .push((hash, index));
    }

    let mut keep = HashSet::new();
    for group in by_source.values_mut() {
        group.sort_unstable();
        for &(_, index) in group.iter().take(cap) {
            keep.insert(index);
        }
    }

    samples
        .into_iter()
        .enumerate()
        .filter(|(index, _)| keep.contains(index))
        .map(|(_, sample)| sample)
        .collect()
}

/// Retain only samples whose canonical SFEN is present in a teacher cache.
#[cfg(test)]
pub fn retain_cached_samples(
    samples: Vec<PositionSample>,
    cache: &HashMap<String, i32>,
) -> (Vec<PositionSample>, usize) {
    let total = samples.len();
    let kept = samples
        .into_iter()
        .filter(|sample| cache.contains_key(&board_to_sfen(&sample.board)))
        .collect();
    (kept, total)
}

/// FNV-1a hash XORed with seed — used for deterministic split and source cap.
pub fn sfen_hash(sfen: &str, seed: u64) -> u64 {
    let mut hash = 14695981039346656037u64;
    for byte in sfen.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash ^ seed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use tempfile::NamedTempFile;

    const STARTPOS_SFEN: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
    const SFEN_2: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2";

    fn record(sfen: &str, phase: &str, side: &str, ply: u32, source: &str) -> String {
        format!(
            r#"{{"schema_version":11,"sfen":"{sfen}","source":{{"kind":"csa","path":"{source}","ply":{ply}}},"tags":{{"phase":"{phase}","side_to_move":"{side}","in_check":false,"has_capture":false}},"observations":[]}}"#
        )
    }

    fn make_jsonl(records: &[(&str, &str, &str, u32, &str)]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        for &(sfen, phase, side, ply, source) in records {
            writeln!(file, "{}", record(sfen, phase, side, ply, source)).unwrap();
        }
        file
    }

    fn samples(path: &Path) -> Vec<PositionSample> {
        load_positions(path, true).unwrap().samples
    }

    #[test]
    fn load_positions_basic() {
        let file = make_jsonl(&[(STARTPOS_SFEN, "opening", "black", 1, "game1.csa")]);
        let loaded = load_positions(file.path(), true).unwrap();
        assert_eq!(loaded.samples.len(), 1);
        assert_eq!(loaded.samples[0].phase, "opening");
        assert_eq!(loaded.samples[0].side_to_move, "black");
        assert_eq!(loaded.samples[0].ply, 1);
        assert_eq!(loaded.samples[0].source, "game1.csa");
        assert_eq!(loaded.schema_versions.get(&11), Some(&1));
        assert_eq!(
            loaded.sha256,
            format!("{:x}", Sha256::digest(std::fs::read(file.path()).unwrap()))
        );
    }

    #[test]
    fn content_identity_ignores_path_but_detects_same_size_mutation() {
        let first = make_jsonl(&[(STARTPOS_SFEN, "opening", "black", 1, "game1.csa")]);
        let second = NamedTempFile::new().unwrap();
        std::fs::copy(first.path(), second.path()).unwrap();
        let a = load_positions(first.path(), true).unwrap().sha256;
        let b = load_positions(second.path(), true).unwrap().sha256;
        assert_eq!(a, b);

        let mut bytes = std::fs::read(second.path()).unwrap();
        let index = bytes.iter().position(|byte| *byte == b'1').unwrap();
        bytes[index] = b'2';
        std::fs::write(second.path(), bytes).unwrap();
        let changed = load_positions(second.path(), false).unwrap().sha256;
        assert_ne!(a, changed);
    }

    #[test]
    fn scan_preserves_raw_rows_offsets_and_complete_input_digest() {
        let first = format!(
            "{}\r\n",
            record(STARTPOS_SFEN, "opening", "black", 1, "棋譜.csa")
        );
        let last = record(SFEN_2, "opening", "white", 2, "b.csa");
        let prefix = format!("\r\n{first} \t\n");
        let rejected = "not json\r\n";
        let input = format!("{prefix}{rejected}{last}");
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(input.as_bytes()).unwrap();
        let mut rows = Vec::new();
        let report = scan_positions(file.path(), false, |sample, raw| {
            rows.push((sample.source, raw.to_string()));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            rows,
            vec![("棋譜.csa".to_string(), first), ("b.csa".to_string(), last)]
        );
        assert_eq!(report.input_bytes, input.len() as u64);
        assert_eq!(report.compression, "plain");
        assert_eq!(report.decoded_bytes, report.input_bytes);
        assert_eq!(report.decoded_sha256, report.sha256);
        assert_eq!(
            report.sha256,
            format!("{:x}", Sha256::digest(input.as_bytes()))
        );
        assert_eq!(report.physical_lines, 5);
        assert_eq!(report.non_empty_lines, 3);
        assert_eq!(report.accepted, 2);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.schema_versions.get(&11), Some(&2));
        assert_eq!(report.missing_stability, 2);
        assert_eq!(report.missing_game_result, 2);
        let example = &report.rejected_examples[0];
        assert_eq!(example.line_number, 4);
        assert_eq!(example.byte_offset, prefix.len() as u64);
        assert_eq!(
            example.row_sha256,
            format!("{:x}", Sha256::digest(rejected.as_bytes()))
        );
        assert_eq!(example.reason, "invalid_json_or_required_field");
        assert_eq!(example.preview, rejected);
        assert!(!example.preview_truncated);
        let serialized = serde_json::to_value(&report).unwrap();
        assert_eq!(serialized["accepted"], 2);
        assert_eq!(
            serialized["rejected_examples"][0]["byte_offset"],
            prefix.len() as u64
        );
    }

    #[test]
    fn scan_classifies_rejections_and_continues_to_later_valid_rows() {
        let valid = record(STARTPOS_SFEN, "opening", "black", 1, "a");
        let rows = [
            "not json".to_string(),
            valid.replace("\"schema_version\":11", "\"schema_version\":12"),
            record("invalid", "opening", "black", 1, "a"),
            record(STARTPOS_SFEN, "opening", "white", 1, "a"),
            valid,
        ];
        let input = rows.join("\n");
        let mut accepted = 0;
        let report = scan_reader(std::io::Cursor::new(&input), false, |_, _| {
            accepted += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(accepted, 1);
        assert_eq!(report.accepted, 1);
        assert_eq!(report.non_empty_lines, 5);
        assert_eq!(report.skipped, 4);
        let reasons = [
            "invalid_json_or_required_field",
            "unsupported_schema_version",
            "invalid_sfen",
            "side_to_move_mismatch",
        ];
        for (index, reason) in reasons.iter().enumerate() {
            assert_eq!(report.skipped_by_reason.get(reason), Some(&1));
            assert_eq!(report.rejected_examples[index].reason, *reason);
            assert_eq!(report.rejected_examples[index].line_number, index + 1);
        }
    }

    #[test]
    fn rejection_examples_and_unicode_previews_are_bounded() {
        let row = format!("{}\n", "不正".repeat(300));
        let input = row.repeat(20);
        let report = scan_reader(std::io::Cursor::new(&input), false, |_, _| {
            panic!("invalid rows must not reach the callback")
        })
        .unwrap();
        assert_eq!(report.accepted, 0);
        assert_eq!(report.skipped, 20);
        assert_eq!(
            report.skipped_by_reason["invalid_json_or_required_field"],
            20
        );
        assert_eq!(report.rejected_examples.len(), MAX_REJECTED_EXAMPLES);
        for (index, example) in report.rejected_examples.iter().enumerate() {
            assert_eq!(example.line_number, index + 1);
            assert_eq!(example.byte_offset, (index * row.len()) as u64);
            assert_eq!(example.preview.chars().count(), REJECTED_PREVIEW_CHARS);
            assert!(example.preview_truncated);
            assert_eq!(
                example.row_sha256,
                format!("{:x}", Sha256::digest(row.as_bytes()))
            );
        }
        assert_eq!(
            report.sha256,
            format!("{:x}", Sha256::digest(input.as_bytes()))
        );
    }

    #[test]
    fn callback_failure_stops_without_a_complete_report() {
        let row = record(STARTPOS_SFEN, "opening", "black", 1, "a");
        let input = format!("\n{row}\n{row}\n{row}\n");
        for strict in [false, true] {
            let mut accepted = 0;
            let error = scan_reader(std::io::Cursor::new(&input), strict, |_, _| {
                accepted += 1;
                if accepted == 2 {
                    Err("spool full".to_string())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(accepted, 2);
            assert!(error.contains("positions line 3"));
            assert!(error.contains("accepted-row callback failed: spool full"));
        }
    }

    #[test]
    fn invalid_utf8_is_a_fatal_read_error_even_in_non_strict_mode() {
        let row = record(STARTPOS_SFEN, "opening", "black", 1, "a");
        let mut input = format!("{row}\n").into_bytes();
        input.extend_from_slice(b"\xff\n");
        input.extend_from_slice(row.as_bytes());
        for strict in [false, true] {
            let mut accepted = 0;
            let error = scan_reader(std::io::Cursor::new(&input), strict, |_, _| {
                accepted += 1;
                Ok(())
            })
            .unwrap_err();
            assert_eq!(accepted, 1);
            assert!(error.contains("positions line 2: cannot read JSONL row"));
        }
    }

    #[test]
    fn io_failure_is_fatal_after_preceding_complete_rows() {
        struct FailingReader;
        impl Read for FailingReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("read interrupted"))
            }
        }
        let row = format!("{}\n", record(STARTPOS_SFEN, "opening", "black", 1, "a"));
        for strict in [false, true] {
            let reader = BufReader::new(std::io::Cursor::new(&row).chain(FailingReader));
            let mut accepted = 0;
            let error = scan_reader(reader, strict, |_, _| {
                accepted += 1;
                Ok(())
            })
            .unwrap_err();
            assert_eq!(accepted, 1);
            assert!(error.contains("positions line 2: cannot read JSONL row: read interrupted"));
        }
    }

    #[test]
    fn blank_rows_contribute_to_audit_but_not_sample_counts() {
        for input in ["", "\n \t\r\n\t"] {
            let report = scan_reader(std::io::Cursor::new(input), true, |_, _| {
                panic!("blank rows must not reach the callback")
            })
            .unwrap();
            assert_eq!(report.physical_lines, if input.is_empty() { 0 } else { 3 });
            assert_eq!(report.input_bytes, input.len() as u64);
            assert_eq!(report.non_empty_lines, 0);
            assert_eq!(report.accepted, 0);
            assert_eq!(report.skipped, 0);
            assert!(report.rejected_examples.is_empty());
            assert_eq!(
                report.sha256,
                format!("{:x}", Sha256::digest(input.as_bytes()))
            );
        }
    }

    #[test]
    fn canonical_shogiesa_v11_fixture_maps_all_contract_fields() {
        let fixture = include_str!("../tests/fixtures/shogiesa_schema_contract_v11.jsonl");
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{fixture}").unwrap();
        let loaded = load_positions(file.path(), true).unwrap();

        assert_eq!(loaded.samples.len(), 2);
        let cp = &loaded.samples[0];
        assert_eq!(cp.source_kind, "kif");
        assert_eq!(cp.root_id.as_deref(), Some("games/sample.kif"));
        assert_eq!(cp.variation_id.as_deref(), Some("var1"));
        assert_eq!(cp.branch_from_ply, Some(20));
        assert_eq!(cp.game_result, GameResult::BlackWin);
        assert_eq!(cp.game_result_source.as_deref(), Some("kif_marker"));
        assert!(cp.stability.is_some());
        assert!(matches!(
            cp.observations[0].score,
            shogiesa_core::schema::Score::Cp { value: 43 }
        ));
        assert_eq!(
            cp.observations[0].score_perspective,
            shogiesa_core::schema::ScorePerspective::SideToMove
        );
        assert_eq!(
            cp.observations[0].score_bound,
            shogiesa_core::schema::ScoreBound::Exact
        );
        assert_eq!(
            cp.observations[0].weight_sha256.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
        let stability = cp.stability.as_ref().unwrap();
        assert!(stability.bestmove_agreement);
        assert_eq!(stability.score_swing_cp, Some(0));
        assert!(matches!(
            loaded.samples[1].observations[0].score,
            shogiesa_core::schema::Score::Mate { moves: 5 }
        ));
        assert_eq!(loaded.samples[1].game_result, GameResult::Draw);
        assert_eq!(loaded.cp_observations, 1);
        assert_eq!(loaded.mate_observations, 1);
    }

    #[test]
    fn nnue_phase3_pilot_fixture_has_no_invalid_sfen() {
        let fixture = include_str!("../../../scripts/fixtures/nnue_phase3_pilot.jsonl");
        let mut count = 0;
        for (line, raw) in fixture.lines().enumerate() {
            let value: serde_json::Value = serde_json::from_str(raw).unwrap();
            let sfen = value["sfen"].as_str().unwrap();
            Board::from_sfen(sfen).unwrap_or_else(|error| {
                panic!("pilot fixture line {} has invalid SFEN: {error}", line + 1)
            });
            count += 1;
        }
        assert_eq!(count, 6);
    }

    #[test]
    fn strict_position_validation_reports_invalid_row_line() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            record(STARTPOS_SFEN, "opening", "black", 1, "a")
        )
        .unwrap();
        writeln!(file, "{}", record("invalid", "opening", "black", 2, "a")).unwrap();
        let error = validate_positions(file.path()).unwrap_err();
        assert!(error.contains("line 2"));
        assert!(error.contains("invalid SFEN"));
    }

    #[test]
    fn streaming_reader_preserves_physical_line_numbers_across_blanks() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file).unwrap();
        writeln!(
            file,
            "{}",
            record(STARTPOS_SFEN, "opening", "black", 1, "a")
        )
        .unwrap();
        writeln!(file, "not json").unwrap();

        let error = load_positions(file.path(), true).err().unwrap();
        assert!(error.contains("line 3"));
    }

    #[test]
    fn non_strict_load_counts_each_rejection_reason() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "not json").unwrap();
        writeln!(file, "{}", record("invalid", "opening", "black", 1, "a")).unwrap();
        writeln!(
            file,
            "{}",
            record(STARTPOS_SFEN, "opening", "white", 1, "a")
        )
        .unwrap();
        let report = load_positions(file.path(), false).unwrap();
        assert!(report.samples.is_empty());
        assert_eq!(
            report
                .skipped_by_reason
                .get("invalid_json_or_required_field"),
            Some(&1)
        );
        assert_eq!(report.skipped_by_reason.get("invalid_sfen"), Some(&1));
        assert_eq!(
            report.skipped_by_reason.get("side_to_move_mismatch"),
            Some(&1)
        );
    }

    #[test]
    fn unsupported_schema_is_never_treated_as_current() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            record(STARTPOS_SFEN, "opening", "black", 1, "a")
                .replace("\"schema_version\":11", "\"schema_version\":12")
        )
        .unwrap();
        let error = load_positions(file.path(), true).err().unwrap();
        assert!(error.contains("line 1"));
        assert!(error.contains("unsupported shogiesa schema_version 12"));
        let report = load_positions(file.path(), false).unwrap();
        assert_eq!(
            report.skipped_by_reason.get("unsupported_schema_version"),
            Some(&1)
        );
    }

    #[test]
    fn wdl_mapping_covers_wins_draw_unknown_and_missing() {
        let outcomes = [
            (Some(GameOutcome::BlackWins), GameResult::BlackWin),
            (Some(GameOutcome::WhiteWins), GameResult::WhiteWin),
            (Some(GameOutcome::Draw), GameResult::Draw),
            (Some(GameOutcome::Unknown), GameResult::Unknown),
            (None, GameResult::Unknown),
        ];
        for (input, expected) in outcomes {
            assert_eq!(map_game_result(input), expected);
        }
    }

    #[test]
    fn source_cap_limits_per_source_and_is_order_independent() {
        let file = make_jsonl(&[
            (STARTPOS_SFEN, "opening", "black", 1, "game1.csa"),
            (SFEN_2, "opening", "white", 2, "game1.csa"),
        ]);
        let all = samples(file.path());
        assert_eq!(apply_source_cap(all.clone(), 1, 42).len(), 1);
        assert_eq!(apply_source_cap(all.clone(), 0, 42).len(), 2);

        let forward: HashSet<String> = apply_source_cap(all.clone(), 1, 42)
            .iter()
            .map(|sample| board_to_sfen(&sample.board))
            .collect();
        let mut reversed = all;
        reversed.reverse();
        let backward: HashSet<String> = apply_source_cap(reversed, 1, 42)
            .iter()
            .map(|sample| board_to_sfen(&sample.board))
            .collect();
        assert_eq!(forward, backward);
    }

    #[test]
    fn validation_split_is_deterministic() {
        assert_eq!(sfen_hash(STARTPOS_SFEN, 42), sfen_hash(STARTPOS_SFEN, 42));
        assert_ne!(sfen_hash(STARTPOS_SFEN, 42), sfen_hash(STARTPOS_SFEN, 99));
        assert_ne!(sfen_hash(STARTPOS_SFEN, 42), sfen_hash(SFEN_2, 42));
    }

    #[test]
    fn missing_required_fields_are_not_guessed() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"schema_version":11,"sfen":"{STARTPOS_SFEN}"}}"#).unwrap();
        let error = load_positions(file.path(), true).err().unwrap();
        assert!(error.contains("line 1"));
        assert!(error.contains("missing field"));
    }

    #[test]
    fn oversized_source_ply_is_rejected_instead_of_saturated() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            record(STARTPOS_SFEN, "opening", "black", 1, "a")
                .replace("\"ply\":1", "\"ply\":4294967296")
        )
        .unwrap();
        let error = load_positions(file.path(), true).err().unwrap();
        assert!(error.contains("line 1"));
        assert!(error.contains("invalid value"));
    }

    #[test]
    fn retain_cached_samples_filters_by_canonical_sfen() {
        let file = make_jsonl(&[
            (STARTPOS_SFEN, "opening", "black", 1, "a"),
            (SFEN_2, "opening", "white", 2, "a"),
        ]);
        let loaded = samples(file.path());
        let mut cache = HashMap::new();
        cache.insert(board_to_sfen(&loaded[0].board), 12);
        let (kept, total) = retain_cached_samples(loaded, &cache);
        assert_eq!(total, 2);
        assert_eq!(kept.len(), 1);
        assert_eq!(board_to_sfen(&kept[0].board), STARTPOS_SFEN);
    }
}
