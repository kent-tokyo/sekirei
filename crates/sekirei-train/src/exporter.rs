use std::io::{self, Write};

use sekirei_core::{
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
    let mut board = game.initial_board.clone();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csa::parse_csa;

    fn short_game() -> CsaGame {
        parse_csa("+7776FU\n-3334FU\n+2726FU\n%TORYO\n").expect("valid CSA game")
    }

    #[test]
    fn export_game_writes_search_labels_for_selected_positions() {
        let game = short_game();
        let mut output = Vec::new();
        export_game(&game, 2, false, 0, &[1, 2], 10_000, &mut output).expect("export succeeds");
        let text = String::from_utf8(output).expect("UTF-8 JSONL");
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 4, "plies 0 and 2 at two depths each");
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
            assert_eq!(value["label"], "equal");
            assert_eq!(value["evaluator_id"], "sekirei-search");
            assert!(matches!(value["budget"].as_u64(), Some(1 | 2)));
            assert!(value["sample_id"].as_str().is_some());
        }
    }

    #[test]
    fn export_game_honours_minimum_ply_and_empty_depths() {
        let game = short_game();
        let mut output = Vec::new();
        export_game(&game, 1, false, game.moves.len(), &[1], 0, &mut output)
            .expect("skipped export succeeds");
        assert!(output.is_empty());

        export_game(&game, 1, false, 0, &[], 0, &mut output).expect("empty depth list succeeds");
        assert!(output.is_empty());
    }

    #[test]
    fn json_string_escapes_quotes_and_backslashes() {
        assert_eq!(json_string("a\\b\"c"), "\"a\\\\b\\\"c\"");
    }
}
