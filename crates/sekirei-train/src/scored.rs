use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Load a quietset scored JSONL into a map of sfen → stability_score.
/// Only entries with stability_score >= min_stability are kept.
pub fn load_scored(path: &Path, min_stability: f32) -> HashMap<String, f32> {
    let mut map = HashMap::new();
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warn: cannot read scored file {}: {e}", path.display());
            return map;
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let (Some(id), Some(score)) = (
            extract_str(line, "sample_id"),
            extract_f32(line, "stability_score"),
        ) && score >= min_stability
        {
            map.insert(id, score);
        }
    }
    eprintln!(
        "Loaded {} stable samples (min_stability={min_stability}) from {}",
        map.len(),
        path.display()
    );
    map
}

fn extract_str(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\":\"", key);
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_f32(line: &str, key: &str) -> Option<f32> {
    let needle = format!("\"{}\":", key);
    let start = line.find(&needle)? + needle.len();
    let rest = line[start..].trim_start();
    let end = rest.find([',', '}', ' ']).unwrap_or(rest.len());
    rest[..end].parse().ok()
}
