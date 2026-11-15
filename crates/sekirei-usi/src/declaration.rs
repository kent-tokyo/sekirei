//! USI clock guard for a board-eligible entering-king declaration.

use sekirei_core::color::Color;

/// An explicit zero clock (including the Fischer increment) cannot be used
/// to claim a declaration win.
/// Analysis commands with no clock leave time adjudication to the caller.
pub(crate) fn has_declaration_time(args: &str, side: Color) -> bool {
    let mut side_time = None;
    let mut side_increment = None;
    let mut byoyomi = None;
    let mut movetime = None;
    let mut explicit_clock = false;
    let tokens: Vec<&str> = args.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        let value = || {
            tokens
                .get(index + 1)
                .and_then(|text| text.parse::<u64>().ok())
        };
        match *token {
            "btime" => {
                explicit_clock = true;
                let Some(value) = value() else {
                    return false;
                };
                if side == Color::Black {
                    side_time = Some(value);
                }
            }
            "wtime" => {
                explicit_clock = true;
                let Some(value) = value() else {
                    return false;
                };
                if side == Color::White {
                    side_time = Some(value);
                }
            }
            "binc" => {
                explicit_clock = true;
                let Some(value) = value() else {
                    return false;
                };
                if side == Color::Black {
                    side_increment = Some(value);
                }
            }
            "winc" => {
                explicit_clock = true;
                let Some(value) = value() else {
                    return false;
                };
                if side == Color::White {
                    side_increment = Some(value);
                }
            }
            "byoyomi" => {
                explicit_clock = true;
                byoyomi = value();
                if byoyomi.is_none() {
                    return false;
                }
            }
            "movetime" => {
                explicit_clock = true;
                movetime = value();
                if movetime.is_none() {
                    return false;
                }
            }
            _ => {}
        }
    }
    if !explicit_clock {
        return true;
    }
    side_time.unwrap_or(0) > 0
        || side_increment.unwrap_or(0) > 0
        || byoyomi.unwrap_or(0) > 0
        || movetime.unwrap_or(0) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_zero_clock_is_not_remaining_time() {
        assert!(has_declaration_time(
            "btime 0 wtime 1000 binc 2000",
            Color::Black
        ));
        assert!(!has_declaration_time(
            "btime 0 wtime 1000 binc 0",
            Color::Black
        ));
        assert!(has_declaration_time(
            "btime 0 wtime 1000 byoyomi 1000",
            Color::Black
        ));
        assert!(has_declaration_time("btime 0 wtime 1000", Color::White));
        assert!(has_declaration_time("depth 1", Color::Black));
        assert!(!has_declaration_time("movetime 0", Color::White));
        assert!(!has_declaration_time("wtime 1000", Color::Black));
        assert!(!has_declaration_time("btime", Color::Black));
        assert!(!has_declaration_time("btime malformed", Color::Black));
        assert!(!has_declaration_time("binc 0", Color::Black));
    }
}
