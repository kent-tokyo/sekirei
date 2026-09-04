use std::io::{self, Write};

use sekirei_core::{
    board::Board,
    movegen::is_in_check,
    nnue::weights_active,
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
    label_threshold_cp: i32,
    out: &mut W,
) -> io::Result<()> {
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
        let model_id = if weights_active() { "nnue" } else { "material" };

        for &depth in depths {
            let config = SearchConfig {
                max_depth: depth,
                time_limit: None,
                node_limit: None,
                soft_limit: None,
                multi_pv: 1,
            };
            let info = searcher.search(&mut board, config);
            let score = info.score as f64 / 600.0;
            let label = if info.score > label_threshold_cp {
                "adv"
            } else if info.score < -label_threshold_cp {
                "disadv"
            } else {
                "equal"
            };
            writeln!(
                out,
                r#"{{"sample_id":{},"label":"{}","score":{:.4},"evaluator_id":"sekirei-search","budget":{},"model_id":"{}"}}"#,
                json_string(&sfen),
                label,
                score,
                depth,
                model_id
            )?;
        }

        board.do_move(mv);
    }
    Ok(())
}

fn json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
