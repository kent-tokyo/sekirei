use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Load teacher scores from a JSONL cache file, keeping only entries whose
/// recorded `label_depth` and `teacher_identity` match the requested teacher.
/// A different depth or fixed evaluator is a different signal; mixing either
/// into one run would make cache hits silently change the objective.
/// Legacy cache lines without a teacher identity are treated as `material`.
/// Each native line includes `sfen`, `label_depth`, `teacher_identity`, and
/// `score_cp`.
pub fn load(path: &Path, expected_depth: u32, expected_teacher: &str) -> HashMap<String, i32> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("teacher cache: cannot read {:?}: {e}", path);
            return HashMap::new();
        }
    };
    let mut map = HashMap::new();
    let mut skipped = 0usize;
    let mut depth_mismatch = 0usize;
    let mut teacher_mismatch = 0usize;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            skipped += 1;
            continue;
        };
        let Some(sfen) = val.get("sfen").and_then(|v| v.as_str()) else {
            skipped += 1;
            continue;
        };
        // Accept the native cache shape, plus the existing analysis_record_v1
        // shape produced by Gate 2.  The latter is only a compatibility
        // bridge: its settings.depth and first PV score must still match the
        // requested teacher depth before it can be reused.
        let (cp, recorded_depth) = if let Some(cp) = val.get("score_cp").and_then(|v| v.as_i64()) {
            (cp, val.get("label_depth").and_then(|v| v.as_u64()))
        } else {
            let Some(depth) = val
                .get("settings")
                .and_then(|settings| settings.get("depth"))
                .and_then(|v| v.as_u64())
            else {
                skipped += 1;
                continue;
            };
            let Some(cp) = val
                .get("lines")
                .and_then(|lines| lines.as_array())
                .and_then(|lines| lines.first())
                .and_then(|line| line.get("score_cp"))
                .and_then(|v| v.as_i64())
            else {
                skipped += 1;
                continue;
            };
            (cp, Some(depth))
        };
        let Ok(cp) = i32::try_from(cp) else {
            skipped += 1;
            continue;
        };
        match recorded_depth {
            Some(d) if d as u32 == expected_depth => {}
            Some(_) => {
                depth_mismatch += 1;
                continue;
            }
            None => {
                skipped += 1;
                continue;
            }
        }
        let recorded_teacher = val
            .get("teacher_identity")
            .and_then(|v| v.as_str())
            .unwrap_or("material");
        if recorded_teacher != expected_teacher {
            teacher_mismatch += 1;
            continue;
        }
        // Last occurrence wins on a duplicate key -- deterministic given
        // JSONL is read top-to-bottom, and matches `write`'s own contract
        // (it always writes the current in-memory value, so a re-written
        // file's later lines reflect the most recent search).
        map.insert(sfen.to_string(), cp);
    }
    if skipped > 0 {
        eprintln!("teacher cache: {skipped} lines skipped (unparseable)");
    }
    if depth_mismatch > 0 {
        eprintln!(
            "teacher cache: {depth_mismatch} entries skipped (label_depth != {expected_depth})"
        );
    }
    if teacher_mismatch > 0 {
        eprintln!(
            "teacher cache: {teacher_mismatch} entries skipped (teacher_identity != {expected_teacher})"
        );
    }
    eprintln!(
        "teacher cache: {} entries loaded from {:?}",
        map.len(),
        path
    );
    map
}

/// Write teacher cache to a JSONL file, atomically: the full content is
/// written to a sibling `.tmp` file first, then renamed into place. A
/// crash or kill mid-write leaves the original file untouched (the
/// half-written `.tmp` is simply orphaned) rather than leaving `path`
/// truncated -- `fs::File::create` + direct in-place write would instead
/// truncate `path` immediately, so an interruption could lose every
/// previously-cached entry, not just fail to add new ones.
/// Entries are written in sorted SFEN order so identical maps produce
/// byte-identical artifacts across processes.
/// `entries`: sfen → score_cp mapping; depth and teacher identity are recorded
/// per line so a cache cannot be reused under another labeling contract.
pub fn write(
    path: &Path,
    entries: &HashMap<String, i32>,
    label_depth: u32,
    teacher_identity: &str,
) -> std::io::Result<()> {
    let tmp_path = path.with_extension(format!(
        "jsonl.tmp-{}-{}",
        std::process::id(),
        WRITE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let write_result = (|| -> std::io::Result<()> {
        let f = fs::File::create(&tmp_path)?;
        let mut w = BufWriter::new(f);
        let mut sfens: Vec<&String> = entries.keys().collect();
        sfens.sort_unstable();
        for sfen in sfens {
            let cp = entries[sfen];
            writeln!(
                w,
                r#"{{"sfen":{},"label_depth":{},"teacher_identity":{},"score_cp":{}}}"#,
                json_string(sfen),
                label_depth,
                json_string(teacher_identity),
                cp
            )?;
        }
        w.flush()?;
        let file = w.into_inner().map_err(|error| error.into_error())?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    Ok(())
}

fn json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    const SFEN_A: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
    const SFEN_B: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2";

    #[test]
    fn roundtrip() {
        let f = NamedTempFile::new().unwrap();
        let mut expected = HashMap::new();
        expected.insert(SFEN_A.to_string(), 48i32);
        expected.insert(SFEN_B.to_string(), -120i32);
        write(f.path(), &expected, 4, "material").unwrap();
        let loaded = load(f.path(), 4, "material");
        assert_eq!(loaded, expected);
    }

    #[test]
    fn broken_lines_skipped() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "not json").unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":4,"score_cp":100}}"#).unwrap();
        let loaded = load(f.path(), 4, "material");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[SFEN_A], 100);
    }

    #[test]
    fn analysis_record_v1_can_seed_a_matching_depth_cache() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"sfen":"{SFEN_A}","settings":{{"depth":4}},"lines":[{{"score_cp":-120}}]}}"#
        )
        .unwrap();
        let loaded = load(f.path(), 4, "material");
        assert_eq!(loaded.get(SFEN_A), Some(&-120));
    }

    #[test]
    fn analysis_record_v1_wrong_depth_is_not_reused() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"sfen":"{SFEN_A}","settings":{{"depth":2}},"lines":[{{"score_cp":-120}}]}}"#
        )
        .unwrap();
        assert!(load(f.path(), 4, "material").is_empty());
    }

    #[test]
    fn missing_score_cp_skipped() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":4}}"#).unwrap();
        let loaded = load(f.path(), 4, "material");
        assert!(loaded.is_empty());
    }

    #[test]
    fn out_of_range_score_cp_is_skipped() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"sfen":"{SFEN_A}","label_depth":4,"score_cp":2147483648}}"#
        )
        .unwrap();
        let loaded = load(f.path(), 4, "material");
        assert!(loaded.is_empty());
    }

    #[test]
    fn truncated_trailing_line_is_skipped_not_fatal() {
        // Simulates a write interrupted mid-line (e.g. a kill during a
        // pre-atomic-write version's direct write): the last line has no
        // trailing newline and is invalid JSON. Earlier, complete lines
        // must still load.
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":4,"score_cp":100}}"#).unwrap();
        write!(f, r#"{{"sfen":"{SFEN_B}","label_depth":4,"sco"#).unwrap(); // cut off, no newline
        let loaded = load(f.path(), 4, "material");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[SFEN_A], 100);
    }

    #[test]
    fn wrong_depth_entries_are_filtered_out_and_reported() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":1,"score_cp":999}}"#).unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_B}","label_depth":4,"score_cp":100}}"#).unwrap();
        let loaded = load(f.path(), 4, "material");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[SFEN_B], 100);
        assert!(
            !loaded.contains_key(SFEN_A),
            "depth-1 entry must not be usable as a depth-4 cache hit"
        );
    }

    #[test]
    fn duplicate_key_resolves_to_last_occurrence_in_file() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":4,"score_cp":100}}"#).unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":4,"score_cp":250}}"#).unwrap();
        let loaded = load(f.path(), 4, "material");
        assert_eq!(loaded[SFEN_A], 250);
    }

    #[test]
    fn write_is_atomic_no_tmp_file_left_behind_on_success() {
        let f = NamedTempFile::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert(SFEN_A.to_string(), 48i32);
        write(f.path(), &entries, 4, "material").unwrap();
        let tmp_path = f.path().with_extension("jsonl.tmp");
        assert!(
            !tmp_path.exists(),
            "the intermediate .tmp file must be renamed away, not left behind"
        );
        assert_eq!(load(f.path(), 4, "material"), entries);
    }

    #[test]
    fn write_is_deterministic_for_identical_maps() {
        let first = NamedTempFile::new().unwrap();
        let second = NamedTempFile::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert(SFEN_A.to_string(), 48i32);
        entries.insert(SFEN_B.to_string(), -120i32);

        write(first.path(), &entries, 4, "material").unwrap();
        write(second.path(), &entries, 4, "material").unwrap();

        assert_eq!(
            fs::read(first.path()).unwrap(),
            fs::read(second.path()).unwrap(),
            "identical cache maps must produce identical artifacts"
        );
    }

    #[test]
    fn fixed_nnue_cache_is_bound_to_exact_teacher_identity() {
        let f = NamedTempFile::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert(SFEN_A.to_string(), 48i32);
        write(f.path(), &entries, 4, "nnue:0123456789abcdef").unwrap();

        assert_eq!(load(f.path(), 4, "nnue:0123456789abcdef"), entries);
        assert!(load(f.path(), 4, "nnue:fedcba9876543210").is_empty());
        assert!(load(f.path(), 4, "material").is_empty());
    }
}
