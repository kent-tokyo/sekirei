use std::io::Write;

use sekirei_core::{
    board::Board,
    movegen::is_in_check,
    search::{SearchConfig, Searcher},
    sfen::board_to_sfen,
    tt::Tt,
};

use crate::csa::CsaGame;

pub fn export_game<W: Write>(
    game: &CsaGame,
    sample_every: usize,
    quiet: bool,
    min_ply: usize,
    depths: &[u32],
    out: &mut W,
) {
    let searcher = Searcher::new(Tt::new(4));
    let mut board = Board::startpos();

    for (ply, &mv) in game.moves.iter().enumerate() {
        if ply < min_ply || ply % sample_every != 0 {
            board.do_move(mv);
            continue;
        }
        if quiet && (is_in_check(&board, board.side_to_move) || board.piece_at(mv.to).is_some()) {
            board.do_move(mv);
            continue;
        }

        let sfen = board_to_sfen(&board);

        for &depth in depths {
            let config = SearchConfig {
                max_depth: depth,
                time_limit: None,
            };
            let info = searcher.search(&mut board, config);
            let score = info.score as f64 / 600.0;
            let label = if info.score > 50 {
                "adv"
            } else if info.score < -50 {
                "disadv"
            } else {
                "draw"
            };
            let _ = writeln!(
                out,
                r#"{{"sample_id":{},"label":"{}","score":{:.4},"evaluator_id":"sekirei","budget":{}}}"#,
                json_string(&sfen),
                label,
                score,
                depth
            );
        }

        board.do_move(mv);
    }
}

fn json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
