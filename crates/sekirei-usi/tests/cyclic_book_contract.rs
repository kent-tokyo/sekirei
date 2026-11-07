//! Synthetic process-level contract checks; no matches or strength claims.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

struct Engine {
    child: Child,
    input: ChildStdin,
    output: Receiver<String>,
}

impl Engine {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sekirei"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            child,
            input,
            output,
        }
    }

    fn send(&mut self, command: &str) {
        writeln!(self.input, "{command}").unwrap();
        self.input.flush().unwrap();
    }

    fn until(&self, stop: impl Fn(&str) -> bool) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            let line = self
                .output
                .recv_timeout(Duration::from_secs(10))
                .expect("bounded USI response");
            let done = stop(&line);
            lines.push(line);
            if done {
                return lines;
            }
        }
    }

    fn ready(&mut self) -> Vec<String> {
        self.send("isready");
        self.until(|line| line == "readyok")
    }

    #[cfg(feature = "cyclic-book")]
    fn go(&mut self) -> Vec<String> {
        self.send("go depth 1");
        self.until(|line| line.starts_with("bestmove "))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Tests never leave engines alive, including after a failed assertion.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(not(feature = "cyclic-book"))]
#[test]
fn default_binary_has_no_cyclic_book_surface_or_file_access() {
    let mut engine = Engine::spawn();
    engine.send("usi");
    let options = engine.until(|line| line == "usiok");
    assert!(options.iter().all(|line| !line.contains("CyclicBook")));
    engine.send("setoption name CyclicBookFile value /not/a/cyclic-book.json");
    engine.send("setoption name UseCyclicBook value true");
    assert!(
        engine
            .ready()
            .iter()
            .all(|line| !line.contains("cyclic book"))
    );
}

#[cfg(feature = "cyclic-book")]
mod enabled {
    use super::*;
    use sekirei_core::{
        board::Board,
        movegen::{generate_legal_moves, is_in_check},
        sfen::{board_to_sfen, move_from_usi, move_to_usi},
    };
    use serde_json::{Value, json};
    use tempfile::NamedTempFile;

    const ORIGIN: &str = "4k4/9/4P4/9/9/9/9/9/4K4 b G 1";
    const CYCLE: &str = "5i4i 5a4a 4i5i 4a5a";

    fn key(board: &Board) -> String {
        board_to_sfen(board)
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn fixture(include_cycle: bool, include_mate: bool) -> Value {
        let root = Board::from_sfen(ORIGIN).unwrap();
        let mut boards = vec![root.clone()];
        if include_cycle {
            let mut board = root.clone();
            for action in CYCLE.split_whitespace() {
                board.do_move(move_from_usi(action, &board).unwrap());
                if boards.iter().all(|stored| key(stored) != key(&board)) {
                    boards.push(board.clone());
                }
            }
        }
        if include_mate {
            let mut after = root.clone();
            after.do_move(move_from_usi("G*5b", &root).unwrap());
            boards.push(after);
        }
        let nodes = boards.iter().map(|board| {
            let successors = generate_legal_moves(&mut board.clone()).into_iter().map(|mv| {
                let mut after = board.clone(); after.do_move(mv);
                json!({"action": move_to_usi(mv), "target": boards.iter().position(|stored| key(stored) == key(&after)), "gave_check": is_in_check(&after, after.side_to_move)})
            }).collect::<Vec<_>>();
            json!({"sfen": board_to_sfen(board), "successors": successors})
        }).collect::<Vec<_>>();
        json!({"schema": "sekirei.cyclic-book", "version": 1, "rules": "exact-history-checkmate-only-v1", "producer": "synthetic-process-contract", "source": "synthetic", "verification": {"max_depth": 4, "node_limit": 1000}, "nodes": nodes})
    }

    fn file(value: &Value) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        serde_json::to_writer(&mut file, value).unwrap();
        file.flush().unwrap();
        file
    }

    fn load(engine: &mut Engine, file: &NamedTempFile) -> Vec<String> {
        engine.send(&format!(
            "setoption name CyclicBookFile value {}",
            file.path().display()
        ));
        engine.send("setoption name UseCyclicBook value true");
        engine.ready()
    }

    fn assert_fallback(lines: &[String], reason: &str) {
        assert!(
            lines
                .iter()
                .any(|line| line == &format!("info string cyclic book fallback {reason}")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("cyclic book verified mate")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("bestmove ")),
            "{lines:?}"
        );
    }

    #[test]
    fn feature_is_default_off_and_missing_artifact_falls_back() {
        let mut engine = Engine::spawn();
        engine.send("usi");
        assert!(
            engine
                .until(|line| line == "usiok")
                .iter()
                .any(|line| line == "option name UseCyclicBook type check default false")
        );
        engine.send("setoption name CyclicBookFile value /not/a/cyclic-book.json");
        assert!(
            engine
                .ready()
                .iter()
                .all(|line| !line.contains("cyclic book"))
        );
        engine.send("setoption name UseCyclicBook value true");
        let ready = engine.ready();
        assert!(
            ready
                .iter()
                .any(|line| line.contains("priority over UseBook; miss uses normal search"))
        );
        assert!(
            ready
                .iter()
                .any(|line| line.contains("cyclic book load failed"))
        );
        engine.send("position startpos");
        assert_fallback(&engine.go(), "unavailable");
    }

    #[test]
    fn supplied_position_history_produces_only_a_legally_verified_mate_move() {
        let fixture = file(&fixture(false, true));
        let mut engine = Engine::spawn();
        assert!(
            load(&mut engine, &fixture)
                .iter()
                .any(|line| line.contains("cyclic book loaded (schema=1"))
        );
        engine.send(&format!("position sfen {ORIGIN}"));
        let result = engine.go();
        assert!(
            result
                .iter()
                .any(|line| line.contains("cyclic book verified mate")),
            "{result:?}"
        );
        assert_eq!(result.last().unwrap(), "bestmove G*5b");
        let board = Board::from_sfen(ORIGIN).unwrap();
        assert!(move_from_usi("G*5b", &board).is_ok());
    }

    #[test]
    fn missing_unrecorded_midgame_and_invalidated_histories_use_search() {
        let fixture = file(&fixture(false, true));
        let mut engine = Engine::spawn();
        load(&mut engine, &fixture);
        assert_fallback(&engine.go(), "history_unavailable");
        engine.send(&format!("position sfen {ORIGIN} moves {CYCLE}"));
        assert_fallback(&engine.go(), "history_not_represented");
        engine.send("position sfen 4k4/9/4P4/9/9/9/9/9/4K4 b G 5");
        assert_fallback(&engine.go(), "history_unavailable");
        engine.send(&format!("position sfen {ORIGIN}"));
        engine.send("position not-a-position");
        assert_fallback(&engine.go(), "history_unavailable");
        engine.send("usinewgame");
        assert_fallback(&engine.go(), "history_unavailable");
    }

    #[test]
    fn unknown_and_actual_fourfold_terminal_never_become_book_moves() {
        let mut engine = Engine::spawn();
        let unknown = file(&fixture(false, false));
        load(&mut engine, &unknown);
        engine.send(&format!("position sfen {ORIGIN}"));
        assert_fallback(&engine.go(), "unknown");
        let cycle = file(&fixture(true, true));
        load(&mut engine, &cycle);
        engine.send(&format!(
            "position sfen {ORIGIN} moves {CYCLE} {CYCLE} {CYCLE}"
        ));
        assert_fallback(&engine.go(), "non_mate_outcome");
    }

    #[test]
    fn incompatible_artifact_is_reported_and_does_not_keep_a_previous_book() {
        let mut engine = Engine::spawn();
        let good = file(&fixture(false, true));
        load(&mut engine, &good);
        let mut bad = fixture(false, true);
        bad["version"] = json!(2);
        let bad = file(&bad);
        assert!(
            load(&mut engine, &bad)
                .iter()
                .any(|line| line.contains("cyclic book load failed"))
        );
        engine.send(&format!("position sfen {ORIGIN}"));
        assert_fallback(&engine.go(), "unavailable");
    }

    #[cfg(feature = "opening-book")]
    #[test]
    fn cyclic_priority_miss_search_and_disabling_restore_the_statistical_book() {
        let mut statistical = NamedTempFile::new().unwrap();
        writeln!(statistical, "{}", json!({"state": ORIGIN, "actions": [{"action": "5i4i", "count": 10, "weighted_count": 10.0, "success_rate": 0.5, "mean_score": 0.5, "prior": 1.0, "confidence": 1.0}]})).unwrap();
        statistical.flush().unwrap();
        let mut engine = Engine::spawn();
        engine.send(&format!(
            "setoption name BookFile value {}",
            statistical.path().display()
        ));
        engine.send("setoption name UseBook value true");
        let good = file(&fixture(false, true));
        load(&mut engine, &good);
        engine.send(&format!("position sfen {ORIGIN}"));
        let proven = engine.go();
        assert!(
            proven
                .iter()
                .any(|line| line.contains("cyclic book verified mate"))
        );
        assert_eq!(proven.last().unwrap(), "bestmove G*5b");
        let unknown = file(&fixture(false, false));
        load(&mut engine, &unknown);
        engine.send(&format!("position sfen {ORIGIN}"));
        let fallback = engine.go();
        assert_fallback(&fallback, "unknown");
        assert!(fallback.iter().all(|line| line != "info string book move"));
        engine.send("setoption name UseCyclicBook value false");
        engine.ready();
        engine.send(&format!("position sfen {ORIGIN}"));
        let prior = engine.go();
        assert!(
            prior.iter().any(|line| line == "info string book move"),
            "{prior:?}"
        );
        assert_eq!(prior.last().unwrap(), "bestmove 5i4i");
    }
}
