//! Convert `gensfen` text into fixed-size HalfKP training records, or print
//! the engine's static evaluation of such positions with a HalfKP file.
//!
//! ```text
//! halfkp_pack pack  OUT.bin IN.txt...     # training records
//! halfkp_pack check NN.bin FV_SCALE IN.txt [N]   # sfen \t engine eval (first N lines)
//! ```
//!
//! When the input is the six-column output of `gensfen`, split it by
//! `source_game_id` before packing train and validation files separately.
//! The game identifier is provenance metadata and is not stored in the
//! fixed-size binary record.
//!
//! A record is 158 bytes, little-endian:
//!
//! ```text
//! u8 side to move (0 Black, 1 White) | i8 result (stm view) | i16 score (stm view)
//! u8 king square of the side to move | u8 king square of the opponent
//! u16 x 38  piece features seen from the side to move
//! u16 x 38  piece features seen from the opponent
//! ```
//!
//! King squares and piece features are the two parts of a HalfKP feature
//! index (`king * 1548 + piece`) as computed by `sekirei_core::halfkp`, so the
//! trainer uses exactly the engine's feature mapping. Unused slots hold 1548.
use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::halfkp::{PIECE_FEATURES, board_feature, hand_feature};
use sekirei_core::piece::PieceKind;
use sekirei_core::square::Square;
use std::io::{BufRead, BufWriter, Write};

const SLOTS: usize = 38;
const HAND_KINDS: [PieceKind; 7] = [
    PieceKind::Fu,
    PieceKind::Kyou,
    PieceKind::Kei,
    PieceKind::Gin,
    PieceKind::Kin,
    PieceKind::Kaku,
    PieceKind::Hisha,
];

/// King square index and piece features of `board` seen from `perspective`.
fn features(board: &Board, perspective: Color) -> (u8, [u16; SLOTS]) {
    let king = board.king_square(perspective).expect("king");
    let mut out = [PIECE_FEATURES as u16; SLOTS];
    let mut n = 0;
    let mut king_index = 0u8;
    let mut push = |f: usize| {
        king_index = (f / PIECE_FEATURES) as u8;
        assert!(n < SLOTS, "more than {SLOTS} pieces");
        out[n] = (f % PIECE_FEATURES) as u16;
        n += 1;
    };
    for i in 0..81u8 {
        let sq = Square::from_index(i);
        if let Some(piece) = board.piece_at(sq)
            && let Some(f) = board_feature(king, sq, piece.kind, piece.color, perspective)
        {
            push(f);
        }
    }
    for color in [Color::Black, Color::White] {
        for kind in HAND_KINDS {
            for count in 1..=board.hand(color).get(kind) {
                if let Some(f) = hand_feature(king, kind, count, color, perspective) {
                    push(f);
                }
            }
        }
    }
    // A position without hand or board pieces (impossible in real games) has
    // no feature to take the king index from; compute it directly.
    if n == 0 {
        king_index = (board_feature(king, king, PieceKind::Fu, perspective, perspective).unwrap()
            / PIECE_FEATURES) as u8;
    }
    (king_index, out)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pack") => pack(&args[1], &args[2..]),
        Some("check") => check(&args[1], args[2].parse().unwrap(), &args[3], args.get(4)),
        _ => panic!("usage: halfkp_pack pack OUT IN... | check NN FV IN [N]"),
    }
}

fn pack(out: &str, inputs: &[String]) {
    let mut w = BufWriter::new(std::fs::File::create(out).expect("output"));
    let mut records = 0u64;
    let mut skipped = 0u64;
    for path in inputs {
        let file = std::fs::File::open(path).expect("input");
        for line in std::io::BufReader::new(file).lines() {
            let line = line.expect("read");
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 3 {
                skipped += 1;
                continue;
            }
            let Ok(board) = Board::from_sfen(cols[0]) else {
                skipped += 1;
                continue;
            };
            // A file still being appended to can end in a partial line.
            let (Ok(score), Ok(result)) = (cols[1].parse::<i32>(), cols[2].parse::<i8>()) else {
                skipped += 1;
                continue;
            };
            if cols.len() < 5 || !(-1..=1).contains(&result) {
                skipped += 1;
                continue;
            }
            let stm = board.side_to_move;
            let (k_us, f_us) = features(&board, stm);
            let (k_them, f_them) = features(&board, stm.flip());
            let mut rec = Vec::with_capacity(158);
            rec.push(u8::from(stm == Color::White));
            rec.push(result as u8);
            rec.extend_from_slice(&(score.clamp(-32_000, 32_000) as i16).to_le_bytes());
            rec.push(k_us);
            rec.push(k_them);
            for f in f_us.iter().chain(f_them.iter()) {
                rec.extend_from_slice(&f.to_le_bytes());
            }
            w.write_all(&rec).expect("write");
            records += 1;
        }
    }
    w.flush().expect("flush");
    eprintln!("{records} records, {skipped} lines skipped -> {out}");
}

fn check(nn: &str, fv_scale: i32, input: &str, n: Option<&String>) {
    let n: usize = n.map_or(20, |v| v.parse().unwrap());
    sekirei_core::halfkp::set_fv_scale(fv_scale).expect("fv scale");
    sekirei_core::nnue::load_evaluator(std::path::Path::new(nn)).expect("eval file");
    let file = std::fs::File::open(input).expect("input");
    for line in std::io::BufReader::new(file).lines().take(n) {
        let line = line.expect("read");
        let sfen = line.split('\t').next().unwrap();
        let board = Board::from_sfen(sfen).expect("sfen");
        println!("{sfen}\t{}", sekirei_core::eval::evaluate(&board));
    }
}
