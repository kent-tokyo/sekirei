//! Position storage choice with identical selection and epoch order.
//!
//! Disk storage keeps accepted JSONL in an anonymous snapshot and only row
//! offsets in memory. Offsets, shuffle indices, and teacher caches still scale
//! with record count; this is not a constant-memory training claim.

use crate::csa::GameResult;
use crate::position_store::PositionStore;
use crate::positions::{self, PositionIngestReport, PositionSample};
use sekirei_core::sfen::board_to_sfen;
use std::collections::HashMap;
use std::path::Path;

pub type SampleIter<'a> = Box<dyn Iterator<Item = Result<PositionSample, String>> + 'a>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PositionStorage {
    #[default]
    Memory,
    Disk,
}

impl PositionStorage {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "memory" => Ok(Self::Memory),
            "disk" => Ok(Self::Disk),
            _ => Err(format!(
                "unknown --positions-storage {value:?}; expected memory or disk"
            )),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Disk => "disk",
        }
    }
}

pub enum PositionDataset {
    Memory(Vec<PositionSample>),
    Disk(PositionStore),
}

#[derive(Default)]
pub struct DatasetStats {
    pub count: usize,
    pub black: usize,
    pub known_result: usize,
    pub split_hash: u64,
}

impl DatasetStats {
    pub fn side_weights(&self) -> HashMap<String, f32> {
        let total = self.count as f32;
        let black = self.black as f32;
        let white = total - black;
        [("black", black), ("white", white)]
            .into_iter()
            .map(|(side, count)| {
                (
                    side.to_string(),
                    if count > 0.0 {
                        0.5 * total / count
                    } else {
                        1.0
                    },
                )
            })
            .collect()
    }
}

impl PositionDataset {
    pub fn load(
        path: &Path,
        strict: bool,
        storage: PositionStorage,
    ) -> Result<(Self, PositionIngestReport), String> {
        match storage {
            PositionStorage::Memory => {
                let mut samples = Vec::new();
                let report = positions::scan_positions(path, strict, |sample, _| {
                    samples.push(sample);
                    Ok(())
                })?;
                Ok((Self::Memory(samples), report))
            }
            PositionStorage::Disk => {
                let (store, report) = PositionStore::load(path, strict)?;
                Ok((Self::Disk(store), report))
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Memory(v) => v.len(),
            Self::Disk(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Memory(v) => v.is_empty(),
            Self::Disk(v) => v.is_empty(),
        }
    }

    pub fn iter(&self) -> Result<SampleIter<'_>, String> {
        match self {
            Self::Memory(v) => Ok(Box::new(v.iter().cloned().map(Ok))),
            Self::Disk(v) => Ok(Box::new(v.iter()?)),
        }
    }

    pub fn epoch_iter<'a>(&'a self, order: Option<&'a [usize]>) -> Result<SampleIter<'a>, String> {
        let Some(order) = order else {
            return self.iter();
        };
        match self {
            Self::Memory(v) => Ok(Box::new(order.iter().map(|&i| {
                v.get(i)
                    .cloned()
                    .ok_or_else(|| format!("position index {i} exceeds {} rows", v.len()))
            }))),
            Self::Disk(v) => Ok(Box::new(v.ordered_iter(order)?)),
        }
    }

    pub fn source_cap(self, cap: usize, seed: u64) -> Result<Self, String> {
        match self {
            Self::Memory(v) => Ok(Self::Memory(positions::apply_source_cap(v, cap, seed))),
            Self::Disk(v) => Ok(Self::Disk(v.source_cap(cap, seed)?)),
        }
    }

    pub fn partition(
        self,
        mut predicate: impl FnMut(&PositionSample) -> bool,
    ) -> Result<(Self, Self), String> {
        match self {
            Self::Memory(v) => {
                let (a, b) = v.into_iter().partition(|sample| predicate(sample));
                Ok((Self::Memory(a), Self::Memory(b)))
            }
            Self::Disk(v) => {
                let (a, b) = v.partition(predicate)?;
                Ok((Self::Disk(a), Self::Disk(b)))
            }
        }
    }

    pub fn retain(&mut self, predicate: impl FnMut(&PositionSample) -> bool) -> Result<(), String> {
        match self {
            Self::Memory(v) => {
                v.retain(predicate);
                Ok(())
            }
            Self::Disk(v) => v.retain(predicate),
        }
    }

    pub fn stats(&self) -> Result<DatasetStats, String> {
        let mut stats = DatasetStats::default();
        for sample in self.iter()? {
            let sample = sample?;
            stats.count += 1;
            stats.black += usize::from(sample.side_to_move == "black");
            stats.known_result += usize::from(sample.game_result != GameResult::Unknown);
            stats.split_hash = stats
                .split_hash
                .wrapping_add(positions::sfen_hash(&board_to_sfen(&sample.board), 0));
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::Write;
    use tempfile::NamedTempFile;

    const START: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
    const SECOND: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2";
    const MODES: [PositionStorage; 2] = [PositionStorage::Memory, PositionStorage::Disk];

    fn fixture() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        for (sfen, side, source, ply, outcome) in [
            (START, "black", "a", 1, Some("black_wins")),
            (SECOND, "white", "a", 2, Some("white_wins")),
            (START, "black", "a", 3, Some("draw")),
            (START, "black", "b", 4, None),
            (START, "black", "b", 5, Some("unknown")),
            (SECOND, "white", "c", 6, Some("black_wins")),
        ] {
            let mut row = json!({
                "schema_version": 11,
                "sfen": sfen,
                "source": {"kind": "csa", "path": source, "ply": ply},
                "tags": {"phase": "opening", "side_to_move": side,
                    "in_check": false, "has_capture": false},
                "observations": []
            });
            if let Some(outcome) = outcome {
                row["game_result"] = json!({"outcome": outcome, "result_source": "csa_terminal"});
            }
            if ply == 6 {
                write!(file, "{row}").unwrap();
            } else {
                write!(file, "{row}\r\n").unwrap();
            }
        }
        file
    }

    fn signature(sample: PositionSample) -> Value {
        json!({
            "sfen": board_to_sfen(&sample.board), "source": sample.source,
            "phase": sample.phase, "side": sample.side_to_move, "ply": sample.ply,
            "game_result": format!("{:?}", sample.game_result),
            "game_result_source": sample.game_result_source,
        })
    }

    fn epoch_rows(
        dataset: &PositionDataset,
        order: Option<&[usize]>,
    ) -> Result<Vec<Value>, String> {
        dataset
            .epoch_iter(order)?
            .map(|sample| sample.map(signature))
            .collect()
    }

    fn rows(dataset: &PositionDataset) -> Vec<Value> {
        dataset
            .iter()
            .unwrap()
            .map(|sample| signature(sample.unwrap()))
            .collect()
    }

    fn plies(dataset: &PositionDataset) -> Vec<u32> {
        dataset
            .iter()
            .unwrap()
            .map(|sample| sample.unwrap().ply)
            .collect()
    }

    #[test]
    fn storage_setting_defaults_to_memory_and_rejects_unknown_modes() {
        assert_eq!(PositionStorage::default(), PositionStorage::Memory);
        for mode in MODES {
            assert_eq!(PositionStorage::parse(mode.as_str()).unwrap(), mode);
        }
        for unknown in ["", "mmap", "DISK", "memory "] {
            assert!(
                PositionStorage::parse(unknown)
                    .unwrap_err()
                    .contains("expected memory or disk")
            );
        }
    }

    #[test]
    fn both_storage_modes_preserve_original_and_ordered_epoch_rows() {
        let file = fixture();
        let (memory, memory_report) =
            PositionDataset::load(file.path(), true, PositionStorage::Memory).unwrap();
        let expected = rows(&memory);
        let order = [5, 0, 2, 2, 1, 4, 3];
        let reordered: Vec<_> = order.iter().map(|&index| expected[index].clone()).collect();
        for mode in MODES {
            let (dataset, report) = PositionDataset::load(file.path(), true, mode).unwrap();
            assert_eq!(dataset.len(), 6);
            assert!(!dataset.is_empty());
            assert_eq!(
                serde_json::to_value(report).unwrap(),
                serde_json::to_value(&memory_report).unwrap()
            );
            assert_eq!(rows(&dataset), expected);
            assert_eq!(epoch_rows(&dataset, None).unwrap(), expected);
            assert_eq!(epoch_rows(&dataset, Some(&order)).unwrap(), reordered);
            assert!(epoch_rows(&dataset, Some(&[])).unwrap().is_empty());
            assert!(
                epoch_rows(&dataset, Some(&[0, dataset.len()]))
                    .unwrap_err()
                    .contains("index 6")
            );
            assert_eq!(epoch_rows(&dataset, None).unwrap(), expected);
        }
    }

    #[test]
    fn source_caps_partition_and_retain_agree_across_storage_modes() {
        let file = fixture();
        for cap in [0, 1, 2] {
            let mut selected = Vec::new();
            for mode in MODES {
                let (dataset, _) = PositionDataset::load(file.path(), true, mode).unwrap();
                let dataset = dataset.source_cap(cap, 42).unwrap();
                let retained = plies(&dataset);
                assert!(retained.windows(2).all(|pair| pair[0] < pair[1]));
                if cap == 1 {
                    assert_eq!(dataset.len(), 3);
                    assert!(!retained.contains(&3));
                    assert!(!retained.contains(&5));
                }
                selected.push(rows(&dataset));
            }
            assert_eq!(selected[0], selected[1]);
        }
        for mode in MODES {
            let (dataset, _) = PositionDataset::load(file.path(), true, mode).unwrap();
            let (mut odd, mut even) = dataset.partition(|sample| sample.ply % 2 == 1).unwrap();
            assert_eq!(plies(&odd), vec![1, 3, 5]);
            assert_eq!(plies(&even), vec![2, 4, 6]);
            odd.retain(|sample| sample.source == "a").unwrap();
            even.retain(|sample| sample.source != "a").unwrap();
            assert_eq!(plies(&odd), vec![1, 3]);
            assert_eq!(plies(&even), vec![4, 6]);
            drop(odd);
            assert_eq!(plies(&even), vec![4, 6]);
            even.retain(|_| false).unwrap();
            assert!(even.is_empty());
        }
    }

    #[test]
    fn stats_count_known_outcomes_and_preserve_duplicate_split_fingerprints() {
        let file = fixture();
        let expected_split = positions::sfen_hash(START, 0)
            .wrapping_mul(4)
            .wrapping_add(positions::sfen_hash(SECOND, 0).wrapping_mul(2));
        for mode in MODES {
            let (dataset, _) = PositionDataset::load(file.path(), true, mode).unwrap();
            let stats = dataset.stats().unwrap();
            assert_eq!((stats.count, stats.black, stats.known_result), (6, 4, 4));
            assert_eq!(stats.split_hash, expected_split);
            assert_eq!(stats.side_weights()["black"], 0.75);
            assert_eq!(stats.side_weights()["white"], 1.5);

            let (black, white) = dataset
                .partition(|sample| sample.side_to_move == "black")
                .unwrap();
            let black = black.stats().unwrap();
            let white = white.stats().unwrap();
            assert_eq!((black.count, black.black, black.known_result), (4, 4, 2));
            assert_eq!((white.count, white.black, white.known_result), (2, 0, 2));
            assert_eq!(black.side_weights()["black"], 0.5);
            assert_eq!(black.side_weights()["white"], 1.0);
            assert_eq!(white.side_weights()["black"], 1.0);
            assert_eq!(white.side_weights()["white"], 0.5);
            assert_eq!(
                black.split_hash.wrapping_add(white.split_hash),
                expected_split
            );
        }
    }

    #[test]
    fn empty_inputs_remain_empty_through_dataset_operations() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "\r\n \t\n").unwrap();
        for mode in MODES {
            let (dataset, report) = PositionDataset::load(file.path(), true, mode).unwrap();
            assert_eq!(report.accepted, 0);
            assert_eq!(report.physical_lines, 2);
            assert!(dataset.is_empty());
            assert_eq!(dataset.len(), 0);
            assert!(rows(&dataset).is_empty());
            assert!(epoch_rows(&dataset, Some(&[])).unwrap().is_empty());
            assert!(epoch_rows(&dataset, Some(&[0])).is_err());
            let dataset = dataset.source_cap(1, 42).unwrap();
            let (mut left, right) = dataset.partition(|_| panic!("empty selection")).unwrap();
            left.retain(|_| panic!("empty selection")).unwrap();
            assert!(left.is_empty() && right.is_empty());
            let stats = left.stats().unwrap();
            assert_eq!(
                (
                    stats.count,
                    stats.black,
                    stats.known_result,
                    stats.split_hash
                ),
                (0, 0, 0, 0)
            );
            assert_eq!(stats.side_weights()["black"], 1.0);
            assert_eq!(stats.side_weights()["white"], 1.0);
        }
    }

    #[test]
    fn loading_errors_and_non_strict_recovery_are_consistent() {
        let mut file = fixture();
        write!(file, "\ninvalid JSON\n").unwrap();
        for mode in MODES {
            let error = PositionDataset::load(file.path(), true, mode)
                .err()
                .unwrap();
            assert!(error.contains("line 7"));
            let (dataset, report) = PositionDataset::load(file.path(), false, mode).unwrap();
            assert_eq!(dataset.len(), 6);
            assert_eq!(report.accepted, 6);
            assert_eq!(report.skipped, 1);
        }
        let missing = file.path().to_owned();
        drop(file);
        for mode in MODES {
            assert!(PositionDataset::load(&missing, true, mode).is_err());
        }
    }
}
