//! PGX v2.6.0 shogi environment comparison adapter, independent of evaluation.
//!
//! Read newline-delimited SFEN or SFEN\tACTION_INDEX from stdin.
//! `iterations` counts batches, each
//! containing `batch-size` independent operations (default: input count).
//! Probe emits one JSON object per input; timing emits one summary object.
//! Initial parsing/masks/action selection and one warm-up batch are untimed.

use rayon::prelude::*;
use sekirei_core::{
    board::Board,
    color::Color,
    movegen::{FixedMoveList, generate_legal_moves_into_fixed, is_in_check},
    mv::Move,
    piece::PieceKind,
    sfen::board_to_sfen,
    square::Square,
};
use serde_json::json;
use std::{
    env,
    hint::black_box,
    io::{self, BufRead},
    time::Instant,
};

const ACTIONS: usize = 27 * 81;
const CHANNELS: usize = 119;
const OBSERVATION_BITS: usize = 81 * CHANNELS;
const KINDS: [PieceKind; 14] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kaku,
    PieceKind::Hisha,
    PieceKind::Kin,
    PieceKind::Ou,
    PieceKind::Tokin,
    PieceKind::Narikyo,
    PieceKind::Narikei,
    PieceKind::Narigin,
    PieceKind::Uma,
    PieceKind::Ryu,
];
const HAND_PLANES: [(PieceKind, usize); 7] = [
    (PieceKind::Fu, 8),
    (PieceKind::Kyou, 4),
    (PieceKind::Kei, 4),
    (PieceKind::Gin, 4),
    (PieceKind::Kin, 4),
    (PieceKind::Kaku, 2),
    (PieceKind::Hisha, 2),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Mask,
    Step,
}

struct Options {
    operation: Operation,
    iterations: usize,
    threads: usize,
    batch_size: Option<usize>,
    probe: bool,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self {
            operation: Operation::Mask,
            iterations: 100,
            threads: 1,
            batch_size: None,
            probe: false,
        };
        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {arg}"))?;
            match arg.as_str() {
                "--operation" => {
                    options.operation = match value.as_str() {
                        "mask" => Operation::Mask,
                        "step" => Operation::Step,
                        _ => return Err(format!("invalid operation: {value}")),
                    }
                }
                "--output" => {
                    options.probe = match value.as_str() {
                        "probe" => true,
                        "timing" => false,
                        _ => return Err(format!("invalid output: {value}")),
                    }
                }
                "--iterations" | "--threads" | "--batch-size" => {
                    let n = value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid {arg}: {value}"))?;
                    if n == 0 {
                        return Err(format!("{arg} must be positive"));
                    }
                    match arg.as_str() {
                        "--iterations" => options.iterations = n,
                        "--threads" => options.threads = n,
                        _ => options.batch_size = Some(n),
                    }
                }
                _ => return Err(format!("unknown option: {arg}")),
            }
        }
        Ok(options)
    }
}

struct Input {
    board: Board,
    mask: Vec<bool>,
    action: usize,
}

/// PGX uses file 1..9 major, rank 1..9 minor, then rotates for White.
fn pgx_square(square: Square, color: Color) -> usize {
    let index = (square.file() as usize - 1) * 9 + square.rank_0() as usize;
    if color == Color::White {
        80 - index
    } else {
        index
    }
}

fn kind_index(kind: PieceKind) -> usize {
    KINDS
        .iter()
        .position(|&candidate| candidate == kind)
        .unwrap()
}

/// Direction is the ray from destination back toward the move's source,
/// matching PGX's LEGAL_FROM_IDX, including the two knight directions.
fn encode_action(mv: Move, color: Color) -> usize {
    let to = pgx_square(mv.to, color);
    let direction = if let Some(from) = mv.from {
        let from = pgx_square(from, color);
        let dx = (from / 9) as i32 - (to / 9) as i32;
        let dy = (from % 9) as i32 - (to % 9) as i32;
        let direction = if mv.piece_kind == PieceKind::Kei && dy == 2 {
            match dx {
                -1 => 8,
                1 => 9,
                _ => panic!("invalid knight move"),
            }
        } else {
            match (dx.signum(), dy.signum()) {
                (0, 1) => 0,
                (-1, 1) => 1,
                (1, 1) => 2,
                (-1, 0) => 3,
                (1, 0) => 4,
                (0, -1) => 5,
                (-1, -1) => 6,
                (1, -1) => 7,
                _ => panic!("invalid move direction"),
            }
        };
        direction + if mv.promote { 10 } else { 0 }
    } else {
        let index = kind_index(mv.piece_kind);
        assert!(index < 7, "invalid drop kind");
        20 + index
    };
    direction * 81 + to
}

fn absolute_square(index: usize, color: Color) -> Square {
    let index = if color == Color::White {
        80 - index
    } else {
        index
    };
    Square::from_shogi((index / 9 + 1) as u8, (index % 9 + 1) as u8)
}

/// Decode the dense label in the timed step path, as PGX does. Search the
/// destination-to-source ray for the first friendly piece, not the first
/// occupied square. Input-mask validation guarantees the resulting move is legal.
fn decode_action(board: &Board, action: usize) -> Move {
    let direction = action / 81;
    let to_index = action % 81;
    let color = board.side_to_move;
    let to = absolute_square(to_index, color);
    if direction >= 20 {
        return Move::drop(to, KINDS[direction - 20]);
    }
    const SOURCE_DIRECTIONS: [(i32, i32); 10] = [
        (0, 1),
        (-1, 1),
        (1, 1),
        (-1, 0),
        (1, 0),
        (0, -1),
        (-1, -1),
        (1, -1),
        (-1, 2),
        (1, 2),
    ];
    let source_direction = direction % 10;
    let (dx, dy) = SOURCE_DIRECTIONS[source_direction];
    let (mut x, mut y) = ((to_index / 9) as i32, (to_index % 9) as i32);
    for _ in 0..8 {
        x += dx;
        y += dy;
        if !(0..9).contains(&x) || !(0..9).contains(&y) {
            break;
        }
        let from = absolute_square((x * 9 + y) as usize, color);
        if let Some(piece) = board.piece_at(from)
            && piece.color == color
        {
            return Move::normal(from, to, piece.kind, direction >= 10);
        }
        if source_direction >= 8 {
            break;
        }
    }
    panic!("legal PGX action has no friendly source: {action}");
}

fn legal_mask(board: &mut Board, moves: &mut FixedMoveList) -> Vec<bool> {
    generate_legal_moves_into_fixed(board, moves);
    let mut mask = vec![false; ACTIONS];
    for &mv in moves.as_slice() {
        let action = encode_action(mv, board.side_to_move);
        assert!(
            !mask[action],
            "distinct legal moves map to the same PGX action"
        );
        mask[action] = true;
    }
    mask
}

/// Geometrical attacks include occupied endpoints, both friendly and enemy.
/// Only intermediate squares block sliding attacks; pins are irrelevant here.
fn attacks(board: &Board, from: Square, to: Square, kind: PieceKind, color: Color) -> bool {
    if from == to {
        return false;
    }
    let dx = to.file_0() as i32 - from.file_0() as i32;
    let physical_dy = to.rank_0() as i32 - from.rank_0() as i32;
    let dy = if color == Color::Black {
        physical_dy
    } else {
        -physical_dy
    };
    let ax = dx.abs();
    let ay = dy.abs();
    let near = ax <= 1 && ay <= 1;
    let diagonal = ax == ay;
    let orthogonal = dx == 0 || dy == 0;
    let reachable = match kind {
        PieceKind::Fu => dx == 0 && dy == -1,
        PieceKind::Kyou => dx == 0 && dy < 0,
        PieceKind::Kei => ax == 1 && dy == -2,
        PieceKind::Gin => (ax <= 1 && dy == -1) || (ax == 1 && dy == 1),
        PieceKind::Kin
        | PieceKind::Tokin
        | PieceKind::Narikyo
        | PieceKind::Narikei
        | PieceKind::Narigin => {
            (ax <= 1 && dy == -1) || (ax == 1 && dy == 0) || (dx == 0 && dy == 1)
        }
        PieceKind::Kaku => diagonal,
        PieceKind::Hisha => orthogonal,
        PieceKind::Ou => near,
        PieceKind::Uma => diagonal || near,
        PieceKind::Ryu => orthogonal || near,
    };
    if !reachable {
        return false;
    }
    if kind == PieceKind::Kei || near {
        return true;
    }
    let (sx, sy) = (dx.signum(), physical_dy.signum());
    let (mut x, mut y) = (from.file_0() as i32 + sx, from.rank_0() as i32 + sy);
    while x != to.file_0() as i32 || y != to.rank_0() as i32 {
        if board.piece_at(Square::from_fr(x as u8, y as u8)).is_some() {
            return false;
        }
        x += sx;
        y += sy;
    }
    true
}

/// Flatten the public PGX (9, 9, 119) observation in C order.
/// PGX's final rot90(k=3) becomes rank-major, core-file-minor here.
fn observation(board: &Board) -> Vec<bool> {
    let mut bits = vec![false; OBSERVATION_BITS];
    let mut effects = [[[false; 81]; 14]; 2];
    let mut counts = [[0_u8; 81]; 2];
    let stm = board.side_to_move;
    let output_square = |square: Square| {
        let index = if stm == Color::Black {
            square.index() as usize
        } else {
            80 - square.index() as usize
        };
        (index % 9) * 9 + index / 9
    };
    for index in 0..81 {
        let from = Square::from_index(index as u8);
        let Some(piece) = board.piece_at(from) else {
            continue;
        };
        let side = usize::from(piece.color != stm);
        let kind = kind_index(piece.kind);
        bits[output_square(from) * CHANNELS + side * 31 + kind] = true;
        for to_index in 0..81 {
            let to = Square::from_index(to_index as u8);
            if attacks(board, from, to, piece.kind, piece.color) {
                effects[side][kind][to_index] = true;
                counts[side][to_index] += 1;
            }
        }
    }
    for index in 0..81 {
        let offset = output_square(Square::from_index(index as u8)) * CHANNELS;
        for side in 0..2 {
            for kind in 0..14 {
                bits[offset + side * 31 + 14 + kind] = effects[side][kind][index];
            }
            for threshold in 1..=3 {
                bits[offset + side * 31 + 28 + threshold - 1] =
                    counts[side][index] as usize >= threshold;
            }
        }
        let mut channel = 62;
        for color in [stm, stm.flip()] {
            for (kind, planes) in HAND_PLANES {
                for n in 1..=planes {
                    bits[offset + channel] = board.hand(color).get(kind) as usize >= n;
                    channel += 1;
                }
            }
        }
        debug_assert_eq!(channel, 118);
    }
    let checked = is_in_check(board, stm);
    for square in 0..81 {
        bits[square * CHANNELS + 118] = checked;
    }
    bits
}

struct Output {
    board: Option<Board>,
    mask: Vec<bool>,
    observation: Vec<bool>,
    terminal: bool,
    rewards: [i8; 2],
}

fn perform(input: &Input, operation: Operation) -> Output {
    let mut board = input.board.clone();
    let mut moves = FixedMoveList::new();
    if operation == Operation::Mask {
        return Output {
            mask: legal_mask(&mut board, &mut moves),
            board: None,
            observation: Vec::new(),
            terminal: false,
            rewards: [0, 0],
        };
    }
    assert!(
        input.mask[input.action],
        "chosen action must be legal in the input mask"
    );
    let mv = decode_action(&board, input.action);
    let _token = board.do_move(mv);
    let mut mask = legal_mask(&mut board, &mut moves);
    let no_moves = !mask.iter().any(|&legal| legal);
    let terminal = no_moves || board.ply >= 512;
    let mut rewards = [0, 0];
    if no_moves {
        rewards[board.side_to_move.index()] = -1;
        rewards[board.side_to_move.flip().index()] = 1;
    }
    let observation = observation(&board);
    if terminal {
        mask.fill(true);
    }
    Output {
        board: Some(board),
        mask,
        observation,
        terminal,
        rewards,
    }
}

fn batch(
    inputs: &[Input],
    operation: Operation,
    batch_size: usize,
    start: usize,
    parallel: bool,
) -> Vec<Output> {
    if parallel {
        (0..batch_size)
            .into_par_iter()
            .map(|offset| perform(&inputs[(start + offset) % inputs.len()], operation))
            .collect()
    } else {
        (0..batch_size)
            .map(|offset| perform(&inputs[(start + offset) % inputs.len()], operation))
            .collect()
    }
}

fn main() -> Result<(), String> {
    let options = Options::parse()?;
    let mut inputs = Vec::new();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let (sfen, requested_action) = if let Some((sfen, action)) = line.trim().split_once('\t') {
            let action = action
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("invalid action index: {action}"))?;
            (sfen.trim(), Some(action))
        } else {
            (line.trim(), None)
        };
        let original = Board::from_sfen(sfen)?;
        let mut warmed = original.clone();
        let mut moves = FixedMoveList::new();
        let mask = legal_mask(&mut warmed, &mut moves);
        if original.ply >= 512 || !mask.iter().any(|&legal| legal) {
            return Err(format!(
                "only nonterminal legal inputs are supported: {sfen}"
            ));
        }
        let action =
            requested_action.unwrap_or_else(|| mask.iter().position(|&legal| legal).unwrap());
        if !mask.get(action).copied().unwrap_or(false) {
            return Err(format!("requested action {action} is illegal for {sfen}"));
        }
        // Keep mask input cold; step input has the public state's initial mask
        // already computed. Move application invalidates its legality cache.
        let board = if options.operation == Operation::Step {
            warmed
        } else {
            original
        };
        inputs.push(Input {
            board,
            mask,
            action,
        });
    }
    if inputs.is_empty() {
        return Err("no SFEN input".to_owned());
    }
    let batch_size = options.batch_size.unwrap_or(inputs.len());
    options
        .iterations
        .checked_mul(batch_size)
        .ok_or("operation count overflow")?;
    let operation_name = if options.operation == Operation::Mask {
        "mask"
    } else {
        "step"
    };
    if options.probe {
        for (index, input) in inputs.iter().enumerate() {
            let output = perform(input, options.operation);
            let board = output.board.as_ref().unwrap_or(&input.board);
            let observed = if options.operation == Operation::Step {
                output.observation
            } else {
                observation(board)
            };
            println!(
                "{}",
                json!({ "index": index, "operation": operation_name,
                "initial_legal_actions": input.mask.iter().enumerate().filter_map(|(i, &v)| v.then_some(i)).collect::<Vec<_>>(),
                "legal_actions": output.mask.iter().enumerate().filter_map(|(i, &v)| v.then_some(i)).collect::<Vec<_>>(),
                "chosen_action": input.action, "output_sfen": board_to_sfen(board),
                "terminal": output.terminal, "rewards": output.rewards,
                "observation_shape": [9, 9, CHANNELS], "observation_bits": observed })
            );
        }
        return Ok(());
    }
    let run = || {
        black_box(batch(
            &inputs,
            options.operation,
            batch_size,
            0,
            options.threads > 1,
        ));
        let started = Instant::now();
        for iteration in 0..options.iterations {
            black_box(batch(
                &inputs,
                options.operation,
                batch_size,
                iteration * batch_size,
                options.threads > 1,
            ));
        }
        started.elapsed().as_nanos()
    };
    let elapsed_ns = if options.threads == 1 {
        run()
    } else {
        rayon::ThreadPoolBuilder::new()
            .num_threads(options.threads)
            .build()
            .map_err(|e| e.to_string())?
            .install(run)
    };
    let operations = options.iterations * batch_size;
    println!(
        "{}",
        json!({ "operation": operation_name, "threads": options.threads,
        "inputs": inputs.len(), "iterations": options.iterations, "batch_size": batch_size,
        "warmup_batches": 1, "warmup_operations": batch_size,
        "elapsed_ns": elapsed_ns, "ns": elapsed_ns, "num_operations": operations,
        "ns_per_operation": elapsed_ns as f64 / operations as f64,
        "includes": "fresh Board clone; FixedMoveList allocation; dense mask allocation; step also action decode, move, terminal, observation, reward; batch collection and drop" })
    );
    Ok(())
}
