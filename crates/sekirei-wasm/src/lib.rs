//! Browser-safe, stateless bindings for Sekirei's rules and bounded search.
//!
//! The exported API accepts complete SFEN strings and returns either ordinary
//! JavaScript values or an error object with `code` and `message` fields. It
//! deliberately exposes no filesystem, external-weight, or parallel-search
//! controls.

use js_sys::{Array, Object, Reflect};
use sekirei_core::board::Board;
use sekirei_core::movegen::generate_legal_moves;
use sekirei_core::search::{SearchConfig, Searcher};
use sekirei_core::sfen::{STARTPOS_SFEN, board_to_sfen, move_from_usi, move_to_usi};
use sekirei_core::tt::Tt;
use wasm_bindgen::prelude::*;

const MAX_SFEN_BYTES: usize = 512;
const MAX_MOVE_BYTES: usize = 8;
const MAX_SEARCH_DEPTH: u32 = 8;
const MAX_SEARCH_NODES: u32 = 100_000;
const SEARCH_TT_MIB: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ApiError {
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn into_js(self) -> JsValue {
        let error = Object::new();
        let _ = Reflect::set(
            &error,
            &JsValue::from_str("code"),
            &JsValue::from_str(self.code),
        );
        let _ = Reflect::set(
            &error,
            &JsValue::from_str("message"),
            &JsValue::from_str(&self.message),
        );
        error.into()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SearchResult {
    best_move: String,
    score: i32,
    depth: u32,
    nodes: u32,
    used_fallback: bool,
}

fn parse_board(sfen: &str) -> Result<Board, ApiError> {
    if sfen.len() > MAX_SFEN_BYTES {
        return Err(ApiError::new(
            "input_too_large",
            format!("SFEN must be at most {MAX_SFEN_BYTES} bytes"),
        ));
    }
    Board::from_sfen(sfen).map_err(|message| ApiError::new("invalid_sfen", message))
}

fn legal_move_strings(sfen: &str) -> Result<Vec<String>, ApiError> {
    let mut board = parse_board(sfen)?;
    let mut moves = generate_legal_moves(&mut board)
        .into_iter()
        .map(move_to_usi)
        .collect::<Vec<_>>();
    moves.sort_unstable();
    Ok(moves)
}

fn apply_move_impl(sfen: &str, usi_move: &str) -> Result<String, ApiError> {
    if usi_move.len() > MAX_MOVE_BYTES {
        return Err(ApiError::new(
            "input_too_large",
            format!("USI move must be at most {MAX_MOVE_BYTES} bytes"),
        ));
    }
    let mut board = parse_board(sfen)?;
    let mv = move_from_usi(usi_move, &board)
        .map_err(|message| ApiError::new("invalid_move", message))?;
    board.do_move(mv);
    Ok(board_to_sfen(&board))
}

fn validate_search_limits(max_depth: u32, max_nodes: u32) -> Result<(), ApiError> {
    if !(1..=MAX_SEARCH_DEPTH).contains(&max_depth) {
        return Err(ApiError::new(
            "invalid_search_limit",
            format!("maxDepth must be in 1..={MAX_SEARCH_DEPTH}"),
        ));
    }
    if !(1..=MAX_SEARCH_NODES).contains(&max_nodes) {
        return Err(ApiError::new(
            "invalid_search_limit",
            format!("maxNodes must be in 1..={MAX_SEARCH_NODES}"),
        ));
    }
    Ok(())
}

fn computer_move_impl(
    sfen: &str,
    max_depth: u32,
    max_nodes: u32,
) -> Result<SearchResult, ApiError> {
    validate_search_limits(max_depth, max_nodes)?;
    let mut board = parse_board(sfen)?;
    let mut legal = generate_legal_moves(&mut board);
    if legal.is_empty() {
        return Err(ApiError::new(
            "terminal_position",
            "the position has no legal moves",
        ));
    }
    legal.sort_unstable_by_key(|mv| move_to_usi(*mv));

    let mut searcher = Searcher::new(Tt::new_for_evaluation(SEARCH_TT_MIB, false));
    searcher.set_ybw_split(false);
    let info = searcher.search(
        &mut board,
        SearchConfig {
            max_depth,
            node_limit: Some(u64::from(max_nodes)),
            time_limit: None,
            soft_limit: None,
            multi_pv: 1,
        },
    );
    let (best_move, used_fallback) = match info.best_move {
        Some(best_move) => (best_move, false),
        None => (legal[0], true),
    };
    Ok(SearchResult {
        best_move: move_to_usi(best_move),
        score: info.score,
        depth: info.depth,
        nodes: info.nodes.min(u64::from(u32::MAX)) as u32,
        used_fallback,
    })
}

/// Result returned by [`computer_move`].
#[wasm_bindgen]
pub struct ComputerMove {
    result: SearchResult,
}

#[wasm_bindgen]
impl ComputerMove {
    /// Legal USI move selected by the bounded search.
    #[wasm_bindgen(getter, js_name = bestMove)]
    pub fn best_move(&self) -> String {
        self.result.best_move.clone()
    }

    /// Search score from the current side's perspective.
    #[wasm_bindgen(getter)]
    pub fn score(&self) -> i32 {
        self.result.score
    }

    /// Deepest completed iterative-deepening depth.
    #[wasm_bindgen(getter)]
    pub fn depth(&self) -> u32 {
        self.result.depth
    }

    /// Nodes observed by the bounded search.
    #[wasm_bindgen(getter)]
    pub fn nodes(&self) -> u32 {
        self.result.nodes
    }

    /// Whether the node budget expired before search produced a move.
    #[wasm_bindgen(getter, js_name = usedFallback)]
    pub fn used_fallback(&self) -> bool {
        self.result.used_fallback
    }
}

/// Return the standard shogi starting position as SFEN.
#[wasm_bindgen(js_name = startSfen)]
pub fn start_sfen() -> String {
    STARTPOS_SFEN.to_owned()
}

/// Return every legal move for `sfen`, sorted in USI notation.
#[wasm_bindgen(js_name = legalMoves)]
pub fn legal_moves(sfen: &str) -> Result<Array, JsValue> {
    let array = Array::new();
    for mv in legal_move_strings(sfen).map_err(ApiError::into_js)? {
        array.push(&JsValue::from_str(&mv));
    }
    Ok(array)
}

/// Validate and apply one USI move, returning the resulting SFEN.
#[wasm_bindgen(js_name = applyMove)]
pub fn apply_move(sfen: &str, usi_move: &str) -> Result<String, JsValue> {
    apply_move_impl(sfen, usi_move).map_err(ApiError::into_js)
}

/// Select one legal move using material evaluation and bounded sequential search.
///
/// `max_depth` must be in `1..=8`; `max_nodes` must be in `1..=100000`.
/// No external evaluation weights, filesystem access, or worker threads are
/// exposed by this package.
#[wasm_bindgen(js_name = computerMove)]
pub fn computer_move(sfen: &str, max_depth: u32, max_nodes: u32) -> Result<ComputerMove, JsValue> {
    computer_move_impl(sfen, max_depth, max_nodes)
        .map(|result| ComputerMove { result })
        .map_err(ApiError::into_js)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_position_has_thirty_legal_moves() {
        let moves = legal_move_strings(STARTPOS_SFEN).unwrap();
        assert_eq!(moves.len(), 30);
        assert!(moves.contains(&"7g7f".to_owned()));
    }

    #[test]
    fn apply_move_round_trips_through_sfen_and_usi() {
        let next = apply_move_impl(STARTPOS_SFEN, "7g7f").unwrap();
        let reparsed = parse_board(&next).unwrap();
        assert_eq!(board_to_sfen(&reparsed), next);
        assert!(
            legal_move_strings(&next)
                .unwrap()
                .contains(&"3c3d".to_owned())
        );
    }

    #[test]
    fn malformed_inputs_are_structured_errors() {
        let sfen_error = legal_move_strings("not sfen").unwrap_err();
        assert_eq!(sfen_error.code, "invalid_sfen");

        let move_error = apply_move_impl(STARTPOS_SFEN, "7g7z").unwrap_err();
        assert_eq!(move_error.code, "invalid_move");
    }

    #[test]
    fn search_limits_are_enforced() {
        assert_eq!(
            computer_move_impl(STARTPOS_SFEN, 0, 1_000)
                .unwrap_err()
                .code,
            "invalid_search_limit"
        );
        assert_eq!(
            computer_move_impl(STARTPOS_SFEN, 1, MAX_SEARCH_NODES + 1)
                .unwrap_err()
                .code,
            "invalid_search_limit"
        );
    }

    #[test]
    fn bounded_search_returns_a_legal_move() {
        let result = computer_move_impl(STARTPOS_SFEN, 1, 10_000).unwrap();
        assert!(
            legal_move_strings(STARTPOS_SFEN)
                .unwrap()
                .contains(&result.best_move)
        );
        assert!(result.nodes <= 10_000);
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn legal_moves_and_sfen_round_trip_in_browser() {
        let moves = legal_moves(&start_sfen()).expect("start position must be valid");
        assert_eq!(moves.length(), 30);
        let next = apply_move(&start_sfen(), "7g7f").expect("7g7f must be legal");
        let replies = legal_moves(&next).expect("resulting position must be valid");
        let reply = computer_move(&next, 1, 10_000).expect("bounded search must succeed");
        assert!(
            replies
                .iter()
                .any(|candidate| candidate.as_string().as_deref() == Some(&reply.best_move()))
        );
        assert!(reply.nodes() <= 10_000);

        let error = legal_moves("not sfen").expect_err("malformed SFEN must fail");
        assert_eq!(
            Reflect::get(&error, &JsValue::from_str("code"))
                .expect("error must expose code")
                .as_string()
                .as_deref(),
            Some("invalid_sfen")
        );
    }
}
