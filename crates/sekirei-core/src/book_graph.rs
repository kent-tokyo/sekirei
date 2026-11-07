//! Bounded exact-history graph verification for opt-in cyclic-book artifacts.
//!
//! This finite, turn-based graph uses exact position IDs, explicit terminal
//! outcomes, complete legal adjacency declarations, and replayed game history.
//! Statistical priors, static evaluations, and bounded `NoMate`/`Disproven`
//! results are deliberately not accepted as terminal outcomes. The isolated
//! `shogi` adapter checks exact SFEN position identity, every legal transition,
//! check labels, and adjacency completeness with Sekirei's board and movegen.
//! Unrepresented successors are explicit unknown boundaries. It derives only
//! checkmate terminals; other adjudication rules remain outside this prototype.
//!
//! The solver unfolds history without a position-only cache. Fourfold
//! repetition follows Sekirei's current `PositionHistory` convention: the sole
//! continuously checking player loses, otherwise the result is a draw. Missing
//! history, open boundaries, and exhausted limits remain `Unknown` unless an
//! already checked winning branch suffices. A separate certificate checker
//! proves finite adjudication, so a policy that just follows a graph cycle
//! cannot serve as an escape proof. Distance is an upper bound for this
//! certificate, not a shortest-escape claim or a mate distance.
//!
//! This independently written reference implementation is not a Lean proof,
//! a scalable book builder, or a playing-strength claim.

use std::collections::HashSet;

pub use shogi::{
    NodeInput as BookGraphNode, SuccessorInput as BookGraphSuccessor, Target as BookGraphTarget,
};

/// Maximum stored positions admitted by the small reference adapter.
pub const MAX_GRAPH_NODES: usize = 256;
/// Maximum total explicitly declared legal successors.
pub const MAX_GRAPH_EDGES: usize = 16_384;
/// Maximum supplied history length, measured in actual moves.
pub const MAX_HISTORY_PLIES: usize = 512;
/// Maximum further proof depth admitted at runtime.
pub const MAX_PROOF_PLIES: u16 = 16;
/// Maximum visited proof nodes admitted at runtime.
pub const MAX_PROOF_NODES: u64 = 10_000;

/// Caller-selected limits inside the fixed reference adapter ceilings.
#[derive(Clone, Copy, Debug)]
pub struct BookGraphLimits {
    /// Further plies from the current position; must be in `1..=16`.
    pub max_depth: u16,
    /// Visited proof nodes; must be in `1..=10_000`.
    pub node_limit: u64,
}

/// Why the opt-in graph adapter returns control to ordinary search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BookGraphFallback {
    /// No complete, bounded history from a declared game origin was supplied.
    HistoryUnavailable,
    /// A past position or played edge is absent from the known graph.
    HistoryNotRepresented,
    /// The caller's current board does not match replayed history.
    PositionMismatch,
    /// The runtime proof limits exceed the fixed adapter contract.
    InvalidLimits,
    /// Open edges or exhausted proof bounds prevent a proof.
    Unknown,
    /// The result is not a win proven exclusively by checkmate terminals.
    NonMateOutcome,
    /// Final certificate or move validation failed.
    InvalidCertificate,
}

impl BookGraphFallback {
    /// Stable diagnostic code; no fallback is an accepted book move.
    pub const fn code(self) -> &'static str {
        match self {
            Self::HistoryUnavailable => "history_unavailable",
            Self::HistoryNotRepresented => "history_not_represented",
            Self::PositionMismatch => "position_mismatch",
            Self::InvalidLimits => "invalid_limits",
            Self::Unknown => "unknown",
            Self::NonMateOutcome => "non_mate_outcome",
            Self::InvalidCertificate => "invalid_certificate",
        }
    }
}

/// A runtime decision reverified against actual legal shogi transitions.
#[derive(Clone, Copy, Debug)]
pub enum BookGraphDecision {
    /// Every retained opposing reply ends in a verified checkmate.
    ProvenMate {
        /// The legal first move to play.
        best_move: crate::mv::Move,
        /// Worst-case further plies in this proof, not a shortest-mate claim.
        proof_plies: u16,
    },
    /// Ordinary search must handle this request.
    Fallback(BookGraphFallback),
}

/// Small immutable graph whose positions and full legal edge declarations
/// have been checked by Sekirei's board and move generator.
///
/// Only checkmate-based wins can be selected through this production facade.
/// Repetition, perpetual-check adjudication, unknown edges, and other terminal
/// interpretations cannot be promoted to a book move. This API does not
/// establish playing strength, corpus ownership, or reachability from startpos.
pub struct BookGraph(shogi::VerifiedGraph);

impl BookGraph {
    /// Validate exact positions, all legal successors, and every check label.
    pub fn from_nodes(nodes: &[BookGraphNode]) -> Result<Self, String> {
        if nodes.is_empty() || nodes.len() > MAX_GRAPH_NODES {
            return Err("graph node count must be in 1..=256".into());
        }
        let mut edges = 0usize;
        for node in nodes {
            edges = edges
                .checked_add(node.successors.len())
                .ok_or("graph edge count overflow")?;
            if edges > MAX_GRAPH_EDGES || node.sfen.len() > 256 {
                return Err("graph exceeds edge or SFEN size limit".into());
            }
            if node.successors.iter().any(|edge| edge.action.len() > 5) {
                return Err("graph action exceeds canonical USI size".into());
            }
        }
        shogi::VerifiedGraph::from_input(nodes).map(Self)
    }

    /// Verify a bounded mate proof using the full USI `position` body.
    ///
    /// `startpos [moves ...]` is complete by construction. `sfen ... 1
    /// [moves ...]` declares a composed game origin; any other SFEN move number
    /// is rejected because earlier history is unknown. Every supplied past
    /// position and action must be represented by a known graph edge.
    pub fn decision(
        &self,
        position: Option<&str>,
        current: &crate::board::Board,
        limits: BookGraphLimits,
    ) -> BookGraphDecision {
        self.0.production_decision(position, current, limits)
    }
}

type NodeId = usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Player {
    Black,
    White,
}

impl Player {
    fn other(self) -> Self {
        match self {
            Self::Black => Self::White,
            Self::White => Self::Black,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Value {
    Win(Player),
    Draw,
}

impl Value {
    fn score(self) -> i8 {
        match self {
            Self::Win(Player::Black) => 1,
            Self::Draw => 0,
            Self::Win(Player::White) => -1,
        }
    }
}

#[derive(Clone, Debug)]
struct Edge {
    action: String,
    to: NodeId,
    gave_check: bool,
}

#[derive(Clone, Debug)]
struct Node {
    turn: Player,
    edges: Vec<Edge>,
    /// True only when every legal action is represented.
    closed: bool,
    /// Trusted synthetic adjudication, never an evaluation or prior.
    terminal: Option<Value>,
}

#[derive(Clone, Debug)]
struct Graph(Vec<Node>);

impl Graph {
    fn validate(&self) -> Result<(), &'static str> {
        if self.0.is_empty() {
            return Err("empty graph");
        }
        for node in &self.0 {
            if node.terminal.is_some() && (!node.closed || !node.edges.is_empty()) {
                return Err("terminal must be closed with no outgoing edges");
            }
            if node.terminal.is_none() && node.closed && node.edges.is_empty() {
                return Err("closed leaf needs explicit adjudication");
            }
            let mut actions = HashSet::new();
            for edge in &node.edges {
                let Some(target) = self.0.get(edge.to) else {
                    return Err("edge target is absent");
                };
                if target.turn != node.turn.other() {
                    return Err("edge does not alternate players");
                }
                if edge.action.is_empty() || !actions.insert(edge.action.as_str()) {
                    return Err("empty or duplicate action");
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Visit {
    node: NodeId,
    mover: Option<Player>,
    gave_check: bool,
}

/// Full history replayed from an explicitly supplied game origin.
///
/// There is no implicit "current position means new game" constructor in
/// `solve`. An adapter must supply the actual origin and every played action.
#[derive(Clone, Debug)]
struct History {
    visits: Vec<Visit>,
    actions: Vec<usize>,
}

impl History {
    fn replay(graph: &Graph, origin: NodeId, actions: &[usize]) -> Result<Self, &'static str> {
        graph.validate()?;
        if origin >= graph.0.len() {
            return Err("history origin is absent");
        }
        let mut result = Self {
            visits: vec![Visit {
                node: origin,
                mover: None,
                gave_check: false,
            }],
            actions: Vec::new(),
        };
        for &action in actions {
            let node = &graph.0[result.current()];
            if node.terminal.is_some() || result.repetition().is_some() {
                return Err("history continues after adjudication");
            }
            if action >= node.edges.len() {
                return Err("history action is absent");
            }
            result.push(graph, action);
        }
        Ok(result)
    }

    fn current(&self) -> NodeId {
        self.visits
            .last()
            .expect("replayed history is nonempty")
            .node
    }

    fn push(&mut self, graph: &Graph, action: usize) {
        let node = &graph.0[self.current()];
        let edge = &node.edges[action];
        self.visits.push(Visit {
            node: edge.to,
            mover: Some(node.turn),
            gave_check: edge.gave_check,
        });
        self.actions.push(action);
    }

    fn pop(&mut self) {
        self.actions.pop();
        self.visits.pop();
    }

    fn repetition(&self) -> Option<Value> {
        let current = self.current();
        let start = self
            .visits
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, visit)| visit.node == current)
            .nth(3)?
            .0;
        let continuous = |player| {
            let mut moves = self.visits[start + 1..]
                .iter()
                .filter(|visit| visit.mover == Some(player))
                .peekable();
            moves.peek().is_some() && moves.all(|visit| visit.gave_check)
        };
        Some(
            match (continuous(Player::Black), continuous(Player::White)) {
                (true, false) => Value::Win(Player::White),
                (false, true) => Value::Win(Player::Black),
                _ => Value::Draw,
            },
        )
    }
}

#[derive(Clone, Debug)]
enum ProofKind {
    Terminal,
    Repetition,
    Branch(Vec<(usize, Certificate)>),
}

#[derive(Clone, Debug)]
struct Certificate {
    node: NodeId,
    value: Value,
    /// Worst-case further plies across the retained proof branches.
    plies_to_adjudication: u16,
    kind: ProofKind,
}

#[derive(Debug)]
enum Verdict {
    Proven(Certificate),
    Unknown,
}

#[derive(Clone, Copy)]
struct Limits {
    depth: u16,
    nodes: u64,
}

fn leaf(node: NodeId, value: Value, kind: ProofKind) -> Certificate {
    Certificate {
        node,
        value,
        plies_to_adjudication: 0,
        kind,
    }
}

fn branch(node: NodeId, value: Value, children: Vec<(usize, Certificate)>) -> Certificate {
    let distance = children
        .iter()
        .map(|(_, child)| child.plies_to_adjudication)
        .max()
        .expect("branch proof needs a child")
        + 1;
    Certificate {
        node,
        value,
        plies_to_adjudication: distance,
        kind: ProofKind::Branch(children),
    }
}

fn solve(
    graph: &Graph,
    history: Option<&History>,
    limits: Limits,
) -> Result<Verdict, &'static str> {
    graph.validate()?;
    let Some(history) = history else {
        return Ok(Verdict::Unknown);
    };
    // Reconstruct check labels and positions from the current graph. A history
    // carried across another graph revision cannot inject stale check facts.
    let Some(origin) = history.visits.first() else {
        return Err("empty history");
    };
    let mut replay = History::replay(graph, origin.node, &history.actions)?;
    if !same_visits(&replay, history) {
        return Err("history disagrees with graph replay");
    }
    let mut remaining = limits.nodes;
    Ok(visit(graph, &mut replay, limits.depth, &mut remaining))
}

fn same_visits(left: &History, right: &History) -> bool {
    left.visits.len() == right.visits.len()
        && left
            .visits
            .iter()
            .zip(&right.visits)
            .all(|(a, b)| a.node == b.node && a.mover == b.mover && a.gave_check == b.gave_check)
}

fn visit(graph: &Graph, history: &mut History, depth: u16, remaining: &mut u64) -> Verdict {
    if *remaining == 0 {
        return Verdict::Unknown;
    }
    *remaining -= 1;
    let id = history.current();
    let node = &graph.0[id];
    // History adjudication takes priority over static labels at that position.
    if let Some(value) = history.repetition() {
        return Verdict::Proven(leaf(id, value, ProofKind::Repetition));
    }
    if let Some(value) = node.terminal {
        return Verdict::Proven(leaf(id, value, ProofKind::Terminal));
    }
    if depth == 0 {
        return Verdict::Unknown;
    }
    let mut children = Vec::new();
    let mut incomplete = !node.closed;
    for action in 0..node.edges.len() {
        history.push(graph, action);
        let result = visit(graph, history, depth - 1, remaining);
        history.pop();
        match result {
            Verdict::Unknown => incomplete = true,
            Verdict::Proven(proof) => {
                if proof.value == Value::Win(node.turn) {
                    return Verdict::Proven(branch(id, proof.value, vec![(action, proof)]));
                }
                children.push((action, proof));
            }
        }
    }
    if incomplete || children.is_empty() {
        return Verdict::Unknown;
    }
    let best = children
        .iter()
        .map(|(_, child)| child.value)
        .max_by_key(|value| match node.turn {
            Player::Black => value.score(),
            Player::White => -value.score(),
        })
        .expect("nonempty completed children");
    Verdict::Proven(branch(id, best, children))
}

/// Independent certificate traversal: no solver or solver-limit state is used.
///
/// Every recorded branch must be a real edge. A player's win needs one winning
/// action at their turn and every legal reply at the opponent's turn. A draw
/// requires a complete child set; no unknown or merely cyclic leaf is accepted.
fn verify(graph: &Graph, history: &History, proof: &Certificate) -> Result<(), &'static str> {
    graph.validate()?;
    let Some(origin) = history.visits.first() else {
        return Err("empty certificate history");
    };
    let mut replay = History::replay(graph, origin.node, &history.actions)?;
    if !same_visits(&replay, history) {
        return Err("certificate history disagrees with graph replay");
    }
    check_node(graph, &mut replay, proof)
}

fn check_node(
    graph: &Graph,
    history: &mut History,
    proof: &Certificate,
) -> Result<(), &'static str> {
    if proof.node != history.current() {
        return Err("certificate position mismatch");
    }
    let node = &graph.0[proof.node];
    match &proof.kind {
        ProofKind::Terminal => {
            if proof.plies_to_adjudication != 0
                || history.repetition().is_some()
                || node.terminal != Some(proof.value)
            {
                return Err("unverified terminal leaf");
            }
        }
        ProofKind::Repetition => {
            if proof.plies_to_adjudication != 0 || history.repetition() != Some(proof.value) {
                return Err("unverified repetition leaf");
            }
        }
        ProofKind::Branch(children) => {
            if node.terminal.is_some() || history.repetition().is_some() || children.is_empty() {
                return Err("branch at adjudicated position or empty branch");
            }
            let mut seen = HashSet::new();
            let mut longest = 0;
            let mut values = Vec::new();
            for (action, child) in children {
                if *action >= node.edges.len() || !seen.insert(*action) {
                    return Err("certificate action absent or duplicated");
                }
                if child.plies_to_adjudication >= proof.plies_to_adjudication {
                    return Err("certificate does not strictly approach adjudication");
                }
                history.push(graph, *action);
                let checked = check_node(graph, history, child);
                history.pop();
                checked?;
                longest = longest.max(child.plies_to_adjudication);
                values.push(child.value);
            }
            if longest.checked_add(1) != Some(proof.plies_to_adjudication) {
                return Err("incorrect adjudication distance");
            }
            if proof.value == Value::Win(node.turn) {
                if values.iter().any(|&value| value != proof.value) {
                    return Err("winning choice includes an unproved action");
                }
            } else {
                if !node.closed || seen.len() != node.edges.len() {
                    return Err("proof omits a legal reply or open boundary");
                }
                // Compare integer extrema here rather than using the solver's
                // move-selection function.
                let scores: Vec<_> = values.iter().map(|value| value.score()).collect();
                let expected = match node.turn {
                    Player::Black => scores.iter().max(),
                    Player::White => scores.iter().min(),
                };
                if expected.copied() != Some(proof.value.score()) {
                    return Err("certificate minimax outcome mismatch");
                }
            }
        }
    }
    Ok(())
}

/// Choose a non-losing continuation from a verified proof and carry its child
/// certificate forward. A draw proof can also contain losing alternatives;
/// those must never be selected just because they are present in the tree.
///
/// Ties use distance within this retained proof only. Following the returned
/// certificate at the next position strictly decreases distance, including
/// across graph cycles. Re-solving and discarding the continuation would need
/// its own policy-consistency contract. A proven loss offers no safe action.
fn select_nonlosing_continuation<'a>(
    graph: &Graph,
    history: &History,
    proof: &'a Certificate,
) -> Result<Option<(usize, &'a Certificate)>, &'static str> {
    verify(graph, history, proof)?;
    if proof.value == Value::Win(graph.0[proof.node].turn.other()) {
        return Ok(None);
    }
    let ProofKind::Branch(children) = &proof.kind else {
        return Ok(None);
    };
    Ok(children
        .iter()
        .filter(|(_, child)| child.value == proof.value)
        .min_by_key(|(_, child)| child.plies_to_adjudication)
        .map(|(action, child)| (*action, child)))
}

mod shogi {
    use super::*;
    use crate::board::Board;
    use crate::color::Color;
    use crate::movegen::{generate_legal_moves, is_in_check};
    use crate::mv::Move;
    use crate::piece::PieceKind;
    use crate::sfen::{
        PositionHistory, RepetitionOutcome, board_to_sfen, move_from_usi, move_to_usi,
    };
    use crate::square::Square;
    use std::collections::HashMap;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    /// Whether a legal successor is represented by an exact stored position.
    pub enum Target {
        /// Index of the successor in the input node array.
        Known(NodeId),
        /// Explicit boundary that cannot be treated as a proven leaf.
        Unknown,
    }

    #[derive(Clone, Debug)]
    /// One canonical legal USI action with independently checked metadata.
    pub struct SuccessorInput {
        /// Canonical USI move string.
        pub action: String,
        /// Exact stored successor or an explicit unknown boundary.
        pub target: Target,
        /// Whether playing the action checks the new side to move.
        pub gave_check: bool,
    }

    #[derive(Clone, Debug)]
    /// Exact shogi position plus a complete partition of its legal moves.
    pub struct NodeInput {
        /// Position SFEN; ordinal move number is excluded from identity.
        pub sfen: String,
        /// Every legal move must occur exactly once, even at an open boundary.
        pub successors: Vec<SuccessorInput>,
    }

    pub(super) struct VerifiedGraph {
        graph: Graph,
        boards: Vec<Board>,
        /// String equality, not Zobrist equality, decides position identity.
        ids: HashMap<String, NodeId>,
        #[cfg(test)]
        unknown_actions: Vec<Vec<String>>,
    }

    struct VerifiedHistory {
        origin: String,
        actions: Vec<String>,
        board: Board,
        exact: History,
        engine: PositionHistory,
    }

    fn player(color: Color) -> Player {
        match color {
            Color::Black => Player::Black,
            Color::White => Player::White,
        }
    }

    /// Board, turn, and hands define a position; move number does not. The
    /// engine's serializer makes alternate hand-order spellings identical.
    fn exact_key(board: &Board) -> String {
        let mut normalized = board.clone();
        normalized.ply = 0;
        board_to_sfen(&normalized)
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Accept composed positions with missing non-king material, but reject
    /// malformed ordinary-shogi states before asking movegen for transitions.
    /// This is not a proof of reachability from the standard initial position.
    fn admitted_board(sfen: &str) -> Result<Board, String> {
        let mut board = Board::from_sfen(sfen)?;
        // Ordinal SFEN move numbers are not graph identity or proof distance.
        board.ply = 0;
        for color in [Color::Black, Color::White] {
            if board.pieces(color, PieceKind::Ou).popcount() != 1 {
                return Err("real graph requires exactly one king per side".into());
            }
            let king_rank = board.king_square(color).expect("one king checked").rank();
            if (color == Color::Black && king_rank <= 3)
                || (color == Color::White && king_rank >= 7)
            {
                return Err("king in opponent camp is outside the mate-only book contract".into());
            }
            let mut pawn_files = [false; 9];
            for index in 0..Square::NUM {
                let square = Square::from_index(index as u8);
                let Some(piece) = board.piece_at(square) else {
                    continue;
                };
                if piece.color != color {
                    continue;
                }
                let remaining = match color {
                    Color::Black => square.rank() - 1,
                    Color::White => 9 - square.rank(),
                };
                if (matches!(piece.kind, PieceKind::Fu | PieceKind::Kyou) && remaining == 0)
                    || (piece.kind == PieceKind::Kei && remaining < 2)
                {
                    return Err("unpromoted piece has no possible forward move".into());
                }
                if piece.kind == PieceKind::Fu {
                    let occupied = &mut pawn_files[square.file_0() as usize];
                    if *occupied {
                        return Err("two unpromoted pawns in one file".into());
                    }
                    *occupied = true;
                }
            }
        }
        for (kind, maximum) in [
            (PieceKind::Fu, 18),
            (PieceKind::Kyou, 4),
            (PieceKind::Kei, 4),
            (PieceKind::Gin, 4),
            (PieceKind::Kin, 4),
            (PieceKind::Kaku, 2),
            (PieceKind::Hisha, 2),
        ] {
            let count: u32 = [Color::Black, Color::White]
                .into_iter()
                .map(|color| {
                    let on_board: u32 = (0..PieceKind::COUNT)
                        .filter_map(|index| PieceKind::from_u8(index as u8))
                        .filter(|piece| piece.unpromoted() == kind)
                        .map(|piece| board.pieces(color, piece).popcount())
                        .sum();
                    on_board + u32::from(board.hand(color).get(kind))
                })
                .sum();
            if count > maximum {
                return Err("material exceeds ordinary shogi inventory".into());
            }
        }
        if is_in_check(&board, board.side_to_move.flip()) {
            return Err("non-moving king is already in check".into());
        }
        Ok(board)
    }

    fn engine_outcome(history: &PositionHistory) -> Option<Value> {
        history
            .outcome_at_current_position()
            .map(|outcome| match outcome {
                RepetitionOutcome::Draw => Value::Draw,
                RepetitionOutcome::PerpetualCheck(loser) => Value::Win(player(loser).other()),
            })
    }

    impl VerifiedGraph {
        pub(super) fn from_input(input: &[NodeInput]) -> Result<Self, String> {
            if input.is_empty() {
                return Err("empty real graph".into());
            }
            let boards = input
                .iter()
                .map(|node| admitted_board(&node.sfen))
                .collect::<Result<Vec<_>, _>>()?;
            let mut ids = HashMap::new();
            for (id, board) in boards.iter().enumerate() {
                if ids.insert(exact_key(board), id).is_some() {
                    return Err(
                        "duplicate exact position, including alternate SFEN move numbers".into(),
                    );
                }
            }
            let mut nodes = Vec::new();
            #[cfg(test)]
            let mut unknown_actions = Vec::new();
            for (id, source) in input.iter().enumerate() {
                let board = &boards[id];
                let legal = generate_legal_moves(&mut board.clone())
                    .into_iter()
                    .map(|mv| (move_to_usi(mv), mv))
                    .collect::<HashMap<_, _>>();
                let mut seen = HashSet::new();
                let mut edges = Vec::new();
                let mut unknown = Vec::new();
                for successor in &source.successors {
                    if !seen.insert(successor.action.as_str()) {
                        return Err(format!("duplicate legal successor at node {id}"));
                    }
                    let mv = *legal.get(&successor.action).ok_or_else(|| {
                        format!(
                            "illegal or noncanonical action at node {id}: {}",
                            successor.action
                        )
                    })?;
                    let mut after = board.clone();
                    after.do_move(mv);
                    let gave_check = is_in_check(&after, after.side_to_move);
                    if gave_check != successor.gave_check {
                        return Err(format!(
                            "false check label at node {id}: {}",
                            successor.action
                        ));
                    }
                    match successor.target {
                        Target::Known(target) => {
                            let expected = ids.get(&exact_key(&after)).copied();
                            if expected != Some(target) {
                                return Err(format!(
                                    "edge target is not the exact legal successor at node {id}: {}",
                                    successor.action
                                ));
                            }
                            edges.push(Edge {
                                action: successor.action.clone(),
                                to: target,
                                gave_check,
                            });
                        }
                        Target::Unknown => unknown.push(successor.action.clone()),
                    }
                }
                if seen.len() != legal.len() {
                    return Err(format!(
                        "node {id} omits legal successors; declare them Unknown"
                    ));
                }
                // Only a directly verified checkmate may become a static
                // terminal. Stalemate/entering-king/declaration adjudications
                // have no adapter contract here and stay open/Unknown.
                let terminal = (legal.is_empty() && is_in_check(board, board.side_to_move))
                    .then_some(Value::Win(player(board.side_to_move).other()));
                let closed = terminal.is_some() || (!legal.is_empty() && unknown.is_empty());
                nodes.push(Node {
                    turn: player(board.side_to_move),
                    edges,
                    closed,
                    terminal,
                });
                #[cfg(test)]
                unknown_actions.push(unknown);
            }
            let graph = Graph(nodes);
            graph.validate().map_err(str::to_owned)?;
            Ok(Self {
                graph,
                boards,
                ids,
                #[cfg(test)]
                unknown_actions,
            })
        }

        /// Replay the supplied full history independently with Board/movegen
        /// and PositionHistory, and compare it with exact graph identities.
        /// Missing historical positions or unrepresented played edges are an
        /// error; callers may include them using `inputs_for_positions`.
        fn replay_history(
            &self,
            origin: &str,
            actions: &[String],
        ) -> Result<VerifiedHistory, String> {
            let mut board = admitted_board(origin)?;
            let start = *self
                .ids
                .get(&exact_key(&board))
                .ok_or("history origin is outside graph")?;
            let mut exact = History::replay(&self.graph, start, &[]).map_err(str::to_owned)?;
            let mut engine = PositionHistory::initial(board.hash());
            for action in actions {
                if exact.repetition().is_some() || self.graph.0[exact.current()].terminal.is_some()
                {
                    return Err("real history continues after adjudication".into());
                }
                let mv = move_from_usi(action, &board)?;
                if move_to_usi(mv) != *action {
                    return Err("noncanonical history action".into());
                }
                let action_index = self.graph.0[exact.current()]
                    .edges
                    .iter()
                    .position(|edge| edge.action == *action)
                    .ok_or("played history edge is an Unknown boundary")?;
                let mover = board.side_to_move;
                board.do_move(mv);
                let gives_check = is_in_check(&board, board.side_to_move);
                exact.push(&self.graph, action_index);
                if self.ids.get(&exact_key(&board)).copied() != Some(exact.current()) {
                    return Err("real history disagrees with exact graph identity".into());
                }
                engine.push_after_move(board.hash(), mover, gives_check);
                if exact.repetition() != engine_outcome(&engine) {
                    return Err(
                        "engine hash repetition disagrees with exact-position repetition".into(),
                    );
                }
            }
            Ok(VerifiedHistory {
                origin: origin.into(),
                actions: actions.to_vec(),
                board,
                exact,
                engine,
            })
        }

        fn validate_history(&self, history: &VerifiedHistory) -> Result<(), String> {
            let replay = self.replay_history(&history.origin, &history.actions)?;
            if !same_visits(&replay.exact, &history.exact)
                || replay.exact.actions != history.exact.actions
                || exact_key(&replay.board) != exact_key(&history.board)
                || replay.engine != history.engine
            {
                return Err("verified history has been altered".into());
            }
            Ok(())
        }

        fn solve(&self, history: &VerifiedHistory, limits: Limits) -> Result<Verdict, String> {
            self.validate_history(history)?;
            super::solve(&self.graph, Some(&history.exact), limits).map_err(str::to_owned)
        }

        /// Revalidate the real root and selected move at the point of use;
        /// certificates from another board, hand, or history cannot select it.
        fn select<'a>(
            &self,
            history: &VerifiedHistory,
            current: &Board,
            proof: &'a Certificate,
        ) -> Result<Option<(Move, &'a Certificate)>, String> {
            self.validate_history(history)?;
            if exact_key(current) != exact_key(&history.board) {
                return Err("current board differs from the certified exact position".into());
            }
            let Some((action, child)) =
                select_nonlosing_continuation(&self.graph, &history.exact, proof)
                    .map_err(str::to_owned)?
            else {
                return Ok(None);
            };
            let edge = &self.graph.0[history.exact.current()].edges[action];
            let mut after = current.clone();
            after.ply = 0;
            let mv = move_from_usi(&edge.action, &after)?;
            after.do_move(mv);
            if exact_key(&after) != exact_key(&self.boards[edge.to])
                || is_in_check(&after, after.side_to_move) != edge.gave_check
            {
                return Err("selected move failed real transition verification".into());
            }
            Ok(Some((mv, child)))
        }

        pub(super) fn production_decision(
            &self,
            position: Option<&str>,
            current: &Board,
            limits: BookGraphLimits,
        ) -> BookGraphDecision {
            use BookGraphDecision::{Fallback, ProvenMate};
            use BookGraphFallback as Reason;
            if limits.max_depth == 0
                || limits.max_depth > MAX_PROOF_PLIES
                || limits.node_limit == 0
                || limits.node_limit > MAX_PROOF_NODES
            {
                return Fallback(Reason::InvalidLimits);
            }
            let Some(position) = position.filter(|body| body.len() <= 32_768) else {
                return Fallback(Reason::HistoryUnavailable);
            };
            let tokens = position.split_whitespace().collect::<Vec<_>>();
            let (origin, rest) = match tokens.as_slice() {
                ["startpos", rest @ ..] => (crate::sfen::STARTPOS_SFEN.to_owned(), rest),
                ["sfen", placement, turn, hand, "1", rest @ ..] => {
                    (format!("{placement} {turn} {hand} 1"), rest)
                }
                _ => return Fallback(Reason::HistoryUnavailable),
            };
            let actions = match rest {
                [] => vec![],
                ["moves", actions @ ..] if actions.len() <= MAX_HISTORY_PLIES => {
                    actions.iter().map(|action| (*action).to_owned()).collect()
                }
                _ => return Fallback(Reason::HistoryUnavailable),
            };
            let Ok(history) = self.replay_history(&origin, &actions) else {
                return Fallback(Reason::HistoryNotRepresented);
            };
            if exact_key(current) != exact_key(&history.board) {
                return Fallback(Reason::PositionMismatch);
            }
            let Ok(result) = self.solve(
                &history,
                Limits {
                    depth: limits.max_depth,
                    nodes: limits.node_limit,
                },
            ) else {
                return Fallback(Reason::InvalidCertificate);
            };
            let Verdict::Proven(proof) = result else {
                return Fallback(Reason::Unknown);
            };
            fn only_checkmate_leaves(proof: &Certificate) -> bool {
                match &proof.kind {
                    ProofKind::Terminal => true,
                    ProofKind::Repetition => false,
                    ProofKind::Branch(children) => children
                        .iter()
                        .all(|(_, child)| only_checkmate_leaves(child)),
                }
            }
            if proof.value != Value::Win(player(current.side_to_move))
                || !only_checkmate_leaves(&proof)
            {
                return Fallback(Reason::NonMateOutcome);
            }
            match self.select(&history, current, &proof) {
                Ok(Some((best_move, _))) => ProvenMate {
                    best_move,
                    proof_plies: proof.plies_to_adjudication,
                },
                _ => Fallback(Reason::InvalidCertificate),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        include!("../tests/support/book_graph_shogi_tests.rs");
    }
}

#[cfg(test)]
mod tests {
    include!("../tests/support/book_graph_generic_tests.rs");
}
