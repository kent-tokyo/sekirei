//! Standalone synthetic feature experiment. Never activates an engine evaluator.
//! See docs/design/halfka_feature_probe.md for the versioned diagnostic format.

#![forbid(unsafe_code)]

use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::halfkp;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::mv::{Move, MoveToken};
use sekirei_core::piece::PieceKind;
use sekirei_core::sfen::{board_to_sfen, move_from_usi, move_to_usi};
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

/// Move-derived column operations. The bool is true for an added column.
type ColumnDeltas = [Vec<(usize, bool)>; 2];

#[derive(Clone, Copy)]
struct MoveKinds {
    king: bool,
    drop: bool,
    capture: bool,
    promotion: bool,
}

struct MoveFrame {
    geometry: MoveToken,
    before_hash: u64,
    after_hash: u64,
    before_sfen: String,
    text: String,
    kinds: MoveKinds,
    deltas: ColumnDeltas,
    saved_king: Option<(usize, Box<[i16; HALF]>)>,
}

/// Owns both Board and rollback frames. No frame can be copied or used by a
/// caller on another Board; only the most recent frame can be consumed.
struct MoveState<'a> {
    network: &'a Network,
    board: Board,
    values: [[i16; HALF]; 2],
    frames: Vec<MoveFrame>,
}

impl<'a> MoveState<'a> {
    fn new(network: &'a Network, sfen: &str) -> io::Result<Self> {
        let board = Board::from_sfen(sfen).map_err(|_| invalid("invalid SFEN"))?;
        let values = [
            network.accumulate(&features(&board, Color::Black, network.kings)?),
            network.accumulate(&features(&board, Color::White, network.kings)?),
        ];
        Ok(Self {
            network,
            board,
            values,
            frames: Vec::new(),
        })
    }

    fn verify(&self) -> io::Result<()> {
        for perspective in [Color::Black, Color::White] {
            let full =
                self.network
                    .accumulate(&features(&self.board, perspective, self.network.kings)?);
            if self.values[perspective.index()] != full {
                return Err(invalid(
                    "Board move-derived accumulator/full-refresh mismatch",
                ));
            }
        }
        Ok(())
    }

    fn deltas(&self, mv: Move) -> io::Result<ColumnDeltas> {
        let mover = self.board.side_to_move;
        let captured = self.board.piece_at(mv.to);
        if captured.is_some_and(|piece| piece.kind == PieceKind::Ou) {
            return Err(invalid("king capture is not a legal transition"));
        }
        let mut deltas: ColumnDeltas = std::array::from_fn(|_| Vec::new());
        for perspective in [Color::Black, Color::White] {
            // The moving king changes every conditioning index on its side.
            // The old perspective is saved; after do_move it is fully rebuilt.
            if mv.piece_kind == PieceKind::Ou && perspective == mover {
                continue;
            }
            let own_king = self
                .board
                .king_square(perspective)
                .ok_or_else(|| invalid("missing king"))?;
            let columns = &mut deltas[perspective.index()];
            if mv.piece_kind == PieceKind::Ou {
                if self.network.kings {
                    let from = mv.from.ok_or_else(|| invalid("king drop"))?;
                    let base = BASE_INPUT + format_square(own_king, perspective) * 162 + 81;
                    columns.push((base + format_square(from, perspective), false));
                    columns.push((base + format_square(mv.to, perspective), true));
                }
            } else {
                if let Some(from) = mv.from {
                    columns.push((
                        halfkp::board_feature(own_king, from, mv.piece_kind, mover, perspective)
                            .ok_or_else(|| invalid("missing moved-piece feature"))?,
                        false,
                    ));
                } else {
                    let count = self.board.hand(mover).get(mv.piece_kind);
                    columns.push((
                        halfkp::hand_feature(own_king, mv.piece_kind, count, mover, perspective)
                            .ok_or_else(|| invalid("drop without hand feature"))?,
                        false,
                    ));
                }
                let kind = if mv.promote {
                    mv.piece_kind.promoted()
                } else {
                    mv.piece_kind
                };
                columns.push((
                    halfkp::board_feature(own_king, mv.to, kind, mover, perspective)
                        .ok_or_else(|| invalid("missing destination feature"))?,
                    true,
                ));
            }
            if let Some(piece) = captured {
                columns.push((
                    halfkp::board_feature(own_king, mv.to, piece.kind, piece.color, perspective)
                        .ok_or_else(|| invalid("missing captured-piece feature"))?,
                    false,
                ));
                let hand_kind = piece.kind.unpromoted();
                let count = self.board.hand(mover).get(hand_kind) + 1;
                columns.push((
                    halfkp::hand_feature(own_king, hand_kind, count, mover, perspective)
                        .ok_or_else(|| invalid("missing captured hand feature"))?,
                    true,
                ));
            }
        }
        Ok(deltas)
    }

    fn make_move(&mut self, text: &str) -> io::Result<(MoveKinds, usize, usize)> {
        if self.frames.len() >= 128 {
            return Err(invalid("move sequence exceeds 128 plies"));
        }
        let mv = move_from_usi(text, &self.board).map_err(|_| invalid("invalid USI move"))?;
        if move_to_usi(mv) != text {
            return Err(invalid("expected canonical USI move spelling"));
        }
        if !generate_legal_moves(&mut self.board).contains(&mv) {
            return Err(invalid("move is not legal in the current Board"));
        }
        let mover = self.board.side_to_move.index();
        let kinds = MoveKinds {
            king: mv.piece_kind == PieceKind::Ou,
            drop: mv.is_drop(),
            capture: self.board.piece_at(mv.to).is_some(),
            promotion: mv.promote,
        };
        let deltas = self.deltas(mv)?;
        let saved_king = kinds.king.then(|| (mover, Box::new(self.values[mover])));
        let before_hash = self.board.hash();
        let before_sfen = board_to_sfen(&self.board);
        let geometry = self.board.do_move(mv);
        let mut logical_delta = 0;
        for (side, columns) in deltas.iter().enumerate() {
            for &(feature, add) in columns {
                self.network.add(&mut self.values[side], feature, !add);
                logical_delta += HALF;
            }
        }
        let logical_refresh = if kinds.king {
            let perspective = self.board.side_to_move.flip();
            let active = features(&self.board, perspective, self.network.kings)?;
            self.values[mover] = self.network.accumulate(&active);
            active.len() * HALF
        } else {
            0
        };
        self.frames.push(MoveFrame {
            geometry,
            before_hash,
            after_hash: self.board.hash(),
            before_sfen,
            text: text.to_string(),
            kinds,
            deltas,
            saved_king,
        });
        self.verify()?;
        Ok((kinds, logical_delta, logical_refresh))
    }

    fn undo(&mut self) -> io::Result<(String, MoveKinds, usize)> {
        let frame = self
            .frames
            .last()
            .ok_or_else(|| invalid("no move to undo"))?;
        if self.board.hash() != frame.after_hash {
            return Err(invalid("rollback child identity mismatch"));
        }
        let frame = self.frames.pop().expect("checked stack");
        self.board.undo_move(frame.geometry);
        let mut logical_delta = 0;
        for (side, columns) in frame.deltas.iter().enumerate() {
            for &(feature, add) in columns.iter().rev() {
                self.network.add(&mut self.values[side], feature, add);
                logical_delta += HALF;
            }
        }
        if let Some((side, saved)) = frame.saved_king {
            self.values[side] = *saved;
        }
        if self.board.hash() != frame.before_hash || board_to_sfen(&self.board) != frame.before_sfen
        {
            return Err(invalid("Board::undo_move failed to restore SFEN/hash"));
        }
        self.verify()?;
        Ok((frame.text, frame.kinds, logical_delta))
    }
}

fn trace_json(
    network: &Network,
    board: &Board,
    active: &[Vec<usize>; 2],
    accumulators: &[[i16; HALF]; 2],
    logical_delta_adds: usize,
    delta_checked: bool,
) -> String {
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
    format!(
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
        delta_checked
    )
}

fn emit_move_state(
    state: &MoveState<'_>,
    phase: &str,
    text: &str,
    kinds: MoveKinds,
    logical_delta: usize,
    logical_refresh: usize,
) -> io::Result<()> {
    let active = [
        features(&state.board, Color::Black, state.network.kings)?,
        features(&state.board, Color::White, state.network.kings)?,
    ];
    let trace = trace_json(
        state.network,
        &state.board,
        &active,
        &state.values,
        0,
        false,
    );
    println!(
        "{{\"phase\":\"{}\",\"move\":\"{}\",\"sfen\":\"{}\",\"hash\":\"{:016x}\",\"stack_depth\":{},\"king_move\":{},\"drop\":{},\"capture\":{},\"promotion\":{},\"logical_edge_delta_adds\":{},\"logical_edge_refresh_adds\":{},\"saved_king_payload_bytes\":{},\"trace\":{}}}",
        phase,
        text,
        board_to_sfen(&state.board),
        state.board.hash(),
        state.frames.len(),
        kinds.king,
        kinds.drop,
        kinds.capture,
        kinds.promotion,
        logical_delta,
        logical_refresh,
        if kinds.king { HALF * 2 } else { 0 },
        trace
    );
    Ok(())
}

fn run_moves(network: &Network) -> io::Result<()> {
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (sfen, move_text) = line
            .split_once('\t')
            .ok_or_else(|| invalid("expected SFEN<TAB>USI moves"))?;
        let moves: Vec<_> = move_text.split_whitespace().collect();
        if moves.is_empty() || moves.len() > 128 {
            return Err(invalid("expected 1..128 legal moves"));
        }
        let mut state = MoveState::new(network, sfen)?;
        emit_move_state(
            &state,
            "initial",
            "",
            MoveKinds {
                king: false,
                drop: false,
                capture: false,
                promotion: false,
            },
            0,
            0,
        )?;
        for text in moves {
            let (kinds, delta, refresh) = state.make_move(text)?;
            emit_move_state(&state, "do", text, kinds, delta, refresh)?;
        }
        while !state.frames.is_empty() {
            let (text, kinds, delta) = state.undo()?;
            emit_move_state(&state, "undo", &text, kinds, delta, 0)?;
        }
    }
    Ok(())
}

fn run() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(args.len() == 2 || (args.len() == 3 && args[2] == "--moves"))
        || !["halfkp", "halfka-kings"].contains(&args[1].as_str())
    {
        return Err(invalid(
            "usage: halfka_feature_probe IMAGE halfkp|halfka-kings [--moves] < sfens.txt",
        ));
    }
    let network = Network::load(&args[0], args[1] == "halfka-kings")?;
    if args.len() == 3 {
        return run_moves(&network);
    }
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
        println!(
            "{}",
            trace_json(
                &network,
                &board,
                &active,
                &accumulators,
                logical_delta_adds,
                previous.is_some()
            )
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
