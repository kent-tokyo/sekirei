//! Parsing and budgeting for the USI `go` command.

use std::time::Duration;

use sekirei_core::{color::Color, search::SearchConfig};

pub(crate) fn parse(
    args: &str,
    side: Color,
    overhead_ms: u64,
    pondering: bool,
    multi_pv: u32,
    increment_use_percent: u64,
) -> SearchConfig {
    let mut btime: Option<u64> = None;
    let mut wtime: Option<u64> = None;
    let mut byoyomi: Option<u64> = None;
    let mut binc: Option<u64> = None;
    let mut winc: Option<u64> = None;
    let mut movestogo: Option<u64> = None;
    let mut movetime: Option<u64> = None;
    let mut depth: Option<u32> = None;
    let mut nodes: Option<u64> = None;
    let mut infinite = false;

    let tokens: Vec<&str> = args.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "btime" => {
                i += 1;
                btime = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "wtime" => {
                i += 1;
                wtime = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "byoyomi" => {
                i += 1;
                byoyomi = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "binc" => {
                i += 1;
                binc = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "winc" => {
                i += 1;
                winc = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "movestogo" => {
                i += 1;
                movestogo = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "movetime" => {
                i += 1;
                movetime = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "depth" => {
                i += 1;
                depth = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "nodes" => {
                i += 1;
                nodes = tokens.get(i).and_then(|s| s.parse().ok());
            }
            "infinite" => infinite = true,
            _ => {}
        }
        i += 1;
    }

    let has_clock = btime.is_some() || wtime.is_some() || byoyomi.is_some() || movetime.is_some();
    let (time_limit, soft_limit) = if infinite || pondering {
        (None, None)
    } else if let Some(mt) = movetime {
        (
            Some(Duration::from_millis(
                mt.saturating_sub(overhead_ms).max(50),
            )),
            None,
        )
    } else if depth.is_some() && !has_clock {
        (None, None)
    } else if has_clock {
        let our_time = match side {
            Color::Black => btime.unwrap_or(0),
            Color::White => wtime.unwrap_or(0),
        };
        let increment = match side {
            Color::Black => binc.unwrap_or(0),
            Color::White => winc.unwrap_or(0),
        };
        let byo_ms = byoyomi.unwrap_or(0);
        let effective_time = our_time.saturating_add(increment);
        let moves_left = movestogo.unwrap_or(30).max(1);
        let from_main = if increment_use_percent > 0 {
            (our_time / moves_left)
                .saturating_add(increment.saturating_mul(increment_use_percent) / 100)
        } else {
            effective_time / moves_left
        };
        let from_byo = byo_ms.saturating_mul(13) / 20;
        let panic = our_time < 5_000 && byo_ms > 0;
        let base = if panic {
            byo_ms
        } else {
            from_main.max(from_byo)
        };
        let base = base.saturating_sub(overhead_ms).max(50);
        let byo_safe = byo_ms.saturating_sub(overhead_ms).max(50);
        let hard_ms = if byo_ms > 0 {
            (base.saturating_mul(3) / 2).min(byo_safe)
        } else {
            base.saturating_mul(3) / 2
        }
        .max(50);
        let hard_ms = if increment_use_percent > 0 && byo_ms == 0 {
            hard_ms.min(effective_time.saturating_sub(overhead_ms).max(20))
        } else {
            hard_ms
        };
        let soft_ms = base.saturating_mul(4) / 5;
        (
            Some(Duration::from_millis(hard_ms)),
            (!panic).then(|| Duration::from_millis(soft_ms)),
        )
    } else {
        (None, None)
    };

    SearchConfig {
        max_depth: depth.unwrap_or(50),
        time_limit,
        node_limit: nodes,
        soft_limit,
        multi_pv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_increment_percentage_restores_the_combined_clock_budget() {
        let config = parse(
            "btime 60000 wtime 60000 binc 3000 winc 3000 movestogo 30",
            Color::Black,
            0,
            false,
            1,
            0,
        );

        assert_eq!(config.time_limit, Some(Duration::from_millis(3150)));
        assert_eq!(config.soft_limit, Some(Duration::from_millis(1680)));
    }
}
