//! Statistical opening book built from real game records, via `lineprior`
//! (https://github.com/kent-tokyo/lineprior) -- a domain-agnostic action-prior
//! library. `lineprior` never sees a shogi concept: this module's only job
//! is walking CSA games into `lineprior::Observation`s (state = SFEN, action
//! = USI move) and handing them to `lineprior::build_prior_book`, which
//! does the actual smoothing/confidence-scoring/ranking.
//!
//! Unlike the NNUE eval (which never saw an opening-phase position until
//! this session's fix — see the ply<20 gap documented in the training
//! pipeline), this is built directly from what strong players actually
//! played, weighted by how often and how successfully. It's a short-term
//! complement to opening-phase training, not a replacement: a book only
//! helps for known positions and falls back to search the moment the
//! opponent deviates, so the eval still needs to carry its own weight past
//! the book's coverage.

use std::io::Write;

use lineprior::{BuildConfig, Observation, Outcome};
use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::sfen::{board_to_sfen, move_to_usi};

use crate::csa::{CsaGame, GameResult};

/// Replays every game up to `max_ply`, emitting one `Observation` per ply,
/// then hands the whole set to `lineprior::build_prior_book` for smoothing/
/// ranking. Games are assumed already rating-filtered by the caller (reuses
/// `--min-rate`, same as `--export` mode, rather than adding a separate
/// book-specific flag).
pub fn build_book(games: &[CsaGame], max_ply: usize, min_count: u64, out: &mut impl Write) {
    let mut observations = Vec::new();

    for (game_idx, game) in games.iter().enumerate() {
        let mut board = Board::startpos();
        for (ply, &mv) in game.moves.iter().enumerate() {
            if ply >= max_ply {
                break;
            }
            let mover = if ply % 2 == 0 {
                Color::Black
            } else {
                Color::White
            };
            let (outcome, score) = match (mover, game.result) {
                (Color::Black, GameResult::BlackWin) | (Color::White, GameResult::WhiteWin) => {
                    (Outcome::Success, Some(1.0))
                }
                (Color::Black, GameResult::WhiteWin) | (Color::White, GameResult::BlackWin) => {
                    (Outcome::Failure, Some(0.0))
                }
                (_, GameResult::Draw) => (Outcome::Draw, Some(0.5)),
                (_, GameResult::Unknown) => (Outcome::Unknown, None),
            };

            observations.push(Observation {
                sequence_id: format!("game{game_idx:06}"),
                step: ply as u32,
                state: board_to_sfen(&board),
                action: move_to_usi(mv),
                outcome,
                score,
                weight: 1.0,
                tags: vec!["opening".to_string()],
            });

            board.do_move(mv);
        }
    }

    let seen = observations.len();
    let config = BuildConfig {
        min_count,
        max_actions_per_state: Some(6),
        smoothing_alpha: 5.0,
        ..Default::default()
    };
    let book = match lineprior::build_prior_book(&observations, &config) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("book: lineprior build failed: {e}");
            return;
        }
    };

    let kept = book.entries.len();
    if let Err(e) = lineprior::save_prior_book(&book, &mut *out) {
        eprintln!("book: failed to write prior book: {e}");
        return;
    }
    eprintln!(
        "book: {kept} positions kept (of {seen} observations, min_count={min_count}, max_ply={max_ply})"
    );
}
