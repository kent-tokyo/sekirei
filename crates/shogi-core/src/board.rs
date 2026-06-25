use crate::bitboard::Bitboard;
use crate::color::Color;
use crate::hand::Hand;
use crate::mv::{Move, MoveToken};
use crate::nnue::NnueAcc;
use crate::piece::{Piece, PieceKind};
use crate::square::Square;
use crate::zobrist;

/// Extract hand piece counts as [[u8; 7]; 2] for NNUE refresh.
/// hand_counts[color_idx][kind_idx] = count (kind_idx: Fu=0..Hisha=6)
fn hand_counts_array(hand: &[Hand; 2]) -> [[u8; 7]; 2] {
    let hand_kinds = [
        PieceKind::Fu, PieceKind::Kyou, PieceKind::Kei, PieceKind::Gin,
        PieceKind::Kin, PieceKind::Kaku, PieceKind::Hisha,
    ];
    let mut out = [[0u8; 7]; 2];
    for ci in 0..2 {
        for (ki, &kind) in hand_kinds.iter().enumerate() {
            out[ci][ki] = hand[ci].get(kind);
        }
    }
    out
}

/// Map an SFEN piece character to its base (unpromoted) PieceKind.
fn sfen_char_to_base_kind(c: char) -> Option<PieceKind> {
    match c.to_ascii_lowercase() {
        'p' => Some(PieceKind::Fu),
        'l' => Some(PieceKind::Kyou),
        'n' => Some(PieceKind::Kei),
        's' => Some(PieceKind::Gin),
        'g' => Some(PieceKind::Kin),
        'b' => Some(PieceKind::Kaku),
        'r' => Some(PieceKind::Hisha),
        'k' => Some(PieceKind::Ou),
        _   => None,
    }
}

/// Shogi board position — includes an incrementally-updated NNUE accumulator.
#[derive(Clone)]
pub struct Board {
    /// `piece_bb[color][kind]` = bitboard of that piece type for that color
    piece_bb:         [[Bitboard; PieceKind::COUNT]; 2],
    /// `occ[color]` = occupancy bitboard for all pieces of that color
    occ:              [Bitboard; 2],
    /// Mailbox for O(1) piece lookup by square
    mailbox:          [Option<Piece>; Square::NUM],
    hand:             [Hand; 2],
    pub side_to_move: Color,
    pub ply:          u32,
    hash:             u64,
    /// NNUE accumulator — kept in sync with the board position via inverse deltas
    pub acc:          NnueAcc,
}

impl Board {
    pub(crate) fn empty() -> Self {
        Board {
            piece_bb:     [[Bitboard::EMPTY; PieceKind::COUNT]; 2],
            occ:          [Bitboard::EMPTY; 2],
            mailbox:      [None; Square::NUM],
            hand:         [Hand::new(); 2],
            side_to_move: Color::Black,
            ply:          0,
            hash:         0,
            acc:          NnueAcc::new(),
        }
    }

    // Internal helpers — do NOT touch `hash` or `acc` (managed by do_move / startpos)
    fn put(&mut self, sq: Square, piece: Piece) {
        self.piece_bb[piece.color.index()][piece.kind.index()].set(sq);
        self.occ[piece.color.index()].set(sq);
        self.mailbox[sq.index() as usize] = Some(piece);
    }

    fn take(&mut self, sq: Square) -> Option<Piece> {
        let piece = self.mailbox[sq.index() as usize].take()?;
        self.piece_bb[piece.color.index()][piece.kind.index()].unset(sq);
        self.occ[piece.color.index()].unset(sq);
        Some(piece)
    }

    // ---- Public read API ----

    pub fn piece_at(&self, sq: Square) -> Option<Piece> {
        self.mailbox[sq.index() as usize]
    }

    pub fn pieces(&self, color: Color, kind: PieceKind) -> Bitboard {
        self.piece_bb[color.index()][kind.index()]
    }

    pub fn occ_for(&self, color: Color) -> Bitboard {
        self.occ[color.index()]
    }

    pub fn occ(&self) -> Bitboard {
        self.occ[0] | self.occ[1]
    }

    pub fn hand(&self, color: Color) -> &Hand {
        &self.hand[color.index()]
    }

    /// Current Zobrist hash of the position
    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Add one piece of `kind` to `color`'s hand (used by SFEN parser).
    pub(crate) fn add_hand_piece(&mut self, color: Color, kind: PieceKind) {
        self.hand[color.index()].restore(kind);
    }

    /// Place a piece during position setup (used by SFEN parser).
    pub(crate) fn setup_piece(&mut self, sq: Square, piece: Piece) {
        self.put(sq, piece);
    }

    /// Recompute Zobrist hash and NNUE accumulator from current mailbox + hand.
    /// Must be called after building a position via `setup_piece` / `add_hand_piece`.
    pub fn recompute_derived(&mut self) {
        use crate::zobrist;
        use PieceKind::*;

        // NNUE: full refresh (board + hand)
        let snapshot: [Option<(PieceKind, Color)>; 81] = {
            let mut s = [None; 81];
            for i in 0..81 {
                s[i] = self.mailbox[i].map(|p| (p.kind, p.color));
            }
            s
        };
        let hand_counts = hand_counts_array(&self.hand);
        self.acc.refresh(&snapshot, &hand_counts);

        // Zobrist: recompute from scratch to avoid any hand-count bugs
        let mut h = 0u64;
        for i in 0..Square::NUM {
            if let Some(p) = self.mailbox[i] {
                h ^= zobrist::piece_key(Square::from_index(i as u8), p.color, p.kind);
            }
        }
        let hand_kinds = [Fu, Kyou, Kei, Gin, Kin, Kaku, Hisha];
        for c in 0..2 {
            let color = if c == 0 { Color::Black } else { Color::White };
            for &kind in &hand_kinds {
                for n in 1..=self.hand[c].get(kind) {
                    h ^= zobrist::hand_delta(color, kind, n);
                }
            }
        }
        if self.side_to_move == Color::Black {
            h ^= zobrist::side_key();
        }
        self.hash = h;
    }

    /// Parse a SFEN position string into a Board.
    pub fn from_sfen(sfen: &str) -> Result<Self, String> {

        let parts: Vec<&str> = sfen.split_whitespace().collect();
        if parts.len() < 3 {
            return Err(format!("SFEN needs at least 3 fields, got: '{sfen}'"));
        }

        let mut board = Self::empty();

        // --- Board ---
        let ranks: Vec<&str> = parts[0].split('/').collect();
        if ranks.len() != 9 {
            return Err(format!("SFEN board must have 9 ranks, got {}", ranks.len()));
        }

        for (rank_idx, rank_str) in ranks.iter().enumerate() {
            let rank = (rank_idx + 1) as u8; // 1-9
            let mut file = 9u8;              // starts at file 9, steps down to 1
            let mut chars = rank_str.chars().peekable();

            while let Some(c) = chars.next() {
                if c == '+' {
                    // Next character is the promoted piece
                    let next = chars.next()
                        .ok_or_else(|| format!("SFEN: '+' at end of rank {rank}"))?;
                    let color = if next.is_uppercase() { Color::Black } else { Color::White };
                    let base  = sfen_char_to_base_kind(next)
                        .ok_or_else(|| format!("SFEN: unknown piece '{next}'"))?;
                    let kind  = base.promoted();
                    board.setup_piece(Square::from_shogi(file, rank), Piece::new(color, kind));
                    file = file.checked_sub(1)
                        .ok_or_else(|| format!("SFEN: too many pieces in rank {rank}"))?;
                } else if let Some(n) = c.to_digit(10) {
                    file = file.checked_sub(n as u8)
                        .ok_or_else(|| format!("SFEN: digit overflow in rank {rank}"))?;
                } else {
                    let color = if c.is_uppercase() { Color::Black } else { Color::White };
                    let kind  = sfen_char_to_base_kind(c)
                        .ok_or_else(|| format!("SFEN: unknown piece '{c}'"))?;
                    board.setup_piece(Square::from_shogi(file, rank), Piece::new(color, kind));
                    file = file.checked_sub(1)
                        .ok_or_else(|| format!("SFEN: too many pieces in rank {rank}"))?;
                }
            }
        }

        // --- Side to move ---
        board.side_to_move = match parts[1] {
            "b" => Color::Black,
            "w" => Color::White,
            s   => return Err(format!("SFEN: unknown side '{s}'")),
        };

        // --- Hand ---
        if parts[2] != "-" {
            let mut count: u8 = 0;
            for c in parts[2].chars() {
                if let Some(n) = c.to_digit(10) {
                    count = count * 10 + n as u8;
                } else {
                    let color = if c.is_uppercase() { Color::Black } else { Color::White };
                    let kind  = sfen_char_to_base_kind(c)
                        .ok_or_else(|| format!("SFEN: unknown hand piece '{c}'"))?;
                    if kind == PieceKind::Ou {
                        return Err("SFEN: king cannot be in hand".into());
                    }
                    let n = if count == 0 { 1 } else { count };
                    for _ in 0..n {
                        board.add_hand_piece(color, kind);
                    }
                    count = 0;
                }
            }
        }

        // --- Ply (optional 4th field) ---
        if let Some(ply_str) = parts.get(3) {
            if let Ok(ply) = ply_str.parse::<u32>() {
                board.ply = ply.saturating_sub(1); // USI counts from 1
            }
        }

        // Recompute derived state (hash + NNUE accumulator) from scratch
        board.recompute_derived();

        Ok(board)
    }

    /// Rebuild the NNUE accumulator from scratch (call after loading a position).
    pub fn refresh_acc(&mut self) {
        let snapshot: [Option<(PieceKind, Color)>; 81] = {
            let mut s = [None; 81];
            for i in 0..81 {
                s[i] = self.mailbox[i].map(|p| (p.kind, p.color));
            }
            s
        };
        let hand_counts = hand_counts_array(&self.hand);
        self.acc.refresh(&snapshot, &hand_counts);
    }

    // ---- Starting position ----

    /// Standard flat (平手) starting position
    pub fn startpos() -> Self {
        let mut b = Self::empty();

        macro_rules! place {
            ($file:expr, $rank:expr, $color:expr, $kind:expr) => {
                b.put(Square::from_shogi($file, $rank), Piece::new($color, $kind));
            };
        }

        use Color::*;
        use PieceKind::*;

        // White (Gote) pieces — ranks 1-3
        place!(9, 1, White, Kyou);
        place!(8, 1, White, Kei);
        place!(7, 1, White, Gin);
        place!(6, 1, White, Kin);
        place!(5, 1, White, Ou);
        place!(4, 1, White, Kin);
        place!(3, 1, White, Gin);
        place!(2, 1, White, Kei);
        place!(1, 1, White, Kyou);
        place!(8, 2, White, Hisha);
        place!(2, 2, White, Kaku);
        for file in 1u8..=9 {
            place!(file, 3, White, Fu);
        }

        // Black (Sente) pieces — ranks 7-9
        for file in 1u8..=9 {
            place!(file, 7, Black, Fu);
        }
        place!(8, 8, Black, Kaku);
        place!(2, 8, Black, Hisha);
        place!(9, 9, Black, Kyou);
        place!(8, 9, Black, Kei);
        place!(7, 9, Black, Gin);
        place!(6, 9, Black, Kin);
        place!(5, 9, Black, Ou);
        place!(4, 9, Black, Kin);
        place!(3, 9, Black, Gin);
        place!(2, 9, Black, Kei);
        place!(1, 9, Black, Kyou);

        // Compute Zobrist hash and NNUE accumulator from scratch
        let mut h = 0u64;
        for i in 0..Square::NUM {
            if let Some(p) = b.mailbox[i] {
                h ^= zobrist::piece_key(Square::from_index(i as u8), p.color, p.kind);
                b.acc.add_piece(Square::from_index(i as u8), p.kind, p.color);
            }
        }
        h ^= zobrist::side_key(); // Black to move
        b.hash = h;

        b
    }

    // ---- Make / unmake ----

    /// Apply `m` and return a token needed to undo it.
    /// Updates Zobrist hash and NNUE accumulator incrementally.
    pub fn do_move(&mut self, m: Move) -> MoveToken {
        let color     = self.side_to_move;
        let prev_hash = self.hash;

        self.hash ^= zobrist::side_key();

        let token = match m.from {
            None => {
                // Drop: remove from hand, place on board
                let piece     = Piece::new(color, m.piece_kind);
                let old_count = self.hand[color.index()].get(m.piece_kind);

                self.hash ^= zobrist::hand_delta(color, m.piece_kind, old_count);
                self.hand[color.index()].remove(m.piece_kind);

                self.put(m.to, piece);
                self.hash ^= zobrist::piece_key(m.to, color, m.piece_kind);

                // NNUE: threshold feature for old_count turns off (drop: N → N-1)
                self.acc.remove_hand(m.piece_kind, old_count, color);
                // NNUE: piece appears on board
                self.acc.add_piece(m.to, m.piece_kind, color);

                MoveToken {
                    from: None, to: m.to,
                    moved: piece, captured: None, promoted: false,
                    prev_hash,
                }
            }
            Some(from) => {
                let mut moved = self.take(from).expect("no piece at from");
                debug_assert_eq!(moved.color, color);
                debug_assert_eq!(moved.kind,  m.piece_kind);
                self.hash ^= zobrist::piece_key(from, color, moved.kind);

                // NNUE: remove piece from its old square
                self.acc.remove_piece(from, moved.kind, color);

                let captured = self.take(m.to);
                if let Some(cap) = captured {
                    self.hash ^= zobrist::piece_key(m.to, cap.color, cap.kind);
                    let base      = cap.kind.unpromoted();
                    let new_count = self.hand[color.index()].get(base) + 1;
                    self.hash ^= zobrist::hand_delta(color, base, new_count);
                    self.hand[color.index()].add_captured(cap.kind);

                    // NNUE: captured piece leaves the board; threshold feature for new_count turns on
                    self.acc.remove_piece(m.to, cap.kind, cap.color);
                    self.acc.add_hand(base, new_count, color);
                }

                let pre_kind = moved.kind;
                if m.promote {
                    moved.kind = moved.kind.promoted();
                }
                self.put(m.to, moved);
                self.hash ^= zobrist::piece_key(m.to, color, moved.kind);

                // NNUE: piece arrives at its new square (possibly promoted)
                self.acc.add_piece(m.to, moved.kind, color);

                MoveToken {
                    from: Some(from), to: m.to,
                    moved: Piece::new(color, pre_kind),
                    captured, promoted: m.promote,
                    prev_hash,
                }
            }
        };

        self.side_to_move = color.flip();
        self.ply += 1;
        token
    }

    /// Restore position to before `do_move` using inverse NNUE deltas.
    /// No accumulator stack needed — the deltas are symmetric.
    pub fn undo_move(&mut self, token: MoveToken) {
        self.hash         = token.prev_hash;
        self.side_to_move = self.side_to_move.flip();
        self.ply         -= 1;
        let color         = self.side_to_move;

        match token.from {
            None => {
                // Undo drop: remove from board, restore to hand
                self.take(token.to);
                self.hand[color.index()].restore(token.moved.kind);

                // NNUE inverse: piece leaves the board; threshold feature for restored count turns on
                self.acc.remove_piece(token.to, token.moved.kind, color);
                let restored = self.hand[color.index()].get(token.moved.kind);
                self.acc.add_hand(token.moved.kind, restored, color);
            }
            Some(from) => {
                // The piece currently at `to` may be the promoted form
                let kind_at_to = if token.promoted {
                    token.moved.kind.promoted()
                } else {
                    token.moved.kind
                };

                self.take(token.to);

                // NNUE inverse: remove the piece that was at `to`
                self.acc.remove_piece(token.to, kind_at_to, color);

                self.put(from, token.moved); // restore pre-promotion piece

                // NNUE inverse: put back the original piece at `from`
                self.acc.add_piece(from, token.moved.kind, color);

                if let Some(cap) = token.captured {
                    self.put(token.to, cap);
                    let before_remove = self.hand[color.index()].get(cap.kind.unpromoted());
                    self.hand[color.index()].remove(cap.kind.unpromoted());

                    // NNUE inverse: captured piece reappears on board; threshold feature for before_remove turns off
                    self.acc.add_piece(token.to, cap.kind, cap.color);
                    self.acc.remove_hand(cap.kind.unpromoted(), before_remove, color);
                }
            }
        }
    }

    // ---- Null Move (for Null Move Pruning) ----

    /// Apply a null move: pass the turn without moving any piece.
    /// Toggles side_to_move and flips the side-to-move Zobrist key.
    /// Does NOT update ply, acc, or piece bitboards.
    pub fn do_null_move(&mut self) -> NullToken {
        let prev_hash = self.hash;
        self.hash ^= zobrist::side_key();
        self.side_to_move = self.side_to_move.flip();
        NullToken { prev_hash }
    }

    /// Undo a null move, restoring side_to_move and hash.
    pub fn undo_null_move(&mut self, tok: NullToken) {
        self.hash         = tok.prev_hash;
        self.side_to_move = self.side_to_move.flip();
    }
}

/// Opaque token returned by `Board::do_null_move`; passed to `Board::undo_null_move`.
#[derive(Clone, Copy, Debug)]
pub struct NullToken {
    pub(crate) prev_hash: u64,
}
