//! Browser-safe, stateless bindings for Sekirei's rules and bounded search.
//!
//! The exported API accepts complete SFEN strings and returns either ordinary
//! JavaScript values or an error object with `code` and `message` fields. It
//! deliberately exposes no filesystem or external-weight controls. Browser
//! search is currently fixed to one worker; callers can inspect that contract
//! through [`search_capabilities`] and each [`ComputerMove`] result.

use js_sys::{Array, Object, Reflect};
use sekirei_core::board::Board;
use sekirei_core::color::Color;
use sekirei_core::mate::{
    MateAnalysisAbortReason, MateInOneInvalidReason, analyze_mate as analyze_core_mate,
    analyze_mate_in_one as analyze_core_mate_in_one,
};
use sekirei_core::movegen::{generate_legal_moves, is_in_check};
use sekirei_core::piece::PieceKind;
use sekirei_core::search::{MATE_SCORE, SearchConfig, Searcher};
use sekirei_core::sfen::{
    PositionHistory, STARTPOS_SFEN, board_to_sfen, move_from_usi, move_to_usi,
};
use sekirei_core::tt::Tt;
use wasm_bindgen::prelude::*;

const MAX_SFEN_BYTES: usize = 512;
const MAX_MOVE_BYTES: usize = 8;
const MAX_SEARCH_DEPTH: u32 = 8;
const MAX_SEARCH_NODES: u32 = 100_000;
const MAX_MATE_PLY: u32 = 15;
const MAX_MATE_NODES: u32 = 1_000_000;
const SEARCH_TT_MIB: usize = 4;
const BROWSER_SEARCH_WORKERS: u32 = 1;
#[cfg(test)]
const ALREADY_CHECKED_MATE_SFEN: &str = "4k4/2S3S2/3S1S3/4R4/9/9/9/9/4K4 b - 1";
#[cfg(test)]
const VALID_MATE_SFEN: &str = "4k4/2S3S2/2SGpGS2/9/4R4/9/9/9/4K4 b - 1";
#[cfg(test)]
const SHORTEST_THREE_SFEN: &str = "4k4/9/2G3S2/5R3/2GG5/9/9/9/4K4 b - 1";
#[cfg(test)]
const UNIQUE_FIVE_SFEN: &str =
    "lnS1k2nl/3sr1G2/2p+Pb3p/p3Sp1p1/PpP2P1P1/5Gp2/LPN2gP1P/2+p+pr1G1L/Kb2S2N1 w P 110";

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
struct PositionResult {
    kind: &'static str,
    score_cp: Option<i32>,
    mate_plies: Option<u32>,
    winner: Option<&'static str>,
    side_to_move: &'static str,
    depth: u32,
    nodes: u32,
    bound: &'static str,
    aborted: bool,
    abort_reason: Option<&'static str>,
    used_fallback: bool,
    best_move: Option<String>,
    terminal_reason: Option<&'static str>,
    in_check: bool,
}

fn side_name(side: Color) -> &'static str {
    match side {
        Color::Black => "b",
        Color::White => "w",
    }
}

fn classify_score(result: &mut PositionResult, score: i32, side: Color) {
    if score.abs() >= MATE_SCORE - 1000 {
        result.kind = "mate";
        result.mate_plies = Some((MATE_SCORE - score.abs()).max(0) as u32);
        result.winner = Some(side_name(if score > 0 { side } else { side.flip() }));
    } else {
        result.kind = "cp";
        result.score_cp = Some(score);
    }
}

fn validate_analysis_inventory(sfen: &str) -> Result<(), ApiError> {
    if sfen.len() > MAX_SFEN_BYTES {
        return Err(ApiError::new(
            "input_too_large",
            format!("SFEN must be at most {MAX_SFEN_BYTES} bytes"),
        ));
    }
    let mut fields = sfen.split_whitespace();
    let (Some(board), Some(_side), Some(hand)) = (fields.next(), fields.next(), fields.next())
    else {
        // Keep the existing SFEN parser's malformed-input diagnostics.
        return Ok(());
    };
    let index = |piece: char| match piece.to_ascii_uppercase() {
        'P' => Some(0),
        'L' => Some(1),
        'N' => Some(2),
        'S' => Some(3),
        'G' => Some(4),
        'B' => Some(5),
        'R' => Some(6),
        'K' => Some(7),
        _ => None,
    };
    let mut total = [0u32; 8];
    let limit = [18, 4, 4, 4, 4, 2, 2, 2];
    let inventory_error = || {
        ApiError::new(
            "invalid_position",
            "piece inventory exceeds standard shogi limits",
        )
    };
    // Count promoted pieces as their base kind, across both colors. Missing
    // pieces are allowed (handicaps and composed problems), extras are not.
    for piece in board.chars().filter_map(index) {
        total[piece] += 1;
        if total[piece] > limit[piece] {
            return Err(inventory_error());
        }
    }
    if hand != "-" {
        let mut count = 0u32;
        for token in hand.chars() {
            if let Some(digit) = token.to_digit(10) {
                count = count
                    .checked_mul(10)
                    .and_then(|n| n.checked_add(digit))
                    .ok_or_else(inventory_error)?;
            } else {
                let piece = index(token)
                    .filter(|&kind| kind != 7)
                    .ok_or_else(|| ApiError::new("invalid_sfen", "invalid hand piece"))?;
                total[piece] = total[piece]
                    .checked_add(count.max(1))
                    .ok_or_else(inventory_error)?;
                if total[piece] > limit[piece] {
                    return Err(inventory_error());
                }
                count = 0;
            }
        }
    }
    // This must run before Board::from_sfen: its packed Hand representation
    // assumes a physical inventory, and oversized counts can panic while
    // recomputing the hash. Combined inventory also protects later captures.
    Ok(())
}

fn analyze_position_impl(
    sfen: &str,
    max_depth: u32,
    max_nodes: u32,
) -> Result<PositionResult, ApiError> {
    validate_search_limits(max_depth, max_nodes)?;
    validate_analysis_inventory(sfen)?;
    let mut board = parse_board(sfen)?;
    // Rules APIs retain their original permissive parsing contract. Analysis
    // requires kings so a malformed setup is not mistaken for a finite score.
    if [Color::Black, Color::White]
        .iter()
        .any(|&side| board.pieces(side, PieceKind::Ou).popcount() != 1)
        || is_in_check(&board, board.side_to_move.flip())
    {
        return Err(ApiError::new(
            "invalid_position",
            "analysis requires one king per side and the non-moving king not in check",
        ));
    }
    let side = board.side_to_move;
    let in_check = is_in_check(&board, side);
    let mut result = PositionResult {
        kind: "unknown",
        score_cp: None,
        mate_plies: None,
        winner: None,
        side_to_move: side_name(side),
        depth: 0,
        nodes: 0,
        bound: "unknown",
        aborted: false,
        abort_reason: None,
        used_fallback: false,
        best_move: None,
        terminal_reason: None,
        in_check,
    };
    if generate_legal_moves(&mut board).is_empty() {
        result.kind = "terminal";
        result.bound = "exact";
        result.terminal_reason = Some(if in_check { "checkmate" } else { "no_moves" });
        // In shogi, having no legal move is a loss, not a chess stalemate.
        result.winner = Some(side_name(side.flip()));
        return Ok(result);
    }
    let mut searcher = Searcher::new(Tt::new_for_evaluation(SEARCH_TT_MIB, false));
    searcher.set_ybw_split(false);
    let history = PositionHistory::initial(board.hash());
    let (info, iterations) = searcher.search_with_history_trace(
        &mut board,
        SearchConfig {
            max_depth,
            node_limit: Some(u64::from(max_nodes)),
            time_limit: None,
            soft_limit: None,
            multi_pv: 1,
        },
        &history,
    );
    result.nodes = info.nodes.min(u64::from(u32::MAX)) as u32;
    result.aborted = info.aborted;
    result.abort_reason = info.aborted.then_some("node_limit");
    // The core may select a move from a partially completed deeper pass.
    // Keep score, bound, depth, and bestMove together from the same completed
    // pass instead. An initial fallback is useful for play, not graph data.
    if let Some(completed) = iterations.last() {
        result.depth = completed.depth;
        result.bound = completed.bound.as_str();
        result.best_move = completed.best_move.map(move_to_usi);
        if completed.bound.as_str() != "unknown" {
            classify_score(&mut result, completed.score, side);
        }
    } else {
        result.used_fallback = info.best_move.is_some();
        result.best_move = info.best_move.map(move_to_usi);
    }
    Ok(result)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MateInOneResult {
    valid_position: bool,
    defender_already_in_check: bool,
    invalid_reason: Option<&'static str>,
    solutions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MateResult {
    valid_position: bool,
    outcome: &'static str,
    shortest_mate_ply: Option<u32>,
    solutions: Vec<String>,
    unique_solution: bool,
    nodes: u32,
    aborted: bool,
    reason: Option<&'static str>,
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

fn validate_mate_limits(max_ply: u32, node_limit: u32) -> Result<(), ApiError> {
    if !(1..=MAX_MATE_PLY).contains(&max_ply) {
        return Err(ApiError::new(
            "invalid_mate_limit",
            format!("maxPly must be in 1..={MAX_MATE_PLY}"),
        ));
    }
    if !(1..=MAX_MATE_NODES).contains(&node_limit) {
        return Err(ApiError::new(
            "invalid_mate_limit",
            format!("nodeLimit must be in 1..={MAX_MATE_NODES}"),
        ));
    }
    Ok(())
}

fn analyze_mate_impl(sfen: &str, max_ply: u32, node_limit: u32) -> Result<MateResult, ApiError> {
    validate_mate_limits(max_ply, node_limit)?;
    let board = parse_board(sfen)?;
    let analysis = analyze_core_mate(&board, max_ply, u64::from(node_limit));
    let valid_position = analysis.valid_position();
    let outcome = analysis.outcome.code();
    let shortest_mate_ply = analysis.shortest_mate_ply;
    let unique_solution = analysis.unique_solution();
    let mut solutions = analysis
        .solutions
        .into_iter()
        .map(move_to_usi)
        .collect::<Vec<_>>();
    solutions.sort_unstable();
    let reason = analysis
        .invalid_reason
        .map(MateInOneInvalidReason::code)
        .or_else(|| analysis.abort_reason.map(MateAnalysisAbortReason::code));
    Ok(MateResult {
        valid_position,
        outcome,
        shortest_mate_ply,
        solutions,
        unique_solution,
        nodes: analysis.nodes.min(u64::from(u32::MAX)) as u32,
        aborted: analysis.aborted,
        reason,
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

/// Typed evaluation of one position, separate from the move-selection API.
#[wasm_bindgen]
pub struct PositionAnalysis {
    result: PositionResult,
}

#[wasm_bindgen]
impl PositionAnalysis {
    /// `cp`, `mate`, `terminal`, or `unknown`; unknown has no numeric score.
    #[wasm_bindgen(
        getter,
        unchecked_return_type = "'cp' | 'mate' | 'terminal' | 'unknown'"
    )]
    pub fn kind(&self) -> String {
        self.result.kind.to_owned()
    }

    /// Completed normal evaluation, in centipawns from the moving side.
    #[wasm_bindgen(getter, js_name = scoreCp)]
    pub fn score_cp(&self) -> Option<i32> {
        self.result.score_cp
    }

    /// Absolute mate distance in plies, as found by search (not shortest-mate proof).
    #[wasm_bindgen(getter, js_name = matePlies)]
    pub fn mate_plies(&self) -> Option<u32> {
        self.result.mate_plies
    }

    /// Winning side for a mate score or terminal position, when known.
    #[wasm_bindgen(getter, unchecked_return_type = "'b' | 'w' | undefined")]
    pub fn winner(&self) -> Option<String> {
        self.result.winner.map(str::to_owned)
    }

    /// `b` means Black/Sente; `w` means White/Gote.
    #[wasm_bindgen(getter, js_name = sideToMove, unchecked_return_type = "'b' | 'w'")]
    pub fn side_to_move(&self) -> String {
        self.result.side_to_move.to_owned()
    }

    /// Perspective of normal scores and bound inequalities.
    #[wasm_bindgen(getter, js_name = scorePerspective, unchecked_return_type = "'sideToMove'")]
    pub fn score_perspective(&self) -> String {
        "sideToMove".to_owned()
    }

    /// Unit used only by `scoreCp`; mate and terminal are not centipawns.
    #[wasm_bindgen(getter, js_name = scoreUnit, unchecked_return_type = "'cp'")]
    pub fn score_unit(&self) -> String {
        "cp".to_owned()
    }

    /// Completed iteration depth corresponding to this score (zero for none).
    #[wasm_bindgen(getter)]
    pub fn depth(&self) -> u32 {
        self.result.depth
    }

    /// Total search nodes, including an interrupted deeper pass.
    #[wasm_bindgen(getter)]
    pub fn nodes(&self) -> u32 {
        self.result.nodes
    }

    /// Bound of the returned completed score, not the interrupted pass.
    #[wasm_bindgen(
        getter,
        unchecked_return_type = "'exact' | 'lower' | 'upper' | 'unknown'"
    )]
    pub fn bound(&self) -> String {
        self.result.bound.to_owned()
    }

    /// Whether a later pass was interrupted by the node budget.
    #[wasm_bindgen(getter)]
    pub fn aborted(&self) -> bool {
        self.result.aborted
    }

    /// Stable stop reason; absent if the requested search completed.
    #[wasm_bindgen(getter, js_name = abortReason, unchecked_return_type = "'node_limit' | undefined")]
    pub fn abort_reason(&self) -> Option<String> {
        self.result.abort_reason.map(str::to_owned)
    }

    /// A legal fallback was selected without completing any iteration.
    #[wasm_bindgen(getter, js_name = usedFallback)]
    pub fn used_fallback(&self) -> bool {
        self.result.used_fallback
    }

    /// Move from the completed iteration, or a legal fallback for unknown.
    #[wasm_bindgen(getter, js_name = bestMove)]
    pub fn best_move(&self) -> Option<String> {
        self.result.best_move.clone()
    }

    /// Reason for terminal (no-legal-moves) results only.
    #[wasm_bindgen(getter, js_name = terminalReason, unchecked_return_type = "'checkmate' | 'no_moves' | undefined")]
    pub fn terminal_reason(&self) -> Option<String> {
        self.result.terminal_reason.map(str::to_owned)
    }

    /// Whether the king of the moving side is currently in check.
    #[wasm_bindgen(getter, js_name = inCheck)]
    pub fn in_check(&self) -> bool {
        self.result.in_check
    }

    /// This browser build uses built-in material evaluation only.
    #[wasm_bindgen(getter, js_name = evaluatorId, unchecked_return_type = "'material'")]
    pub fn evaluator_id(&self) -> String {
        "material".to_owned()
    }

    /// Version of the built-in material values; no weight file is loaded.
    #[wasm_bindgen(getter, js_name = evaluatorVersion)]
    pub fn evaluator_version(&self) -> String {
        "material-v1".to_owned()
    }

    /// Cargo package version (candidate builds also need source provenance).
    #[wasm_bindgen(getter, js_name = engineVersion)]
    pub fn engine_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    /// Contract schema version, separate from engine and evaluator versions.
    #[wasm_bindgen(getter, js_name = apiVersion)]
    pub fn api_version(&self) -> u32 {
        1
    }
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

/// Complete shortest-mate analysis for a browser-supplied SFEN position.
#[wasm_bindgen]
pub struct MateAnalysis {
    result: MateResult,
}

#[wasm_bindgen]
impl MateAnalysis {
    /// Whether the initial position satisfies the mate-problem contract.
    #[wasm_bindgen(getter, js_name = validPosition)]
    pub fn valid_position(&self) -> bool {
        self.result.valid_position
    }

    /// `mate`, `no_mate`, or `unknown`.
    #[wasm_bindgen(getter)]
    pub fn outcome(&self) -> String {
        self.result.outcome.to_owned()
    }

    /// Shortest proven mate length in plies, when `outcome` is `mate`.
    #[wasm_bindgen(getter, js_name = shortestMatePly)]
    pub fn shortest_mate_ply(&self) -> Option<u32> {
        self.result.shortest_mate_ply
    }

    /// Every first move that forces mate at the shortest proven length.
    #[wasm_bindgen(getter, unchecked_return_type = "string[]")]
    pub fn solutions(&self) -> Array {
        let array = Array::new();
        for solution in &self.result.solutions {
            array.push(&JsValue::from_str(solution));
        }
        array
    }

    /// Whether the completed shortest-depth search found exactly one first move.
    #[wasm_bindgen(getter, js_name = uniqueSolution)]
    pub fn unique_solution(&self) -> bool {
        self.result.unique_solution
    }

    /// Positions visited across all completed and partial depth passes.
    #[wasm_bindgen(getter)]
    pub fn nodes(&self) -> u32 {
        self.result.nodes
    }

    /// Whether the node budget stopped the complete search.
    #[wasm_bindgen(getter)]
    pub fn aborted(&self) -> bool {
        self.result.aborted
    }

    /// Stable invalid-position or abort reason code.
    #[wasm_bindgen(getter)]
    pub fn reason(&self) -> Option<String> {
        self.result.reason.map(str::to_owned)
    }
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

/// Find the shortest forced mate up to `maxPly` and return all first moves.
///
/// The attacker may play checking moves only; the defender may play every
/// legal reply. `maxPly` must be in `1..=15` and `nodeLimit` in
/// `1..=1000000`. An exhausted node budget returns `outcome = "unknown"`
/// with no partial solutions.
#[wasm_bindgen(js_name = analyzeMate)]
pub fn analyze_mate(sfen: &str, max_ply: u32, node_limit: u32) -> Result<MateAnalysis, JsValue> {
    analyze_mate_impl(sfen, max_ply, node_limit)
        .map(|result| MateAnalysis { result })
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

/// Analyze a position without applying a move. Score, bound, and depth always
/// refer to the same fully completed iteration. An initial node-limit fallback
/// returns `unknown`, never a fabricated zero. Limits match [`computer_move`].
#[wasm_bindgen(js_name = analyzePosition)]
pub fn analyze_position(
    sfen: &str,
    max_depth: u32,
    max_nodes: u32,
) -> Result<PositionAnalysis, JsValue> {
    analyze_position_impl(sfen, max_depth, max_nodes)
        .map(|result| PositionAnalysis { result })
        .map_err(ApiError::into_js)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASYMMETRIC_BLACK: &str = "4k4/9/9/9/9/9/9/9/4K4 b P 1";
    const ASYMMETRIC_WHITE: &str = "4k4/9/9/9/9/9/9/9/4K4 w P 1";

    #[test]
    fn analysis_rejects_oversized_inventory_before_board_parsing() {
        for hand in [
            "255r255b255g255s255n255l255p",
            "2R2R",
            "2Rr",
            "19P",
            "99999999999999999999999999999999999P",
        ] {
            let sfen = format!("4k4/9/9/9/9/9/9/9/4K4 b {hand} 1");
            assert_eq!(
                analyze_position_impl(&sfen, 1, 1000).unwrap_err().code,
                "invalid_position"
            );
        }
        for sfen in [
            "4k4/9/9/9/4R4/9/9/9/4K4 b 2R 1",
            "4k4/9/9/9/2+R+R+R4/9/9/9/4K4 b - 1",
        ] {
            assert_eq!(
                analyze_position_impl(sfen, 1, 1000).unwrap_err().code,
                "invalid_position"
            );
        }
        let legal =
            analyze_position_impl("4k4/9/9/9/9/9/9/9/4K4 b 18P4L4N4S4G2B2R 1", 1, 1).unwrap();
        assert_eq!(legal.kind, "unknown");
        assert_eq!(legal.score_cp, None);
    }

    #[test]
    fn position_analysis_matches_core_score_and_side_to_move() {
        for (sfen, side, sign) in [(ASYMMETRIC_BLACK, "b", 1), (ASYMMETRIC_WHITE, "w", -1)] {
            let result = analyze_position_impl(sfen, 1, 10_000).unwrap();
            let core = computer_move_impl(sfen, 1, 10_000).unwrap();
            assert_eq!(result.kind, "cp");
            assert_eq!(result.side_to_move, side);
            assert_eq!(result.score_cp, Some(core.score));
            assert_eq!(result.score_cp.unwrap().signum(), sign);
            assert_eq!(result.depth, core.depth);
            assert_eq!(result.bound, "exact");
            assert!(!result.aborted);
            assert!(result.mate_plies.is_none());
        }
    }

    #[test]
    fn initial_cutoff_has_no_score_and_marks_fallback() {
        let result = analyze_position_impl(STARTPOS_SFEN, 8, 1).unwrap();
        assert_eq!(result.kind, "unknown");
        assert_eq!(result.score_cp, None);
        assert_eq!(result.mate_plies, None);
        assert_eq!(result.depth, 0);
        assert_eq!(result.bound, "unknown");
        assert!(result.aborted && result.used_fallback);
        assert_eq!(result.abort_reason, Some("node_limit"));
        assert!(
            legal_move_strings(STARTPOS_SFEN)
                .unwrap()
                .contains(&result.best_move.unwrap())
        );
    }

    #[test]
    fn later_cutoff_retains_the_exact_completed_iteration() {
        let first = analyze_position_impl(STARTPOS_SFEN, 1, MAX_SEARCH_NODES).unwrap();
        let later = analyze_position_impl(STARTPOS_SFEN, 8, first.nodes + 1).unwrap();
        assert!(later.aborted);
        assert!(!later.used_fallback);
        assert_eq!(later.abort_reason, Some("node_limit"));
        assert_eq!(later.kind, first.kind);
        assert_eq!(later.score_cp, first.score_cp);
        assert_eq!(later.depth, first.depth);
        assert_eq!(later.bound, first.bound);
        assert_eq!(later.best_move, first.best_move);
        assert_eq!(later.depth, 1);
    }

    #[test]
    fn mate_score_classification_handles_both_signs_without_cp() {
        for (side, sign, winner) in [
            (Color::Black, 1, "b"),
            (Color::Black, -1, "w"),
            (Color::White, 1, "w"),
            (Color::White, -1, "b"),
        ] {
            let mut result = analyze_position_impl(STARTPOS_SFEN, 8, 1).unwrap();
            classify_score(&mut result, sign * (MATE_SCORE - 5), side);
            assert_eq!(result.kind, "mate");
            assert_eq!(result.score_cp, None);
            assert_eq!(result.mate_plies, Some(5));
            assert_eq!(result.winner, Some(winner));
        }
        let mate = analyze_position_impl(VALID_MATE_SFEN, 3, MAX_SEARCH_NODES).unwrap();
        assert_eq!(mate.kind, "mate");
        assert_eq!(mate.mate_plies, Some(1));
        assert_eq!(mate.winner, Some("b"));
        assert!(mate.score_cp.is_none());
        let forcing = analyze_mate_impl(SHORTEST_THREE_SFEN, 3, MAX_MATE_NODES).unwrap();
        let defending = apply_move_impl(SHORTEST_THREE_SFEN, &forcing.solutions[0]).unwrap();
        let loss = analyze_position_impl(&defending, 3, MAX_SEARCH_NODES).unwrap();
        assert_eq!(loss.kind, "mate");
        assert_eq!(loss.side_to_move, "w");
        assert_eq!(loss.winner, Some("b"));
        assert_eq!(loss.mate_plies, Some(2));
        assert!(loss.score_cp.is_none());
    }

    #[test]
    fn terminal_and_invalid_positions_are_not_cp() {
        let terminal = apply_move_impl(VALID_MATE_SFEN, "5e5c+").unwrap();
        let result = analyze_position_impl(&terminal, 1, 1000).unwrap();
        assert_eq!(result.kind, "terminal");
        assert_eq!(result.terminal_reason, Some("checkmate"));
        assert_eq!(result.winner, Some("b"));
        assert!(result.in_check);
        assert_eq!(result.bound, "exact");
        assert!(result.score_cp.is_none() && result.mate_plies.is_none());
        assert!(!result.aborted && !result.used_fallback);
        assert_eq!(result.nodes, 0);
        let no_moves = analyze_position_impl("3PKP3/3PPP3/9/9/9/9/9/9/4k4 b - 1", 1, 1000).unwrap();
        assert_eq!(no_moves.kind, "terminal");
        assert_eq!(no_moves.terminal_reason, Some("no_moves"));
        assert!(!no_moves.in_check);
        assert_eq!(no_moves.winner, Some("w"));
        assert_eq!(
            analyze_position_impl("not sfen", 1, 1000).unwrap_err().code,
            "invalid_sfen"
        );
        assert_eq!(
            analyze_position_impl("9/9/9/9/9/9/9/9/9 b - 1", 1, 1000)
                .unwrap_err()
                .code,
            "invalid_position"
        );
        assert_eq!(
            analyze_position_impl(ALREADY_CHECKED_MATE_SFEN, 1, 1000)
                .unwrap_err()
                .code,
            "invalid_position"
        );
        assert_eq!(
            analyze_position_impl(STARTPOS_SFEN, 0, 1).unwrap_err().code,
            "invalid_search_limit"
        );
    }

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

    #[test]
    fn bounded_mate_adapter_reports_shortest_unique_and_unknown_results() {
        let multiple = analyze_mate_impl(VALID_MATE_SFEN, 5, MAX_MATE_NODES).unwrap();
        assert_eq!(multiple.outcome, "mate");
        assert_eq!(multiple.shortest_mate_ply, Some(1));
        assert!(!multiple.unique_solution);
        assert!(multiple.solutions.contains(&"5e5c".to_owned()));
        assert!(multiple.solutions.contains(&"5e5c+".to_owned()));

        let shorter = analyze_mate_impl(SHORTEST_THREE_SFEN, 5, MAX_MATE_NODES).unwrap();
        assert_eq!(shorter.outcome, "mate");
        assert_eq!(shorter.shortest_mate_ply, Some(3));

        let no_mate = analyze_mate_impl(STARTPOS_SFEN, 5, MAX_MATE_NODES).unwrap();
        assert_eq!(no_mate.outcome, "no_mate");
        assert!(no_mate.shortest_mate_ply.is_none());
        assert!(no_mate.solutions.is_empty());
        assert!(!no_mate.aborted);

        let unique = analyze_mate_impl(UNIQUE_FIVE_SFEN, 5, MAX_MATE_NODES).unwrap();
        assert_eq!(unique.outcome, "mate");
        assert_eq!(unique.shortest_mate_ply, Some(5));
        assert_eq!(unique.solutions, vec!["7h8h"]);
        assert!(unique.unique_solution);

        let cutoff = analyze_mate_impl(UNIQUE_FIVE_SFEN, 5, 1).unwrap();
        assert_eq!(cutoff.outcome, "unknown");
        assert!(cutoff.shortest_mate_ply.is_none());
        assert!(cutoff.solutions.is_empty());
        assert!(!cutoff.unique_solution);
        assert!(cutoff.aborted);
        assert_eq!(cutoff.reason, Some("node_limit"));
    }

    #[test]
    fn bounded_mate_limits_are_enforced() {
        assert_eq!(
            analyze_mate_impl(VALID_MATE_SFEN, 0, 1_000)
                .unwrap_err()
                .code,
            "invalid_mate_limit"
        );
        assert_eq!(
            analyze_mate_impl(VALID_MATE_SFEN, 5, MAX_MATE_NODES + 1)
                .unwrap_err()
                .code,
            "invalid_mate_limit"
        );
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn position_analysis_contract_in_browser() {
        let oversized = match analyze_position(
            "4k4/9/9/9/9/9/9/9/4K4 b 255r255b255g255s255n255l255p 1",
            1,
            1000,
        ) {
            Ok(_) => panic!("oversized hand must fail without trapping"),
            Err(error) => error,
        };
        assert_eq!(
            Reflect::get(&oversized, &JsValue::from_str("code"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("invalid_position")
        );
        for (side, sign) in [("b", 1), ("w", -1)] {
            let sfen = format!("4k4/9/9/9/9/9/9/9/4K4 {side} P 1");
            let result = analyze_position(&sfen, 1, 10_000).unwrap();
            let core = computer_move_impl(&sfen, 1, 10_000).unwrap();
            assert_eq!(result.kind(), "cp");
            assert_eq!(result.score_cp(), Some(core.score));
            assert_eq!(result.score_cp().unwrap().signum(), sign);
            assert_eq!(result.side_to_move(), side);
            assert_eq!(result.score_perspective(), "sideToMove");
            assert_eq!(result.score_unit(), "cp");
            assert_eq!(result.evaluator_id(), "material");
            assert_eq!(result.evaluator_version(), "material-v1");
            assert_eq!(result.engine_version(), env!("CARGO_PKG_VERSION"));
            assert_eq!(result.api_version(), 1);
            assert_eq!(result.bound(), "exact");
        }
        let initial = analyze_position(STARTPOS_SFEN, 8, 1).unwrap();
        assert_eq!(initial.kind(), "unknown");
        assert_eq!(initial.score_cp(), None);
        assert_eq!(initial.mate_plies(), None);
        assert_eq!(initial.bound(), "unknown");
        assert!(initial.aborted() && initial.used_fallback());
        assert_eq!(initial.abort_reason().as_deref(), Some("node_limit"));
        let completed = analyze_position(STARTPOS_SFEN, 1, MAX_SEARCH_NODES).unwrap();
        let partial = analyze_position(STARTPOS_SFEN, 8, completed.nodes() + 1).unwrap();
        assert!(partial.aborted());
        assert!(!partial.used_fallback());
        assert_eq!(partial.score_cp(), completed.score_cp());
        assert_eq!(partial.depth(), completed.depth());
        assert_eq!(partial.bound(), completed.bound());
        assert_eq!(partial.best_move(), completed.best_move());
        let mate = analyze_position(VALID_MATE_SFEN, 3, MAX_SEARCH_NODES).unwrap();
        assert_eq!(mate.kind(), "mate");
        assert_eq!(mate.score_cp(), None);
        assert_eq!(mate.mate_plies(), Some(1));
        assert_eq!(mate.winner().as_deref(), Some("b"));
        let forcing = analyze_mate_impl(SHORTEST_THREE_SFEN, 3, MAX_MATE_NODES).unwrap();
        let defending = apply_move_impl(SHORTEST_THREE_SFEN, &forcing.solutions[0]).unwrap();
        let loss = analyze_position(&defending, 3, MAX_SEARCH_NODES).unwrap();
        assert_eq!(loss.kind(), "mate");
        assert_eq!(loss.side_to_move(), "w");
        assert_eq!(loss.winner().as_deref(), Some("b"));
        assert_eq!(loss.mate_plies(), Some(2));
        assert_eq!(loss.score_cp(), None);
        let terminal = apply_move_impl(VALID_MATE_SFEN, "5e5c+").unwrap();
        let result = analyze_position(&terminal, 1, 1000).unwrap();
        assert_eq!(result.kind(), "terminal");
        assert_eq!(result.terminal_reason().as_deref(), Some("checkmate"));
        assert!(result.in_check());
        assert_eq!(result.score_cp(), None);
        let error = match analyze_position("not sfen", 1, 1000) {
            Ok(_) => panic!("malformed SFEN must fail"),
            Err(error) => error,
        };
        assert_eq!(
            Reflect::get(&error, &JsValue::from_str("code"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some("invalid_sfen")
        );
    }

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

        let shortest = analyze_mate(SHORTEST_THREE_SFEN, 5, MAX_MATE_NODES)
            .expect("bounded mate fixture must be valid");
        assert_eq!(shortest.outcome(), "mate");
        assert_eq!(shortest.shortest_mate_ply(), Some(3));
        assert!(!shortest.aborted());

        let unique = analyze_mate(UNIQUE_FIVE_SFEN, 5, MAX_MATE_NODES)
            .expect("unique mate-in-five fixture must be valid");
        assert_eq!(unique.shortest_mate_ply(), Some(5));
        assert!(unique.unique_solution());
        assert_eq!(unique.solutions().length(), 1);

        let cutoff = analyze_mate(UNIQUE_FIVE_SFEN, 5, 1)
            .expect("resource exhaustion must be represented in the result");
        assert_eq!(cutoff.outcome(), "unknown");
        assert!(cutoff.shortest_mate_ply().is_none());
        assert_eq!(cutoff.solutions().length(), 0);
        assert!(!cutoff.unique_solution());
        assert!(cutoff.aborted());
        assert_eq!(cutoff.reason().as_deref(), Some("node_limit"));

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
