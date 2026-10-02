//! Browser-safe, stateless bindings for Sekirei's rules and bounded search.
//!
//! The exported API accepts complete SFEN strings and returns either ordinary
//! JavaScript values or an error object with `code` and `message` fields. It
//! deliberately exposes no filesystem or external-weight controls. Browser
//! search is currently fixed to one worker; callers can inspect that contract
//! through [`search_capabilities`] and each [`ComputerMove`] result.

use js_sys::{Array, Object, Reflect};
use sekirei_core::board::Board;
use sekirei_core::mate::{MateInOneInvalidReason, analyze_mate_in_one as analyze_core_mate_in_one};
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
const BROWSER_SEARCH_WORKERS: u32 = 1;
#[cfg(test)]
const ALREADY_CHECKED_MATE_SFEN: &str = "4k4/2S3S2/3S1S3/4R4/9/9/9/9/4K4 b - 1";
#[cfg(test)]
const VALID_MATE_SFEN: &str = "4k4/2S3S2/2SGpGS2/9/4R4/9/9/9/4K4 b - 1";

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

#[derive(Clone, Debug, Eq, PartialEq)]
struct MateInOneResult {
    valid_position: bool,
    defender_already_in_check: bool,
    invalid_reason: Option<&'static str>,
    solutions: Vec<String>,
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

fn analyze_mate_in_one_impl(sfen: &str) -> Result<MateInOneResult, ApiError> {
    let board = parse_board(sfen)?;
    let analysis = analyze_core_mate_in_one(&board);
    let valid_position = analysis.valid_position();
    let defender_already_in_check = analysis.defender_already_in_check();
    let invalid_reason = analysis.invalid_reason.map(MateInOneInvalidReason::code);
    let mut solutions = analysis
        .solutions
        .into_iter()
        .map(move_to_usi)
        .collect::<Vec<_>>();
    solutions.sort_unstable();
    Ok(MateInOneResult {
        valid_position,
        defender_already_in_check,
        invalid_reason,
        solutions,
    })
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

    /// Workers that actually participated in this search.
    ///
    /// Browser search is currently deterministic and sequential, so this is
    /// always one even when the host has more logical processors.
    #[wasm_bindgen(getter, js_name = effectiveWorkers)]
    pub fn effective_workers(&self) -> u32 {
        BROWSER_SEARCH_WORKERS
    }
}

/// Browser-search worker capabilities for this package build.
///
/// The current implementation intentionally has no worker-thread or
/// `SharedArrayBuffer` dependency. Clients must use these values instead of
/// inferring usable cores from `navigator.hardwareConcurrency`.
#[wasm_bindgen]
pub struct SearchCapabilities;

#[wasm_bindgen]
impl SearchCapabilities {
    /// Maximum worker count accepted by browser search.
    #[wasm_bindgen(getter, js_name = maxWorkers)]
    pub fn max_workers(&self) -> u32 {
        BROWSER_SEARCH_WORKERS
    }

    /// Worker count that browser search will actually use.
    #[wasm_bindgen(getter, js_name = effectiveWorkers)]
    pub fn effective_workers(&self) -> u32 {
        BROWSER_SEARCH_WORKERS
    }

    /// Whether this package can start additional browser worker threads.
    #[wasm_bindgen(getter, js_name = workerThreadsSupported)]
    pub fn worker_threads_supported(&self) -> bool {
        false
    }

    /// Whether browser search requires `SharedArrayBuffer`.
    #[wasm_bindgen(getter, js_name = sharedArrayBufferRequired)]
    pub fn shared_array_buffer_required(&self) -> bool {
        false
    }
}

/// Complete mate-in-one analysis for a browser-supplied SFEN position.
#[wasm_bindgen]
pub struct MateInOneAnalysis {
    result: MateInOneResult,
}

#[wasm_bindgen]
impl MateInOneAnalysis {
    /// Whether the initial position satisfies the mate-problem contract.
    #[wasm_bindgen(getter, js_name = validPosition)]
    pub fn valid_position(&self) -> bool {
        self.result.valid_position
    }

    /// Whether the defending king was already in check in the initial position.
    #[wasm_bindgen(getter, js_name = defenderAlreadyInCheck)]
    pub fn defender_already_in_check(&self) -> bool {
        self.result.defender_already_in_check
    }

    /// Stable reason code when `validPosition` is false.
    #[wasm_bindgen(getter, js_name = invalidReason)]
    pub fn invalid_reason(&self) -> Option<String> {
        self.result.invalid_reason.map(str::to_owned)
    }

    /// Every legal mate-in-one move, sorted in USI notation.
    #[wasm_bindgen(getter, unchecked_return_type = "string[]")]
    pub fn solutions(&self) -> Array {
        let array = Array::new();
        for solution in &self.result.solutions {
            array.push(&JsValue::from_str(solution));
        }
        array
    }

    /// Whether exactly one solution exists.
    #[wasm_bindgen(getter, js_name = uniqueSolution)]
    pub fn unique_solution(&self) -> bool {
        self.result.solutions.len() == 1
    }
}

/// Return the fixed browser-search worker contract for this package build.
#[wasm_bindgen(js_name = searchCapabilities)]
pub fn search_capabilities() -> SearchCapabilities {
    SearchCapabilities
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

/// Validate a mate-in-one problem and return all complete legal solutions.
///
/// The SFEN side to move is the attacker. A malformed SFEN is returned as an
/// `invalid_sfen` error; a structurally invalid problem position is represented
/// by `validPosition = false` and `invalidReason` in the result.
#[wasm_bindgen(js_name = analyzeMateInOne)]
pub fn analyze_mate_in_one(sfen: &str) -> Result<MateInOneAnalysis, JsValue> {
    analyze_mate_in_one_impl(sfen)
        .map(|result| MateInOneAnalysis { result })
        .map_err(ApiError::into_js)
}

/// Validate and apply one USI move, returning the resulting SFEN.
#[wasm_bindgen(js_name = applyMove)]
pub fn apply_move(sfen: &str, usi_move: &str) -> Result<String, JsValue> {
    apply_move_impl(sfen, usi_move).map_err(ApiError::into_js)
}

/// Select one legal move using material evaluation and bounded sequential search.
///
/// `max_depth` must be in `1..=8`; `max_nodes` must be in `1..=100000`.
/// No external evaluation weights or filesystem access are exposed. Search is
/// fixed to one worker; inspect [`search_capabilities`] before presenting any
/// worker-count UI and read `effectiveWorkers` from the returned result.
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

    #[test]
    fn browser_worker_contract_is_single_threaded_and_shared_memory_free() {
        let capabilities = search_capabilities();
        assert_eq!(capabilities.max_workers(), 1);
        assert_eq!(capabilities.effective_workers(), 1);
        assert!(!capabilities.worker_threads_supported());
        assert!(!capabilities.shared_array_buffer_required());

        let result = computer_move(STARTPOS_SFEN, 1, 10_000).unwrap();
        let repeated = computer_move(STARTPOS_SFEN, 1, 10_000).unwrap();
        assert_eq!(result.effective_workers(), 1);
        assert_eq!(result.best_move(), repeated.best_move());
        assert_eq!(result.score(), repeated.score());
        assert_eq!(result.depth(), repeated.depth());
        assert_eq!(result.nodes(), repeated.nodes());
    }

    #[test]
    fn mate_in_one_adapter_matches_the_core_analysis() {
        let invalid = analyze_mate_in_one_impl(ALREADY_CHECKED_MATE_SFEN).unwrap();
        assert!(!invalid.valid_position);
        assert!(invalid.defender_already_in_check);
        assert_eq!(invalid.invalid_reason, Some("defender_already_in_check"));
        assert!(invalid.solutions.is_empty());

        let result = analyze_mate_in_one_impl(VALID_MATE_SFEN).unwrap();
        let board = Board::from_sfen(VALID_MATE_SFEN).unwrap();
        let mut expected = analyze_core_mate_in_one(&board)
            .solutions
            .into_iter()
            .map(move_to_usi)
            .collect::<Vec<_>>();
        expected.sort_unstable();
        assert_eq!(result.solutions, expected);
        assert!(result.solutions.contains(&"5e5c".to_owned()));

        assert_eq!(
            analyze_mate_in_one_impl("not sfen").unwrap_err().code,
            "invalid_sfen"
        );
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
        assert_eq!(reply.effective_workers(), 1);

        let capabilities = search_capabilities();
        assert_eq!(capabilities.max_workers(), 1);
        assert_eq!(capabilities.effective_workers(), 1);
        assert!(!capabilities.worker_threads_supported());
        assert!(!capabilities.shared_array_buffer_required());

        let invalid = analyze_mate_in_one(ALREADY_CHECKED_MATE_SFEN)
            .expect("parseable invalid problem must return an analysis");
        assert!(!invalid.valid_position());
        assert!(invalid.defender_already_in_check());
        assert_eq!(
            invalid.invalid_reason().as_deref(),
            Some("defender_already_in_check")
        );
        assert_eq!(invalid.solutions().length(), 0);

        let mate = analyze_mate_in_one(VALID_MATE_SFEN).expect("mate fixture must be valid");
        assert!(mate.valid_position());
        assert!(
            mate.solutions()
                .iter()
                .any(|solution| solution.as_string().as_deref() == Some("5e5c"))
        );

        let error = legal_moves("not sfen").expect_err("malformed SFEN must fail");
        assert_eq!(
            Reflect::get(&error, &JsValue::from_str("code"))
                .expect("error must expose code")
                .as_string()
                .as_deref(),
            Some("invalid_sfen")
        );

        let mate_error = match analyze_mate_in_one("not sfen") {
            Ok(_) => panic!("malformed mate SFEN must fail"),
            Err(error) => error,
        };
        assert_eq!(
            Reflect::get(&mate_error, &JsValue::from_str("code"))
                .expect("error must expose code")
                .as_string()
                .as_deref(),
            Some("invalid_sfen")
        );
    }
}
