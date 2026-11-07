//! Standalone synthetic feature experiment. Never activates an engine evaluator.
//! See docs/design/halfka_feature_probe.md for the versioned diagnostic format.

#![forbid(unsafe_code)]

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::halfkp;
use sekirei_core::piece::PieceKind;
use sekirei_core::square::Square;
use std::collections::BTreeSet;
use std::io::{self, BufRead, Read};

const HALF: usize = 256;
const HIDDEN: usize = 32;
const BASE_INPUT: usize = 125_388;
const KING_INPUT: usize = 81 * 2 * 81;
const MAX_ROWS: usize = 4096;
const HEADER: usize = 40;
const TAIL: usize = 32 * 4 + 32 * 512 + 32 * 4 + 32 * 32 + 4 + 32;
const MAX_BYTES: usize = HEADER + 512 + MAX_ROWS * 516 + TAIL;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| invalid("offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated image"))?;
        self.offset = end;
        Ok(value.try_into().expect("checked length"))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn i16(&mut self) -> io::Result<i16> {
        Ok(i16::from_le_bytes(self.take()?))
    }
    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.take()?))
    }
    fn i8(&mut self) -> io::Result<i8> {
        Ok(i8::from_le_bytes(self.take()?))
    }
}

struct Layer {
    bias: [i32; HIDDEN],
    weights: Vec<i8>,
    inputs: usize,
}

fn safe_bias(value: i32, inputs: usize) -> bool {
    let margin = (inputs * 127 * 128) as i32;
    value >= i32::MIN + margin && value <= i32::MAX - margin
}

impl Layer {
    fn read(reader: &mut Reader<'_>, inputs: usize) -> io::Result<Self> {
        let mut bias = [0; HIDDEN];
        for item in &mut bias {
            *item = reader.i32()?;
            if !safe_bias(*item, inputs) {
                return Err(invalid("unsafe i32 affine bias headroom"));
            }
        }
        let weights = (0..HIDDEN * inputs)
            .map(|_| reader.i8())
            .collect::<io::Result<_>>()?;
        Ok(Self {
            bias,
            weights,
            inputs,
        })
    }
    fn forward(&self, input: &[i32]) -> [i32; HIDDEN] {
        let mut result = [0; HIDDEN];
        for (output, row) in result.iter_mut().enumerate() {
            let mut total = self.bias[output];
            for (weight, value) in self.weights[output * self.inputs..(output + 1) * self.inputs]
                .iter()
                .zip(input)
            {
                total += i32::from(*weight) * value;
            }
            *row = (total >> 6).clamp(0, 127);
        }
        result
    }
}

struct Network {
    kings: bool,
    scale: i32,
    bias: [i16; HALF],
    rows: Vec<(usize, [i16; HALF])>,
    first: Layer,
    second: Layer,
    output_bias: i32,
    output_weights: [i8; HIDDEN],
}

impl Network {
    fn load(path: &str, expect_kings: bool) -> io::Result<Self> {
        if std::fs::metadata(path)?.len() > MAX_BYTES as u64 {
            return Err(invalid("diagnostic image byte cap exceeded"));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(invalid("diagnostic image grew beyond byte cap"));
        }
        let mut reader = Reader {
            bytes: &bytes,
            offset: 0,
        };
        if &reader.take::<8>()? != b"SKXK0001" || reader.u32()? != 1 {
            return Err(invalid("unsupported diagnostic magic/version"));
        }
        let kind = reader.u32()?;
        if kind > 1 || (kind == 1) != expect_kings {
            return Err(invalid(
                "feature architecture does not match explicit expected kind",
            ));
        }
        let inputs = BASE_INPUT + if expect_kings { KING_INPUT } else { 0 };
        if reader.u32()? as usize != inputs
            || reader.u32()? != HALF as u32
            || reader.u32()? != HIDDEN as u32
            || reader.u32()? != HIDDEN as u32
        {
            return Err(invalid("feature count or layer dimensions mismatch"));
        }
        let scale = reader.u32()?;
        let count = reader.u32()? as usize;
        if !(1..=128).contains(&scale)
            || count > MAX_ROWS
            || bytes.len() != HEADER + 512 + count * 516 + TAIL
        {
            return Err(invalid("scale, sparse row count or exact length mismatch"));
        }
        let mut bias = [0; HALF];
        for value in &mut bias {
            *value = reader.i16()?;
        }
        let mut rows = Vec::with_capacity(count);
        let mut previous = None;
        for _ in 0..count {
            let index = reader.u32()? as usize;
            if index >= inputs || previous.is_some_and(|old| old >= index) {
                return Err(invalid(
                    "FT rows must be strictly increasing, unique and in range",
                ));
            }
            let mut weights = [0; HALF];
            for value in &mut weights {
                *value = reader.i16()?;
            }
            rows.push((index, weights));
            previous = Some(index);
        }
        let first = Layer::read(&mut reader, 2 * HALF)?;
        let second = Layer::read(&mut reader, HIDDEN)?;
        let output_bias = reader.i32()?;
        if !safe_bias(output_bias, HIDDEN) {
            return Err(invalid("unsafe output bias headroom"));
        }
        let mut output_weights = [0; HIDDEN];
        for value in &mut output_weights {
            *value = reader.i8()?;
        }
        if reader.offset != bytes.len() {
            return Err(invalid("trailing bytes"));
        }
        Ok(Self {
            kings: expect_kings,
            scale: scale as i32,
            bias,
            rows,
            first,
            second,
            output_bias,
            output_weights,
        })
    }
    fn add(&self, accumulator: &mut [i16; HALF], feature: usize, subtract: bool) {
        if let Ok(index) = self.rows.binary_search_by_key(&feature, |row| row.0) {
            for (target, value) in accumulator.iter_mut().zip(&self.rows[index].1) {
                *target = if subtract {
                    target.wrapping_sub(*value)
                } else {
                    target.wrapping_add(*value)
                };
            }
        }
    }
    fn accumulate(&self, features: &[usize]) -> [i16; HALF] {
        let mut result = self.bias;
        for &feature in features {
            self.add(&mut result, feature, false);
        }
        result
    }
    fn delta(&self, before: &[usize], after: &[usize], value: &mut [i16; HALF]) -> usize {
        let before: BTreeSet<_> = before.iter().copied().collect();
        let after: BTreeSet<_> = after.iter().copied().collect();
        let mut operations = 0;
        for &feature in before.difference(&after) {
            self.add(value, feature, true);
            operations += HALF;
        }
        for &feature in after.difference(&before) {
            self.add(value, feature, false);
            operations += HALF;
        }
        operations
    }
}

fn format_square(square: Square, perspective: Color) -> usize {
    let value = (8 - square.file_0() as usize) * 9 + square.rank_0() as usize;
    if perspective == Color::Black {
        value
    } else {
        80 - value
    }
}

fn features(board: &Board, perspective: Color, kings: bool) -> io::Result<Vec<usize>> {
    let own_king = board
        .king_square(perspective)
        .ok_or_else(|| invalid("both kings required"))?;
    let other_king = board
        .king_square(perspective.flip())
        .ok_or_else(|| invalid("both kings required"))?;
    let mut output = Vec::new();
    let mut king_counts = [0; 2];
    let mut totals = [0usize; 7];
    for index in 0..81 {
        let square = Square::from_index(index);
        if let Some(piece) = board.piece_at(square) {
            if piece.kind == PieceKind::Ou {
                king_counts[piece.color.index()] += 1;
            } else {
                totals[piece.kind.unpromoted().index()] += 1;
                output.push(
                    halfkp::board_feature(own_king, square, piece.kind, piece.color, perspective)
                        .ok_or_else(|| invalid("non-king mapping failed"))?,
                );
            }
        }
    }
    let kinds = [
        PieceKind::Fu,
        PieceKind::Kyou,
        PieceKind::Kei,
        PieceKind::Gin,
        PieceKind::Kin,
        PieceKind::Kaku,
        PieceKind::Hisha,
    ];
    for color in [Color::Black, Color::White] {
        for kind in kinds {
            let count = board.hand(color).get(kind);
            totals[kind.index()] += count as usize;
            for number in 1..=count {
                output.push(
                    halfkp::hand_feature(own_king, kind, number, color, perspective)
                        .ok_or_else(|| invalid("hand mapping failed"))?,
                );
            }
        }
    }
    if king_counts != [1, 1]
        || totals
            .iter()
            .zip([18, 4, 4, 4, 4, 2, 2])
            .any(|(&count, limit)| count > limit)
    {
        return Err(invalid("invalid king count or piece inventory"));
    }
    if kings {
        let base = BASE_INPUT + format_square(own_king, perspective) * 162;
        output.push(base + format_square(own_king, perspective));
        output.push(base + 81 + format_square(other_king, perspective));
    }
    output.sort_unstable();
    if output.windows(2).any(|pair| pair[0] == pair[1])
        || output.len() > if kings { 40 } else { 38 }
    {
        return Err(invalid("duplicate or excessive active features"));
    }
    Ok(output)
}

fn run() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || !["halfkp", "halfka-kings"].contains(&args[1].as_str()) {
        return Err(invalid(
            "usage: halfka_feature_probe IMAGE halfkp|halfka-kings < sfens.txt",
        ));
    }
    let network = Network::load(&args[0], args[1] == "halfka-kings")?;
    let mut previous: Option<([Vec<usize>; 2], [[i16; HALF]; 2])> = None;
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let board = Board::from_sfen(&line).map_err(|_| invalid("invalid SFEN"))?;
        // Store deltas by absolute color, then order forward inputs by side to move.
        let active = [
            features(&board, Color::Black, network.kings)?,
            features(&board, Color::White, network.kings)?,
        ];
        let accumulators = [
            network.accumulate(&active[0]),
            network.accumulate(&active[1]),
        ];
        let mut logical_delta_adds = 0;
        if let Some((old_features, old_accumulators)) = &previous {
            for side in 0..2 {
                let mut updated = old_accumulators[side];
                logical_delta_adds +=
                    network.delta(&old_features[side], &active[side], &mut updated);
                if updated != accumulators[side] {
                    return Err(invalid("delta/fresh mismatch"));
                }
                network.delta(&active[side], &old_features[side], &mut updated);
                if updated != old_accumulators[side] {
                    return Err(invalid("undo mismatch"));
                }
            }
        }
        let us = board.side_to_move.index();
        let them = 1 - us;
        let input: Vec<i32> = accumulators[us]
            .iter()
            .chain(&accumulators[them])
            .map(|&x| i32::from(x.clamp(0, 127)))
            .collect();
        let first = network.first.forward(&input);
        let second = network.second.forward(&first);
        let raw = network.output_bias
            + network
                .output_weights
                .iter()
                .zip(second)
                .map(|(&w, x)| i32::from(w) * x)
                .sum::<i32>();
        let score = (raw / network.scale).clamp(-30_000, 30_000);
        println!(
            "{{\"features_us\":{:?},\"features_them\":{:?},\"acc_us\":{:?},\"acc_them\":{:?},\"l1\":{:?},\"l2\":{:?},\"raw_output\":{},\"score\":{},\"logical_refresh_adds\":{},\"logical_delta_adds\":{},\"delta_checked\":{},\"logical_dense_macs\":17440}}",
            active[us],
            active[them],
            accumulators[us],
            accumulators[them],
            first,
            second,
            raw,
            score,
            (active[0].len() + active[1].len()) * HALF,
            logical_delta_adds,
            previous.is_some()
        );
        previous = Some((active, accumulators));
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("halfka prototype rejected input: {error}");
        std::process::exit(2);
    }
}
