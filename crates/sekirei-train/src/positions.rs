use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use sekirei_core::board::Board;
use sekirei_core::sfen::board_to_sfen;

#[allow(dead_code)]
#[derive(Clone)]
pub struct PositionSample {
    pub board: Board,
    pub phase: String,        // "opening" | "middlegame" | "endgame"
    pub side_to_move: String, // "black" | "white"
    pub ply: u32,
    pub source: String, // source.path
}

/// Load positions from a shogiesa positions.jsonl file.
pub fn load_positions(path: &Path) -> Vec<PositionSample> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot read positions file {:?}: {e}", path);
            return vec![];
        }
    };

    let mut samples = Vec::new();
    let mut skipped = 0usize;

    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            skipped += 1;
            continue;
        };
        let Some(sfen) = val.get("sfen").and_then(|v| v.as_str()) else {
            eprintln!("positions line {}: missing sfen field", i + 1);
            skipped += 1;
            continue;
        };
        let board = match Board::from_sfen(sfen) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("positions line {}: invalid SFEN ({e})", i + 1);
                skipped += 1;
                continue;
            }
        };

        // Extract tags (fall back to defaults if absent)
        let phase = val
            .pointer("/tags/phase")
            .and_then(|v| v.as_str())
            .unwrap_or("middlegame")
            .to_string();
        let side_to_move = val
            .pointer("/tags/side_to_move")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ply = val
            .pointer("/source/ply")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let source = val
            .pointer("/source/path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        samples.push(PositionSample {
            board,
            phase,
            side_to_move,
            ply,
            source,
        });
    }

    if skipped > 0 {
        eprintln!("positions: {skipped} lines skipped");
    }
    samples
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

    // Build (hash, index) per source
    let mut by_source: HashMap<&str, Vec<(u64, usize)>> = HashMap::new();
    for (i, s) in samples.iter().enumerate() {
        let sfen = board_to_sfen(&s.board);
        let key = format!("{}\0{}", s.source, sfen);
        let h = sfen_hash(&key, seed);
        by_source.entry(&s.source).or_default().push((h, i));
    }

    // Keep the `cap` lowest-hash indices per source
    let mut keep = HashSet::new();
    for group in by_source.values_mut() {
        group.sort_unstable();
        for &(_, idx) in group.iter().take(cap) {
            keep.insert(idx);
        }
    }

    samples
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep.contains(i))
        .map(|(_, s)| s)
        .collect()
}

/// FNV-1a hash XORed with seed — used for deterministic split and source cap.
pub fn sfen_hash(sfen: &str, seed: u64) -> u64 {
    let mut h = 14695981039346656037u64;
    for b in sfen.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h ^ seed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const STARTPOS_SFEN: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
    const SFEN_2: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2";

    fn make_jsonl(records: &[(&str, &str, &str, u32, &str)]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        for (sfen, phase, side, ply, src) in records {
            writeln!(
                f,
                r#"{{"schema_version":1,"sfen":"{sfen}","source":{{"kind":"csa","path":"{src}","ply":{ply}}},"tags":{{"phase":"{phase}","side_to_move":"{side}","in_check":false,"has_capture":false}},"observations":[]}}"#
            )
            .unwrap();
        }
        f
    }

    #[test]
    fn load_positions_basic() {
        let f = make_jsonl(&[(STARTPOS_SFEN, "opening", "black", 1, "game1.csa")]);
        let samples = load_positions(f.path());
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].phase, "opening");
        assert_eq!(samples[0].side_to_move, "black");
        assert_eq!(samples[0].ply, 1);
        assert_eq!(samples[0].source, "game1.csa");
    }

    #[test]
    fn source_cap_limits_per_source() {
        let f = make_jsonl(&[
            (STARTPOS_SFEN, "middlegame", "black", 20, "game1.csa"),
            (SFEN_2, "middlegame", "white", 22, "game1.csa"),
        ]);
        let samples = load_positions(f.path());
        let capped = apply_source_cap(samples, 1, 42);
        assert_eq!(capped.len(), 1, "source cap=1 keeps only 1 from game1.csa");

        let samples2 = load_positions(f.path());
        let uncapped = apply_source_cap(samples2, 0, 42);
        assert_eq!(uncapped.len(), 2);
    }

    #[test]
    fn source_cap_is_deterministic() {
        let f = make_jsonl(&[
            (STARTPOS_SFEN, "middlegame", "black", 20, "g.csa"),
            (SFEN_2, "middlegame", "white", 22, "g.csa"),
        ]);
        let sfens1: Vec<String> = {
            let s = load_positions(f.path());
            apply_source_cap(s, 1, 42)
                .iter()
                .map(|s| board_to_sfen(&s.board))
                .collect()
        };
        let sfens2: Vec<String> = {
            let s = load_positions(f.path());
            apply_source_cap(s, 1, 42)
                .iter()
                .map(|s| board_to_sfen(&s.board))
                .collect()
        };
        assert_eq!(sfens1, sfens2, "same seed → same selection");
    }

    #[test]
    fn source_cap_order_independent() {
        let f = make_jsonl(&[
            (STARTPOS_SFEN, "middlegame", "black", 20, "g.csa"),
            (SFEN_2, "middlegame", "white", 22, "g.csa"),
        ]);
        // forward order
        let s1 = load_positions(f.path());
        let set1: HashSet<String> = apply_source_cap(s1, 1, 42)
            .iter()
            .map(|s| board_to_sfen(&s.board))
            .collect();

        // reversed order
        let mut s2 = load_positions(f.path());
        s2.reverse();
        let set2: HashSet<String> = apply_source_cap(s2, 1, 42)
            .iter()
            .map(|s| board_to_sfen(&s.board))
            .collect();

        assert_eq!(
            set1, set2,
            "file order must not affect which samples are kept"
        );
    }

    #[test]
    fn validation_split_is_deterministic() {
        let h1a = sfen_hash(STARTPOS_SFEN, 42);
        let h1b = sfen_hash(STARTPOS_SFEN, 42);
        assert_eq!(h1a, h1b);
        assert_ne!(sfen_hash(STARTPOS_SFEN, 42), sfen_hash(STARTPOS_SFEN, 99));
        assert_ne!(sfen_hash(STARTPOS_SFEN, 42), sfen_hash(SFEN_2, 42));
    }

    #[test]
    fn missing_sfen_is_skipped() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"not_sfen": "foo"}}"#).unwrap();
        assert!(load_positions(f.path()).is_empty());
    }

    #[test]
    fn tags_fallback_when_absent() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"{{"schema_version":1,"sfen":"{STARTPOS_SFEN}","source":{{"kind":"csa","path":"x.csa","ply":5}},"observations":[]}}"#
        )
        .unwrap();
        let samples = load_positions(f.path());
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].phase, "middlegame");
        assert_eq!(samples[0].side_to_move, "");
    }
}
