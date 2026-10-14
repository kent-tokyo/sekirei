//! Accepted positions backed by an anonymous, immutable JSONL snapshot.
//!
//! The original input is scanned only once. Later passes read the accepted
//! snapshot, so changing or removing that input cannot change an epoch. Resident
//! position data is one decoded row per iterator, plus an O(N) offset index;
//! this is deliberately not a constant-total-memory claim. Partitioning and
//! selection share the snapshot and preserve the original accepted-row order.

use std::cell::RefCell;
use std::collections::{BinaryHeap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::rc::Rc;

use sekirei_core::sfen::board_to_sfen;
use shogiesa_core::schema::parse_json_line;

use crate::positions::{
    PositionIngestReport, PositionSample, map_record, scan_positions, sfen_hash,
};

/// A repeatable selection of accepted positions, without retaining all boards.
pub struct PositionStore {
    snapshot: Rc<RefCell<SnapshotReader>>,
    offsets: Vec<u64>,
}

struct SnapshotReader {
    reader: BufReader<File>,
    next_offset: Option<u64>,
}

impl PositionStore {
    /// Validate the input and snapshot accepted rows in an anonymous tempfile.
    ///
    /// The tempfile is closed and removed when the last selection is dropped,
    /// including on scan/write failure. Only row endings are normalized; the
    /// report's SHA-256 continues to identify the exact original input bytes.
    pub fn load(path: &Path, strict: bool) -> Result<(Self, PositionIngestReport), String> {
        let file = tempfile::tempfile()
            .map_err(|error| format!("cannot create positions snapshot: {error}"))?;
        let mut writer = BufWriter::new(file);
        let mut offsets = Vec::new();
        let mut next_offset = 0u64;
        let report = scan_positions(path, strict, |_sample, raw| {
            let row = raw.trim_end_matches(['\r', '\n']);
            let end = next_offset
                .checked_add(row.len() as u64)
                .and_then(|offset| offset.checked_add(1))
                .ok_or_else(|| "positions snapshot exceeds the byte-offset limit".to_string())?;
            writer
                .write_all(row.as_bytes())
                .and_then(|()| writer.write_all(b"\n"))
                .map_err(|error| format!("cannot write positions snapshot: {error}"))?;
            offsets.push(next_offset);
            next_offset = end;
            Ok(())
        })?;
        let file = writer
            .into_inner()
            .map_err(|error| format!("cannot flush positions snapshot: {error}"))?;
        Ok((
            Self {
                snapshot: Rc::new(RefCell::new(SnapshotReader {
                    reader: BufReader::new(file),
                    next_offset: None,
                })),
                offsets,
            },
            report,
        ))
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Read in the selection's original input order.
    pub fn iter(&self) -> Result<PositionStoreIter<'_>, String> {
        Ok(self.make_iter(IterationOrder::Original(0..self.len())))
    }

    /// Read selected indices in arbitrary order, including repeated indices.
    ///
    /// This supports the existing exact epoch shuffle without cloning boards.
    pub fn ordered_iter<'a>(
        &'a self,
        indices: &'a [usize],
    ) -> Result<PositionStoreIter<'a>, String> {
        if let Some(index) = indices.iter().find(|&&index| index >= self.len()) {
            return Err(format!(
                "positions index {index} is outside selection of length {}",
                self.len()
            ));
        }
        Ok(self.make_iter(IterationOrder::Selected(indices.iter())))
    }

    fn make_iter<'a>(&'a self, order: IterationOrder<'a>) -> PositionStoreIter<'a> {
        PositionStoreIter {
            store: self,
            order,
            line: String::new(),
            failed: false,
        }
    }

    /// Preserve the legacy per-source (FNV hash, input-index) selection exactly.
    ///
    /// Each source retains at most `cap` candidate pairs in a max heap. The
    /// selected offsets are emitted in input order, not heap/hash-map order.
    pub fn source_cap(mut self, cap: usize, seed: u64) -> Result<Self, String> {
        if cap == 0 {
            return Ok(self);
        }
        let mut candidates: HashMap<String, BinaryHeap<(u64, usize)>> = HashMap::new();
        for (index, sample) in self.iter()?.enumerate() {
            let sample = sample?;
            let key = format!("{}\0{}", sample.source, board_to_sfen(&sample.board));
            let candidate = (sfen_hash(&key, seed), index);
            let heap = candidates.entry(sample.source).or_default();
            if heap.len() < cap {
                heap.push(candidate);
            } else if heap.peek().is_some_and(|largest| candidate < *largest) {
                heap.pop();
                heap.push(candidate);
            }
        }
        let mut keep = vec![false; self.len()];
        for heap in candidates.into_values() {
            for (_, index) in heap {
                keep[index] = true;
            }
        }
        let mut index = 0;
        self.offsets.retain(|_| {
            let retained = keep[index];
            index += 1;
            retained
        });
        Ok(self)
    }

    /// Partition in stable input order and share the immutable backing file.
    pub fn partition(
        self,
        mut predicate: impl FnMut(&PositionSample) -> bool,
    ) -> Result<(Self, Self), String> {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (&offset, sample) in self.offsets.iter().zip(self.iter()?) {
            if predicate(&sample?) {
                left.push(offset);
            } else {
                right.push(offset);
            }
        }
        Ok((
            Self {
                snapshot: Rc::clone(&self.snapshot),
                offsets: left,
            },
            Self {
                snapshot: self.snapshot,
                offsets: right,
            },
        ))
    }

    /// Filter in stable order; leave the selection unchanged on read failure.
    pub fn retain(
        &mut self,
        mut predicate: impl FnMut(&PositionSample) -> bool,
    ) -> Result<(), String> {
        let mut retained = Vec::new();
        for (&offset, sample) in self.offsets.iter().zip(self.iter()?) {
            if predicate(&sample?) {
                retained.push(offset);
            }
        }
        self.offsets = retained;
        Ok(())
    }
}

impl SnapshotReader {
    fn read_sample(&mut self, offset: u64, line: &mut String) -> Result<PositionSample, String> {
        // One shared buffered reader avoids duplicated-descriptor seek races.
        // Consecutive rows reuse its buffer; interleaved iterators and shuffled
        // passes seek only when their next row differs from its current cursor.
        if self.next_offset != Some(offset) {
            self.next_offset = None;
            self.reader.seek(SeekFrom::Start(offset)).map_err(|error| {
                format!("positions snapshot byte {offset}: cannot seek: {error}")
            })?;
        }
        line.clear();
        self.next_offset = None;
        let bytes = self.reader.read_line(line).map_err(|error| {
            format!("positions snapshot byte {offset}: cannot read JSONL row: {error}")
        })?;
        if bytes == 0 {
            return Err(format!(
                "positions snapshot byte {offset}: unexpected end of file"
            ));
        }
        self.next_offset = offset.checked_add(bytes as u64);
        let record = parse_json_line(line)
            .map_err(|error| format!("positions snapshot byte {offset}: {error}"))?;
        map_record(record).map_err(|error| format!("positions snapshot byte {offset}: {error}"))
    }
}

enum IterationOrder<'a> {
    Original(std::ops::Range<usize>),
    Selected(std::slice::Iter<'a, usize>),
}

impl Iterator for IterationOrder<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Original(indices) => indices.next(),
            Self::Selected(indices) => indices.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Original(indices) => indices.size_hint(),
            Self::Selected(indices) => indices.size_hint(),
        }
    }
}

/// A fallible row iterator. An I/O/decoding failure terminates that iterator.
pub struct PositionStoreIter<'a> {
    store: &'a PositionStore,
    order: IterationOrder<'a>,
    line: String,
    failed: bool,
}

impl Iterator for PositionStoreIter<'_> {
    type Item = Result<PositionSample, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let index = self.order.next()?;
        let offset = self.store.offsets[index];
        let result = self
            .store
            .snapshot
            .borrow_mut()
            .read_sample(offset, &mut self.line);
        self.failed = result.is_err();
        Some(result)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.failed {
            (0, Some(0))
        } else {
            // A failure can terminate the stream before all rows are read.
            (0, self.order.size_hint().1)
        }
    }
}

impl std::iter::FusedIterator for PositionStoreIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::positions::{apply_source_cap, load_positions};
    use serde_json::{Value, json};
    use tempfile::NamedTempFile;

    const START: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
    const SECOND: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2";

    fn row(sfen: &str, side: &str, source: &str, ply: u32) -> String {
        json!({
            "schema_version": 11, "sfen": sfen,
            "source": {"kind": "csa", "path": source, "ply": ply},
            "tags": {"phase": "opening", "side_to_move": side,
                "in_check": false, "has_capture": false},
            "observations": []
        })
        .to_string()
    }

    fn fixture() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        for (sfen, side, source, ply) in [
            (START, "black", "a", 1),
            (SECOND, "white", "a", 2),
            (START, "black", "b", 3),
            (START, "black", "a", 4), // Same hash; input index breaks the tie.
            (SECOND, "white", "b", 5),
            (START, "black", "c", 6),
        ] {
            writeln!(file, "{}", row(sfen, side, source, ply)).unwrap();
        }
        file
    }

    fn signature(sample: &PositionSample) -> Value {
        json!({
            "sfen": board_to_sfen(&sample.board), "phase": sample.phase,
            "side": sample.side_to_move, "ply": sample.ply,
            "source": sample.source, "source_kind": sample.source_kind,
            "root_id": sample.root_id, "variation_id": sample.variation_id,
            "branch_from_ply": sample.branch_from_ply,
            "observations": sample.observations, "stability": sample.stability,
            "game_result": format!("{:?}", sample.game_result),
            "game_result_source": sample.game_result_source,
        })
    }

    fn signatures(store: &PositionStore) -> Vec<Value> {
        store
            .iter()
            .unwrap()
            .map(|sample| signature(&sample.unwrap()))
            .collect()
    }

    #[test]
    fn snapshot_matches_legacy_load_and_non_strict_rejections() {
        let mut file = fixture();
        writeln!(file, "\ninvalid json").unwrap();
        write!(file, "{}", row(START, "black", "last", 7)).unwrap();
        let legacy = load_positions(file.path(), false).unwrap();
        let (store, report) = PositionStore::load(file.path(), false).unwrap();
        assert_eq!(store.len(), legacy.samples.len());
        assert!(!store.is_empty());
        assert_eq!(
            signatures(&store),
            legacy.samples.iter().map(signature).collect::<Vec<_>>()
        );
        assert_eq!(report.sha256, legacy.sha256);
        assert_eq!(report.non_empty_lines, legacy.non_empty_lines);
        assert_eq!(report.skipped_by_reason, legacy.skipped_by_reason);
    }

    #[test]
    fn source_cap_preserves_legacy_hash_selection_ties_and_order() {
        let file = fixture();
        for cap in [0, 1, 2, 3, usize::MAX] {
            for seed in [0, 42, u64::MAX] {
                let legacy = load_positions(file.path(), true).unwrap().samples;
                let expected = apply_source_cap(legacy, cap, seed);
                let (store, _) = PositionStore::load(file.path(), true).unwrap();
                let store = store.source_cap(cap, seed).unwrap();
                assert_eq!(
                    signatures(&store),
                    expected.iter().map(signature).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn stable_partition_and_retain_match_in_memory_selection() {
        let file = fixture();
        let legacy = load_positions(file.path(), true).unwrap().samples;
        let split =
            |sample: &PositionSample| sfen_hash(&board_to_sfen(&sample.board), 42) % 1000 >= 500;
        let (expected_train, expected_valid): (Vec<_>, Vec<_>) =
            legacy.into_iter().partition(split);
        let (store, _) = PositionStore::load(file.path(), true).unwrap();
        let (mut train, valid) = store.partition(split).unwrap();
        assert_eq!(
            signatures(&train),
            expected_train.iter().map(signature).collect::<Vec<_>>()
        );
        assert_eq!(
            signatures(&valid),
            expected_valid.iter().map(signature).collect::<Vec<_>>()
        );
        train.retain(|sample| sample.source == "a").unwrap();
        assert_eq!(
            signatures(&train),
            expected_train
                .iter()
                .filter(|sample| sample.source == "a")
                .map(signature)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ordered_and_interleaved_iterators_do_not_share_a_logical_cursor() {
        let file = fixture();
        let (store, _) = PositionStore::load(file.path(), true).unwrap();
        let expected = signatures(&store);
        let order = [5, 0, 3, 3, 1, 2, 4];
        let mut original = store.iter().unwrap();
        let mut reordered = store.ordered_iter(&order).unwrap();
        for (index, &selected) in order.iter().enumerate() {
            assert_eq!(
                signature(&reordered.next().unwrap().unwrap()),
                expected[selected]
            );
            if index < expected.len() {
                assert_eq!(
                    signature(&original.next().unwrap().unwrap()),
                    expected[index]
                );
            }
        }
        assert!(original.next().is_none());
        assert!(reordered.next().is_none());
        assert!(store.ordered_iter(&[]).unwrap().next().is_none());
        assert!(
            store
                .ordered_iter(&[store.len()])
                .err()
                .unwrap()
                .contains("outside selection")
        );
    }

    #[test]
    fn snapshot_is_unchanged_after_input_mutation_or_removal() {
        let mut file = fixture();
        let (store, _) = PositionStore::load(file.path(), true).unwrap();
        let expected = signatures(&store);
        file.as_file().set_len(0).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        writeln!(file, "corrupted input").unwrap();
        assert_eq!(signatures(&store), expected);
        drop(file);
        assert_eq!(signatures(&store), expected);
        let (left, right) = store.partition(|sample| sample.source == "a").unwrap();
        drop(left);
        assert_eq!(right.len(), 3);
        assert_eq!(signatures(&right).len(), 3);
    }

    #[test]
    fn empty_input_and_load_errors_are_reported() {
        let mut file = NamedTempFile::new().unwrap();
        let (store, _) = PositionStore::load(file.path(), true).unwrap();
        assert!(store.is_empty());
        assert!(store.iter().unwrap().next().is_none());
        writeln!(file, "broken").unwrap();
        assert!(
            PositionStore::load(file.path(), true)
                .err()
                .unwrap()
                .contains("line 1")
        );
        let path = file.path().to_owned();
        drop(file);
        assert!(PositionStore::load(&path, true).is_err());
    }

    #[test]
    fn snapshot_read_failure_is_fused_and_retain_is_transactional() {
        let file = fixture();
        let (mut store, _) = PositionStore::load(file.path(), true).unwrap();
        let original_offsets = store.offsets.clone();
        store
            .snapshot
            .borrow_mut()
            .reader
            .get_mut()
            .set_len(0)
            .unwrap();
        let mut rows = store.iter().unwrap();
        assert!(
            rows.next()
                .unwrap()
                .err()
                .unwrap()
                .contains("unexpected end of file")
        );
        assert!(rows.next().is_none());
        assert!(rows.next().is_none());
        drop(rows);
        assert!(store.retain(|_| true).is_err());
        assert_eq!(store.offsets, original_offsets);
    }
}
