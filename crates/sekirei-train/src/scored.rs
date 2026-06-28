use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
struct ScoredRow {
    sample_id: String,
    stability_score: f32,
}

/// Load a quietset scored JSONL into a map of sfen → stability_score.
/// Duplicate SFENs are averaged. Invalid scores (NaN/inf/out-of-range) are skipped.
/// Only entries with mean stability_score >= min_stability are kept.
pub fn load_scored(path: &Path, min_stability: f32) -> HashMap<String, f32> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warn: cannot read scored file {}: {e}", path.display());
            return HashMap::new();
        }
    };

    let mut accum: HashMap<String, (f64, u32)> = HashMap::new();
    let mut invalid = 0usize;
    let mut dup_count = 0usize;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<ScoredRow>(line) {
            Ok(row) => {
                let s = row.stability_score;
                if !s.is_finite() || !(0.0f32..=1.0).contains(&s) {
                    invalid += 1;
                    continue;
                }
                let e = accum.entry(row.sample_id).or_insert((0.0, 0));
                if e.1 > 0 {
                    dup_count += 1;
                }
                e.0 += s as f64;
                e.1 += 1;
            }
            Err(_) => {
                invalid += 1;
            }
        }
    }

    let map: HashMap<String, f32> = accum
        .into_iter()
        .filter_map(|(id, (sum, n))| {
            let mean = (sum / n as f64) as f32;
            if mean >= min_stability {
                Some((id, mean))
            } else {
                None
            }
        })
        .collect();

    if invalid > 0 {
        eprintln!("warn: {invalid} invalid/unparseable lines skipped");
    }
    if dup_count > 0 {
        eprintln!("warn: {dup_count} duplicate SFENs — stability_score averaged");
    }
    eprintln!(
        "Loaded {} stable samples (min_stability={min_stability}) from {}",
        map.len(),
        path.display()
    );
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_jsonl(lines: &[&str]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f
    }

    #[test]
    fn test_basic_load() {
        let f = write_jsonl(&[
            r#"{"sample_id":"sfen_a","stability_score":0.9,"decision":"keep"}"#,
            r#"{"sample_id":"sfen_b","stability_score":0.5,"decision":"review"}"#,
        ]);
        let map = load_scored(f.path(), 0.85);
        assert_eq!(map.len(), 1);
        assert!((map["sfen_a"] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn test_duplicate_sfen_averaged() {
        let f = write_jsonl(&[
            r#"{"sample_id":"sfen_a","stability_score":0.9}"#,
            r#"{"sample_id":"sfen_a","stability_score":0.7}"#,
        ]);
        let map = load_scored(f.path(), 0.0);
        assert_eq!(map.len(), 1);
        let mean = map["sfen_a"];
        assert!((mean - 0.8).abs() < 1e-5, "expected ~0.8, got {mean}");
    }

    #[test]
    fn test_invalid_scores_skipped() {
        let f = write_jsonl(&[
            r#"{"sample_id":"good","stability_score":0.9}"#,
            r#"{"sample_id":"too_high","stability_score":1.2}"#,
            r#"{"sample_id":"negative","stability_score":-0.1}"#,
            r#"not valid json"#,
        ]);
        let map = load_scored(f.path(), 0.0);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("good"));
    }

    #[test]
    fn test_min_stability_filter() {
        let f = write_jsonl(&[
            r#"{"sample_id":"high","stability_score":0.9}"#,
            r#"{"sample_id":"low","stability_score":0.4}"#,
        ]);
        let map = load_scored(f.path(), 0.85);
        assert!(map.contains_key("high"));
        assert!(!map.contains_key("low"));
    }

    #[test]
    fn test_spaced_json_parses() {
        // serde_json handles optional whitespace around colon/comma
        let f = write_jsonl(&[r#"{"sample_id": "sfen_a", "stability_score": 0.9}"#]);
        let map = load_scored(f.path(), 0.0);
        assert!(map.contains_key("sfen_a"));
    }
}
