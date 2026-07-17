use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Load teacher scores from a JSONL cache file, keeping only entries whose
/// recorded `label_depth` matches `expected_depth`. A search at a different
/// depth is a different teacher signal -- without this filter, reusing a
/// cache file across depths would silently blend in wrong-depth scores as
/// if they were cache hits at the requested depth. One file is expected to
/// hold one depth (see the `_depth4` naming convention); a mismatch here
/// means the wrong cache file was pointed at, so it's reported loudly.
/// Each line: `{"sfen":"...","label_depth":N,"score_cp":N}`.
pub fn load(path: &Path, expected_depth: u32) -> HashMap<String, i32> {
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
        let Some(cp) = val.get("score_cp").and_then(|v| v.as_i64()) else {
            skipped += 1;
            continue;
        };
        match val.get("label_depth").and_then(|v| v.as_u64()) {
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
        // Last occurrence wins on a duplicate key -- deterministic given
        // JSONL is read top-to-bottom, and matches `write`'s own contract
        // (it always writes the current in-memory value, so a re-written
        // file's later lines reflect the most recent search).
        map.insert(sfen.to_string(), cp as i32);
    }
    if skipped > 0 {
        eprintln!("teacher cache: {skipped} lines skipped (unparseable)");
    }
    if depth_mismatch > 0 {
        eprintln!(
            "teacher cache: {depth_mismatch} entries skipped (label_depth != {expected_depth})"
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
/// `entries`: sfen → score_cp mapping; `label_depth` is recorded per line.
pub fn write(path: &Path, entries: &HashMap<String, i32>, label_depth: u32) -> std::io::Result<()> {
    let tmp_path = path.with_extension("jsonl.tmp");
    {
        let f = fs::File::create(&tmp_path)?;
        let mut w = BufWriter::new(f);
        for (sfen, &cp) in entries {
            writeln!(
                w,
                r#"{{"sfen":{},"label_depth":{},"score_cp":{}}}"#,
                json_string(sfen),
                label_depth,
                cp
            )?;
        }
        w.flush()?;
    }
    fs::rename(&tmp_path, path)?;
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
        write(f.path(), &expected, 4).unwrap();
        let loaded = load(f.path(), 4);
        assert_eq!(loaded, expected);
    }

    #[test]
    fn broken_lines_skipped() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "not json").unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":4,"score_cp":100}}"#).unwrap();
        let loaded = load(f.path(), 4);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[SFEN_A], 100);
    }

    #[test]
    fn missing_score_cp_skipped() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":4}}"#).unwrap();
        let loaded = load(f.path(), 4);
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
        let loaded = load(f.path(), 4);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[SFEN_A], 100);
    }

    #[test]
    fn wrong_depth_entries_are_filtered_out_and_reported() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_A}","label_depth":1,"score_cp":999}}"#).unwrap();
        writeln!(f, r#"{{"sfen":"{SFEN_B}","label_depth":4,"score_cp":100}}"#).unwrap();
        let loaded = load(f.path(), 4);
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
        let loaded = load(f.path(), 4);
        assert_eq!(loaded[SFEN_A], 250);
    }

    #[test]
    fn write_is_atomic_no_tmp_file_left_behind_on_success() {
        let f = NamedTempFile::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert(SFEN_A.to_string(), 48i32);
        write(f.path(), &entries, 4).unwrap();
        let tmp_path = f.path().with_extension("jsonl.tmp");
        assert!(
            !tmp_path.exists(),
            "the intermediate .tmp file must be renamed away, not left behind"
        );
        assert_eq!(load(f.path(), 4), entries);
    }
}
