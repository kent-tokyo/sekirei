//! Scan game records for mates the check-only df-pn solver finds.
//!
//! ```text
//! mate_scan <node_limit> <max_ply> <game files...>
//! ```
//!
//! For every position with Sekirei to move, report a forced mate the solver
//! proves that Sekirei did not play (a missed mate). For the first position
//! in a game where the opponent has a proven forced mate, report whether the
//! preceding Sekirei move was avoidable: some other legal move after which the
//! solver proves no mate within the same budget.
use sekirei_core::mate::solve_mate;
use sekirei_core::movegen::MoveBuffer;
use sekirei_core::sfen::{move_from_usi, move_to_usi, parse_position_cmd};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let node_limit: u64 = args[0].parse().unwrap();
    let max_ply: u32 = args[1].parse().unwrap();
    let mut games = 0;
    let mut sekirei_losses = 0;
    let mut missed = 0;
    let mut missed_in_lost = 0;
    let mut walked_in = 0;
    let mut walked_in_avoidable = 0;
    let mut walked_in_lost = 0;
    let mut total_positions = 0u64;
    let mut total_nodes = 0u64;
    for path in &args[2..] {
        let text = std::fs::read_to_string(path).expect("game file");
        let mut sekirei_black = None;
        let mut sekirei_is_engine1 = None;
        let mut result = "";
        let mut position = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("# Engine1: ") {
                if rest.contains("Sekirei") {
                    sekirei_black = Some(rest.contains("(Black)"));
                    sekirei_is_engine1 = Some(true);
                }
            } else if let Some(rest) = line.strip_prefix("# Engine2: ") {
                if rest.contains("Sekirei") {
                    sekirei_black = Some(rest.contains("(Black)"));
                    sekirei_is_engine1 = Some(false);
                }
            } else if let Some(rest) = line.strip_prefix("# Result: ") {
                result = rest;
            } else if let Some(body) = line.strip_prefix("position ") {
                position = Some(body.to_string());
            }
        }
        let (Some(body), Some(sekirei_black), Some(sekirei_is_engine1)) =
            (position, sekirei_black, sekirei_is_engine1)
        else {
            eprintln!("SKIP {path}: expected one Sekirei engine with a color");
            continue;
        };
        let engine1_win = result.starts_with("Engine1 Win");
        let engine2_win = result.starts_with("Engine2 Win");
        let sekirei_won = if sekirei_is_engine1 {
            engine1_win
        } else {
            engine2_win
        };
        let sekirei_lost = if sekirei_is_engine1 {
            engine2_win
        } else {
            engine1_win
        };
        games += 1;
        if sekirei_lost {
            sekirei_losses += 1;
        }
        let (setup, moves) = match body.split_once(" moves ") {
            Some((s, m)) => (s, m.split_whitespace().collect::<Vec<_>>()),
            None => (body.as_str(), Vec::new()),
        };
        let mut board = parse_position_cmd(setup).expect("sfen");
        let plies = moves.len();
        let mut reported_walk_in = false;
        for (ply, mv) in moves.iter().enumerate() {
            let sekirei_to_move =
                (board.side_to_move == sekirei_core::color::Color::Black) == sekirei_black;
            let m = move_from_usi(mv, &board).expect("move");
            total_positions += 1;
            let r = solve_mate(&board, node_limit, max_ply, None);
            total_nodes += r.nodes;
            if sekirei_to_move {
                if let Some(mate) = r.mate_move.filter(|&mate| mate != m) {
                    // Whether the played move also mated cannot be checked with an
                    // attacker-only solver; approximate it by whether the game ended
                    // within max_ply plies with a Sekirei win.
                    let remaining = plies - ply;
                    let converted = sekirei_won && remaining <= max_ply as usize + 1;
                    if !converted {
                        missed += 1;
                        if sekirei_lost {
                            missed_in_lost += 1;
                        }
                        println!(
                            "MISSED {path} ply {ply} remaining {remaining} result {result} solver {} played {mv} nodes {}",
                            move_to_usi(mate),
                            r.nodes
                        );
                    }
                }
            } else if r.mate_move.is_some() && !reported_walk_in && ply > 0 {
                reported_walk_in = true;
                walked_in += 1;
                if sekirei_lost {
                    walked_in_lost += 1;
                }
                // Undo the previous Sekirei move and look for an alternative
                // that leaves the opponent without a proven mate.
                let mut prev = parse_position_cmd(setup).expect("sfen");
                for pm in &moves[..ply - 1] {
                    let pmv = move_from_usi(pm, &prev).expect("move");
                    prev.do_move(pmv);
                }
                let legal = MoveBuffer::legal(&mut prev);
                let mut safe = Vec::new();
                for &alt in legal.as_slice() {
                    let tok = prev.do_move(alt);
                    let opp = solve_mate(&prev, node_limit, max_ply, None);
                    prev.undo_move(tok);
                    if opp.mate_move.is_none() {
                        safe.push(move_to_usi(alt));
                    }
                }
                let avoidable = !safe.is_empty();
                if avoidable {
                    walked_in_avoidable += 1;
                }
                println!(
                    "WALKIN {path} ply {} remaining {} result {result} played {} safe_alternatives {} (e.g. {})",
                    ply - 1,
                    plies - ply + 1,
                    moves[ply - 1],
                    safe.len(),
                    safe.first().cloned().unwrap_or_default()
                );
            }
            board.do_move(m);
        }
    }
    println!(
        "games {games} sekirei_losses {sekirei_losses} positions {total_positions} mean solver nodes {}",
        total_nodes / total_positions.max(1)
    );
    println!("missed mates {missed} (in lost games {missed_in_lost})");
    println!(
        "walked into a proven mate {walked_in} (avoidable {walked_in_avoidable}, in lost games {walked_in_lost})"
    );
}
