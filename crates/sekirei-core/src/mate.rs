//! Check-only mate solver (depth-first proof-number search).
//!
//! The attacker (side to move at the root) may only play checking moves;
//! the defender plays every legal reply. Proof and disproof numbers are
//! kept in a hash table and the most-proving child is expanded under
//! thresholds (Nagai's df-pn). Positions repeated on the current path count
//! as failed attacks, since perpetual check loses in shogi. The search is
//! bounded by a node limit and a ply limit and returns a proven first move.

use std::collections::HashMap;
use web_time::Instant;

use crate::board::Board;
use crate::color::Color;
use crate::movegen::{
    MoveBuffer, discovered_check_candidates, generate_legal_moves, is_in_check,
    move_gives_direct_check,
};
use crate::mv::Move;
use crate::piece::PieceKind;

const INF: u64 = 1 << 40;

/// Result of a bounded mate search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MateResult {
    /// First move of a proven forced mate, if one was found.
    pub mate_move: Option<Move>,
    /// Number of expanded nodes.
    pub nodes: u64,
}

/// Why a position cannot be treated as a mate-in-one problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MateInOneInvalidReason {
    /// The attacking side (the side to move) has no king.
    MissingAttackerKing,
    /// The defending side has no king.
    MissingDefenderKing,
    /// The attacking side has more than one king.
    MultipleAttackerKings,
    /// The defending side has more than one king.
    MultipleDefenderKings,
    /// The defending king is already in check in the initial position.
    DefenderAlreadyInCheck,
}

impl MateInOneInvalidReason {
    /// Stable machine-readable reason used by non-Rust adapters.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingAttackerKing => "missing_attacker_king",
            Self::MissingDefenderKing => "missing_defender_king",
            Self::MultipleAttackerKings => "multiple_attacker_kings",
            Self::MultipleDefenderKings => "multiple_defender_kings",
            Self::DefenderAlreadyInCheck => "defender_already_in_check",
        }
    }
}

/// King-presence policy used when validating a mate-problem position.
///
/// Ordinary game positions should use [`Self::RequireBothKings`]. Tsume
/// diagrams conventionally omit the attacking king, so callers that are
/// explicitly handling those diagrams may select
/// [`Self::AllowMissingAttackerKing`]. Neither policy relaxes validation of
/// the defending king, duplicate kings, or an initially checked defender.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatePositionPolicy {
    /// Require exactly one king for both the attacker and defender.
    #[default]
    RequireBothKings,
    /// Permit the attacking king to be absent, while allowing at most one.
    AllowMissingAttackerKing,
}

impl MatePositionPolicy {
    /// Stable machine-readable value used by non-Rust adapters.
    pub const fn code(self) -> &'static str {
        match self {
            Self::RequireBothKings => "both_kings_required",
            Self::AllowMissingAttackerKing => "attacker_king_optional",
        }
    }
}

/// Rule used to count a proven mate's length.
///
/// Sekirei currently proves mate against every legal defending reply. This
/// can be longer than a Japanese tsume-shogi problem's published move count,
/// because that convention omits futile interpositions (muda-ai).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MateDistanceRule {
    /// Count every legal defending reply, including futile interpositions.
    AllLegalDefenderReplies,
}

impl MateDistanceRule {
    /// Stable machine-readable value used by non-Rust adapters.
    pub const fn code(self) -> &'static str {
        match self {
            Self::AllLegalDefenderReplies => "all_legal_defender_replies",
        }
    }
}

/// Complete mate-in-one analysis for a problem position.
///
/// The side to move is always the attacker. Unlike the search-oriented
/// [`crate::movegen::mate_in_one`], this result checks every legal attacking
/// move and every legal defending reply, so it also covers distant slider
/// checks and discovered checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateInOneAnalysis {
    /// Side treated as the attacker.
    pub attacker: Color,
    /// Why the initial position is not a valid mate-in-one problem.
    pub invalid_reason: Option<MateInOneInvalidReason>,
    /// Every legal move that checkmates immediately.
    pub solutions: Vec<Move>,
}

/// Result category for a complete bounded tsume analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MateAnalysisOutcome {
    /// A forced mate was completely proven within the requested ply bound.
    Mate,
    /// The complete search proved that no forced mate exists within the bound.
    NoMate,
    /// The node budget expired before the requested bound was completely searched.
    Unknown,
}

impl MateAnalysisOutcome {
    /// Stable machine-readable value used by non-Rust adapters.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Mate => "mate",
            Self::NoMate => "no_mate",
            Self::Unknown => "unknown",
        }
    }
}

/// Why a bounded tsume analysis stopped before completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MateAnalysisAbortReason {
    /// The configured node limit was reached.
    NodeLimit,
}

impl MateAnalysisAbortReason {
    /// Stable machine-readable value used by non-Rust adapters.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NodeLimit => "node_limit",
        }
    }
}

/// Complete shortest-mate analysis under explicit ply and node limits.
///
/// The side to move is the attacker. The attacker may play only checking
/// moves, while the defender may choose any legal reply. `solutions` contains
/// every legal first move that forces mate in exactly `shortest_mate_ply`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateAnalysis {
    /// Side treated as the attacker.
    pub attacker: Color,
    /// Why the initial position is not a valid tsume problem.
    pub invalid_reason: Option<MateInOneInvalidReason>,
    /// Complete result under the requested limits.
    pub outcome: MateAnalysisOutcome,
    /// Shortest proof distance against every legal defending reply, in plies.
    ///
    /// This is not a tsume-shogi convention distance with futile
    /// interpositions removed. Prefer [`Self::shortest_proof_ply`] in new UI.
    pub shortest_mate_ply: Option<u32>,
    /// Every first move that forces mate at the shortest proven length.
    pub solutions: Vec<Move>,
    /// Number of positions visited across all iterative-deepening passes.
    pub nodes: u64,
    /// Whether a resource limit stopped the analysis.
    pub aborted: bool,
    /// Resource limit that stopped the analysis, when `aborted` is true.
    pub abort_reason: Option<MateAnalysisAbortReason>,
}

impl MateAnalysis {
    /// Whether the initial position satisfies the problem-position contract.
    pub fn valid_position(&self) -> bool {
        self.invalid_reason.is_none()
    }

    /// Whether the completed shortest-depth search found exactly one first move.
    pub fn unique_solution(&self) -> bool {
        self.outcome == MateAnalysisOutcome::Mate && !self.aborted && self.solutions.len() == 1
    }

    /// Rule used by [`Self::shortest_proof_ply`] to count defending replies.
    pub const fn distance_rule(&self) -> MateDistanceRule {
        MateDistanceRule::AllLegalDefenderReplies
    }

    /// Shortest proven distance against every legal defending reply.
    ///
    /// This is an explicit alias for [`Self::shortest_mate_ply`]. It is not a
    /// tsume-shogi convention distance with futile interpositions removed.
    pub const fn shortest_proof_ply(&self) -> Option<u32> {
        self.shortest_mate_ply
    }
}

impl MateInOneAnalysis {
    /// Whether the initial position satisfies the problem-position contract.
    pub fn valid_position(&self) -> bool {
        self.invalid_reason.is_none()
    }

    /// Whether the defender was already in check before the attacking move.
    pub fn defender_already_in_check(&self) -> bool {
        self.invalid_reason == Some(MateInOneInvalidReason::DefenderAlreadyInCheck)
    }

    /// Whether exactly one mate-in-one solution exists.
    pub fn unique_solution(&self) -> bool {
        self.solutions.len() == 1
    }
}

/// Analyze a position as a complete mate-in-one problem.
///
/// The board's side to move is the attacker. Invalid problem positions return
/// an empty solution list with an [`MateInOneInvalidReason`] instead of
/// panicking.
pub fn analyze_mate_in_one(board: &Board) -> MateInOneAnalysis {
    analyze_mate_in_one_with_policy(board, MatePositionPolicy::RequireBothKings)
}

/// Analyze a complete mate-in-one problem under an explicit king policy.
///
/// Use [`MatePositionPolicy::AllowMissingAttackerKing`] only for tsume
/// diagrams that intentionally omit the attacking king. If that king exists,
/// ordinary self-check and countercheck legality remains enforced by legal
/// move generation.
pub fn analyze_mate_in_one_with_policy(
    board: &Board,
    position_policy: MatePositionPolicy,
) -> MateInOneAnalysis {
    let attacker = board.side_to_move;
    let defender = attacker.flip();
    let invalid_reason = validate_problem_position(board, position_policy);

    if invalid_reason.is_some() {
        return MateInOneAnalysis {
            attacker,
            invalid_reason,
            solutions: Vec::new(),
        };
    }

    let mut position = board.clone();
    let legal = generate_legal_moves(&mut position);
    let mut solutions = Vec::new();
    for mv in legal {
        let token = position.do_move_for_search(mv);
        let mates =
            is_in_check(&position, defender) && generate_legal_moves(&mut position).is_empty();
        position.undo_move_for_search(token);
        if mates {
            solutions.push(mv);
        }
    }
    solutions.sort_unstable_by_key(|mv| mv.raw());

    MateInOneAnalysis {
        attacker,
        invalid_reason: None,
        solutions,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Proof {
    Mate,
    NoMate,
    Unknown,
}

struct CompleteMateSearch {
    attacker: Color,
    nodes: u64,
    node_limit: u64,
    deadline: Option<Instant>,
    aborted: bool,
    deadline_expired: bool,
    path: Vec<u64>,
}

impl CompleteMateSearch {
    fn visit(&mut self) -> bool {
        if self.nodes >= self.node_limit {
            self.aborted = true;
            return false;
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.aborted = true;
            self.deadline_expired = true;
            return false;
        }
        self.nodes += 1;
        true
    }

    fn checking_moves(&mut self, board: &mut Board) -> Vec<Move> {
        let defender = self.attacker.flip();
        let legal = generate_legal_moves(board);
        let mut checks = Vec::new();
        for mv in legal {
            let token = board.do_move_for_search(mv);
            let gives_check = is_in_check(board, defender);
            board.undo_move_for_search(token);
            if gives_check {
                checks.push(mv);
            }
        }
        checks.sort_unstable_by_key(|mv| mv.raw());
        checks
    }

    fn repeated(&self, board: &Board) -> bool {
        self.path.contains(&board.hash())
    }

    fn with_child(
        &mut self,
        board: &mut Board,
        mv: Move,
        search: impl FnOnce(&mut Self, &mut Board) -> Proof,
    ) -> Proof {
        let token = board.do_move_for_search(mv);
        let proof = if self.repeated(board) {
            Proof::NoMate
        } else {
            self.path.push(board.hash());
            let proof = search(self, board);
            self.path.pop();
            proof
        };
        board.undo_move_for_search(token);
        proof
    }

    fn attacker(&mut self, board: &mut Board, plies_left: u32) -> Proof {
        if !self.visit() {
            return Proof::Unknown;
        }
        if plies_left == 0 {
            return Proof::NoMate;
        }
        for mv in self.checking_moves(board) {
            let proof = self.with_child(board, mv, |state, child| {
                state.defender(child, plies_left - 1)
            });
            match proof {
                Proof::Mate => return Proof::Mate,
                Proof::Unknown => return Proof::Unknown,
                Proof::NoMate => {}
            }
        }
        Proof::NoMate
    }

    fn defender(&mut self, board: &mut Board, plies_left: u32) -> Proof {
        if !self.visit() {
            return Proof::Unknown;
        }
        let mut replies = generate_legal_moves(board);
        replies.sort_unstable_by_key(|mv| mv.raw());
        if replies.is_empty() {
            return if is_in_check(board, board.side_to_move) {
                Proof::Mate
            } else {
                Proof::NoMate
            };
        }
        if plies_left == 0 {
            return Proof::NoMate;
        }
        for mv in replies {
            let proof = self.with_child(board, mv, |state, child| {
                state.attacker(child, plies_left - 1)
            });
            match proof {
                Proof::NoMate => return Proof::NoMate,
                Proof::Unknown => return Proof::Unknown,
                Proof::Mate => {}
            }
        }
        Proof::Mate
    }

    fn root_solutions(&mut self, board: &mut Board, plies: u32) -> Result<Vec<Move>, ()> {
        if !self.visit() {
            return Err(());
        }
        let mut solutions = Vec::new();
        for mv in self.checking_moves(board) {
            let proof = self.with_child(board, mv, |state, child| state.defender(child, plies - 1));
            match proof {
                Proof::Mate => solutions.push(mv),
                Proof::NoMate => {}
                Proof::Unknown => return Err(()),
            }
        }
        Ok(solutions)
    }
}

fn validate_problem_position(
    board: &Board,
    position_policy: MatePositionPolicy,
) -> Option<MateInOneInvalidReason> {
    let attacker = board.side_to_move;
    let defender = attacker.flip();
    let attacker_kings = board.pieces(attacker, PieceKind::Ou).popcount();
    let defender_kings = board.pieces(defender, PieceKind::Ou).popcount();
    if attacker_kings == 0 && position_policy == MatePositionPolicy::RequireBothKings {
        Some(MateInOneInvalidReason::MissingAttackerKing)
    } else if defender_kings == 0 {
        Some(MateInOneInvalidReason::MissingDefenderKing)
    } else if attacker_kings > 1 {
        Some(MateInOneInvalidReason::MultipleAttackerKings)
    } else if defender_kings > 1 {
        Some(MateInOneInvalidReason::MultipleDefenderKings)
    } else if is_in_check(board, defender) {
        Some(MateInOneInvalidReason::DefenderAlreadyInCheck)
    } else {
        None
    }
}

/// Analyze the shortest forced mate up to `max_ply`, under `node_limit`.
///
/// Odd depths are searched in increasing order. A completed result therefore
/// proves both the shortest all-legal-reply distance and the complete set of
/// first moves at that distance. Futile interpositions are not omitted. If the
/// node budget expires, the result is `Unknown`; partial solutions are
/// discarded so callers never mistake an incomplete set for a unique
/// solution. Repetition on the current line is treated as a failed attack,
/// matching shogi perpetual-check rules.
pub fn analyze_mate(board: &Board, max_ply: u32, node_limit: u64) -> MateAnalysis {
    analyze_mate_with_policy(
        board,
        max_ply,
        node_limit,
        MatePositionPolicy::RequireBothKings,
    )
}

/// Analyze the shortest forced mate under an explicit king policy.
///
/// The returned distance always follows
/// [`MateDistanceRule::AllLegalDefenderReplies`]. In particular, selecting
/// [`MatePositionPolicy::AllowMissingAttackerKing`] changes only validation of
/// a deliberately omitted attacking king; it does not remove futile
/// interpositions or otherwise change legal reply generation.
pub fn analyze_mate_with_policy(
    board: &Board,
    max_ply: u32,
    node_limit: u64,
    position_policy: MatePositionPolicy,
) -> MateAnalysis {
    analyze_mate_with_policy_until(board, max_ply, node_limit, position_policy, None).0
}

/// Complete mate analysis with a shared deadline for bounded diagnostics.
///
/// The deadline is checked at each visited position. Move generation within
/// one position is non-preemptive, so elapsed time may slightly exceed it.
pub(crate) fn analyze_mate_with_policy_until(
    board: &Board,
    max_ply: u32,
    node_limit: u64,
    position_policy: MatePositionPolicy,
    deadline: Option<Instant>,
) -> (MateAnalysis, bool) {
    let attacker = board.side_to_move;
    let invalid_reason = validate_problem_position(board, position_policy);
    if invalid_reason.is_some() {
        return (
            MateAnalysis {
                attacker,
                invalid_reason,
                outcome: MateAnalysisOutcome::Unknown,
                shortest_mate_ply: None,
                solutions: Vec::new(),
                nodes: 0,
                aborted: false,
                abort_reason: None,
            },
            false,
        );
    }

    let mut position = board.clone();
    let mut search = CompleteMateSearch {
        attacker,
        nodes: 0,
        node_limit,
        deadline,
        aborted: false,
        deadline_expired: false,
        path: vec![board.hash()],
    };

    for plies in (1..=max_ply).step_by(2) {
        match search.root_solutions(&mut position, plies) {
            Ok(mut solutions) if !solutions.is_empty() => {
                solutions.sort_unstable_by_key(|mv| mv.raw());
                return (
                    MateAnalysis {
                        attacker,
                        invalid_reason: None,
                        outcome: MateAnalysisOutcome::Mate,
                        shortest_mate_ply: Some(plies),
                        solutions,
                        nodes: search.nodes,
                        aborted: false,
                        abort_reason: None,
                    },
                    false,
                );
            }
            Ok(_) => {}
            Err(()) => {
                return (
                    MateAnalysis {
                        attacker,
                        invalid_reason: None,
                        outcome: MateAnalysisOutcome::Unknown,
                        shortest_mate_ply: None,
                        solutions: Vec::new(),
                        nodes: search.nodes,
                        aborted: true,
                        abort_reason: (!search.deadline_expired)
                            .then_some(MateAnalysisAbortReason::NodeLimit),
                    },
                    search.deadline_expired,
                );
            }
        }
    }

    (
        MateAnalysis {
            attacker,
            invalid_reason: None,
            outcome: MateAnalysisOutcome::NoMate,
            shortest_mate_ply: None,
            solutions: Vec::new(),
            nodes: search.nodes,
            aborted: false,
            abort_reason: None,
        },
        false,
    )
}

struct Solver {
    table: HashMap<u64, (u64, u64)>,
    path: Vec<u64>,
    nodes: u64,
    node_limit: u64,
    max_ply: u32,
    deadline: Option<Instant>,
    aborted: bool,
}

/// Search for a forced mate by the side to move within the given budgets.
/// `deadline`, when set, stops the search once passed.
pub fn solve_mate(
    board: &Board,
    node_limit: u64,
    max_ply: u32,
    deadline: Option<Instant>,
) -> MateResult {
    let mut solver = Solver {
        table: HashMap::new(),
        path: Vec::new(),
        nodes: 0,
        node_limit,
        max_ply,
        deadline,
        aborted: false,
    };
    let mut root = board.clone();
    solver.mid(&mut root, INF - 1, INF - 1, true, 0);
    let mut mate_move = None;
    if solver.lookup(root.hash()).0 == 0 {
        for m in solver.children(&mut root, true) {
            let tok = root.do_move_for_search(m);
            let proven = solver.lookup(root.hash()).0 == 0;
            root.undo_move_for_search(tok);
            if proven {
                mate_move = Some(m);
                break;
            }
        }
    }
    MateResult {
        mate_move,
        nodes: solver.nodes,
    }
}

impl Solver {
    fn lookup(&self, hash: u64) -> (u64, u64) {
        self.table.get(&hash).copied().unwrap_or((1, 1))
    }

    /// Attacker: checking moves only. Defender: every legal reply.
    fn children(&self, board: &mut Board, attacker: bool) -> Vec<Move> {
        let legal = MoveBuffer::legal(board);
        if !attacker {
            return legal.as_slice().to_vec();
        }
        let discoverers = discovered_check_candidates(board);
        let mut checks = Vec::new();
        for &m in legal.as_slice() {
            if move_gives_direct_check(board, m) {
                checks.push(m);
            } else if m.from.is_some_and(|from| discoverers.contains(from)) {
                let tok = board.do_move_for_search(m);
                let check = is_in_check(board, board.side_to_move);
                board.undo_move_for_search(tok);
                if check {
                    checks.push(m);
                }
            }
        }
        checks
    }

    fn mid(&mut self, board: &mut Board, th_pn: u64, th_dn: u64, attacker: bool, ply: u32) {
        let hash = board.hash();
        self.nodes += 1;
        if self.nodes > self.node_limit
            || (self.nodes.is_multiple_of(64) && self.deadline.is_some_and(|d| Instant::now() >= d))
        {
            self.aborted = true;
        }
        if self.aborted {
            return;
        }
        let moves = self.children(board, attacker);
        if moves.is_empty() {
            // No checks: the attack fails. No replies to a check: mate.
            let value = if attacker { (INF, 0) } else { (0, INF) };
            self.table.insert(hash, value);
            return;
        }
        if attacker && ply >= self.max_ply {
            self.table.insert(hash, (INF, 0));
            return;
        }
        let child_hashes: Vec<u64> = moves
            .iter()
            .map(|&m| {
                let tok = board.do_move_for_search(m);
                let h = board.hash();
                board.undo_move_for_search(tok);
                h
            })
            .collect();
        self.path.push(hash);
        loop {
            let mut best = 0;
            let mut min = INF;
            let mut second = INF;
            let mut sum = 0u64;
            for (i, &child) in child_hashes.iter().enumerate() {
                // A repetition on the path is a failed attack either way.
                let (cp, cd) = if self.path.contains(&child) {
                    (INF, 0)
                } else {
                    self.lookup(child)
                };
                let (selected, summed) = if attacker { (cp, cd) } else { (cd, cp) };
                if selected < min {
                    second = min;
                    min = selected;
                    best = i;
                } else if selected < second {
                    second = selected;
                }
                sum = (sum + summed).min(INF);
            }
            let (pn, dn) = if attacker { (min, sum) } else { (sum, min) };
            if pn >= th_pn || dn >= th_dn || self.aborted {
                self.table.insert(hash, (pn, dn));
                break;
            }
            let (cp, cd) = self.lookup(child_hashes[best]);
            let (child_pn, child_dn) = if attacker {
                (th_pn.min(second.saturating_add(1)), th_dn - dn + cd)
            } else {
                (th_pn - pn + cp, th_dn.min(second.saturating_add(1)))
            };
            let tok = board.do_move_for_search(moves[best]);
            self.mid(board, child_pn, child_dn, !attacker, ply + 1);
            board.undo_move_for_search(tok);
        }
        self.path.pop();
    }
}

/// A check-only mating strategy: one checking move at each attacker node and
/// every legal answer at each defender node. Empty defender replies are a
/// checked terminal mate, including otherwise futile interpositions upstream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MateProofTree {
    /// The attacker chooses one legal check.
    Attack {
        /// Legal checking move chosen by the attacker.
        mv: Move,
        /// Defender node after the check.
        next: Box<MateProofTree>,
    },
    /// The defender may choose any of these legal replies.
    Defense {
        /// All legal replies and their checked continuations.
        replies: Vec<(Move, MateProofTree)>,
    },
}

impl MateProofTree {
    /// One legal representative line, choosing the first defender reply.
    pub fn representative_line(&self) -> Vec<Move> {
        match self {
            Self::Attack { mv, next } => {
                let mut line = vec![*mv];
                line.extend(next.representative_line());
                line
            }
            Self::Defense { replies } => replies.first().map_or_else(Vec::new, |(mv, next)| {
                let mut line = vec![*mv];
                line.extend(next.representative_line());
                line
            }),
        }
    }

    /// Longest proven branch in plies. This does not prove shortest mate.
    pub fn longest_line_ply(&self) -> u32 {
        match self {
            Self::Attack { next, .. } => 1 + next.longest_line_ply(),
            Self::Defense { replies } => replies
                .iter()
                .map(|(_, next)| 1 + next.longest_line_ply())
                .max()
                .unwrap_or(0),
        }
    }
}

/// Why a bounded existence proof did not finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MateProofStop {
    /// The search node budget was exhausted.
    SearchNodes,
    /// The search deadline was reached.
    SearchDeadline,
    /// The search ended without a verified mate; no-mate is not claimed.
    SearchNoProof,
    /// The proof-table extraction node budget was exhausted.
    ExtractionNodes,
    /// The proof table did not contain a complete bounded strategy.
    ExtractionIncomplete,
    /// The independent replay node budget was exhausted.
    VerificationNodes,
    /// Legal replay rejected the extracted tree.
    VerificationFailed,
}

/// Separate limits for fast search, proof-table extraction, and legal replay.
#[derive(Clone, Copy, Debug)]
pub struct MateProofLimits {
    /// Maximum visited positions in the fast df-pn search.
    pub search_nodes: u64,
    /// Maximum plies in any proved line.
    pub max_ply: u32,
    /// Optional absolute search deadline.
    pub deadline: Option<Instant>,
    /// Maximum positions examined while extracting the candidate tree.
    pub extraction_nodes: u64,
    /// Maximum positions checked while independently replaying the tree.
    pub verification_nodes: u64,
}

/// A bounded existence proof. A present tree has been replayed across every
/// legal defense; absence never means a proven no-mate result.
#[derive(Clone, Debug)]
pub struct MateExistenceProof {
    /// Invalid problem position, if any.
    pub invalid_reason: Option<MateInOneInvalidReason>,
    /// Why no verified proof was returned.
    pub stop_reason: Option<MateProofStop>,
    /// Verified tree, present only after complete extraction and replay.
    pub tree: Option<MateProofTree>,
    /// Actual search positions visited.
    pub search_nodes: u64,
    /// Actual extraction positions visited.
    pub extraction_nodes: u64,
    /// Actual verification positions visited.
    pub verification_nodes: u64,
    /// The limits supplied by the caller.
    pub limits: MateProofLimits,
}

/// Result of replaying a proof tree against freshly generated legal moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateProofVerification {
    /// Whether every branch reached a checked no-reply leaf.
    pub verified: bool,
    /// Invalid initial position, if any.
    pub invalid_reason: Option<MateInOneInvalidReason>,
    /// Replay positions visited; never exceeds the supplied node limit.
    pub nodes: u64,
    /// Why replay did not verify the tree.
    pub stop_reason: Option<MateProofStop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeWorkStop {
    Limit,
    Invalid,
}

#[derive(Clone, Copy)]
struct ReplayLimits {
    nodes: u64,
    max_ply: u32,
}

/// Find and independently verify one bounded check-only mating strategy.
///
/// The fast solver's table only nominates a candidate. A separate replay
/// checks legal attacks, every legal defense, checkmate leaves, path
/// repetition, the ply limit, and board restoration. No shortest-distance or
/// no-mate assertion is made. This API preserves [`solve_mate`] unchanged.
pub fn prove_mate_existence(
    board: &Board,
    limits: MateProofLimits,
    position_policy: MatePositionPolicy,
) -> MateExistenceProof {
    let mut result = MateExistenceProof {
        invalid_reason: validate_problem_position(board, position_policy),
        stop_reason: None,
        tree: None,
        search_nodes: 0,
        extraction_nodes: 0,
        verification_nodes: 0,
        limits,
    };
    if result.invalid_reason.is_some() {
        return result;
    }
    if limits.search_nodes == 0 {
        result.stop_reason = Some(MateProofStop::SearchNodes);
        return result;
    }
    if limits
        .deadline
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        result.stop_reason = Some(MateProofStop::SearchDeadline);
        return result;
    }
    let mut solver = Solver {
        table: HashMap::new(),
        path: Vec::new(),
        nodes: 0,
        // Legacy Solver counts the rejected visit. Reserve that one visit so
        // this new API never reports more than the caller's search budget.
        node_limit: limits.search_nodes - 1,
        max_ply: limits.max_ply,
        deadline: limits.deadline,
        aborted: false,
    };
    let mut position = board.clone();
    solver.mid(&mut position, INF - 1, INF - 1, true, 0);
    result.search_nodes = solver.nodes;
    if solver.aborted {
        result.stop_reason = Some(if solver.nodes > solver.node_limit {
            MateProofStop::SearchNodes
        } else {
            MateProofStop::SearchDeadline
        });
        return result;
    }
    if limits
        .deadline
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        // The legacy solver samples its deadline only every 64 visits.
        // Never expose a proof completed after the caller's search deadline.
        result.stop_reason = Some(MateProofStop::SearchDeadline);
        return result;
    }
    if solver.lookup(position.hash()).0 != 0 {
        result.stop_reason = Some(MateProofStop::SearchNoProof);
        return result;
    }
    let candidate = extract_proof_tree(
        &solver,
        &mut position,
        true,
        0,
        &mut vec![board.hash()],
        &mut result.extraction_nodes,
        limits.extraction_nodes,
    );
    let tree = match candidate {
        Ok(tree) => tree,
        Err(TreeWorkStop::Limit) => {
            result.stop_reason = Some(MateProofStop::ExtractionNodes);
            return result;
        }
        Err(TreeWorkStop::Invalid) => {
            result.stop_reason = Some(MateProofStop::ExtractionIncomplete);
            return result;
        }
    };
    let verification = verify_mate_proof_tree(
        board,
        &tree,
        limits.max_ply,
        limits.verification_nodes,
        position_policy,
    );
    result.verification_nodes = verification.nodes;
    if verification.verified {
        result.tree = Some(tree);
    } else {
        result.stop_reason = verification.stop_reason;
    }
    result
}

/// Replay a claimed strategy with a separate node limit. The verifier
/// regenerates legal moves and does not consult the fast solver's proof table.
pub fn verify_mate_proof_tree(
    board: &Board,
    tree: &MateProofTree,
    max_ply: u32,
    node_limit: u64,
    position_policy: MatePositionPolicy,
) -> MateProofVerification {
    let invalid_reason = validate_problem_position(board, position_policy);
    if invalid_reason.is_some() {
        return MateProofVerification {
            verified: false,
            invalid_reason,
            nodes: 0,
            stop_reason: None,
        };
    }
    let mut position = board.clone();
    let mut nodes = 0;
    let checked = verify_proof_tree(
        tree,
        &mut position,
        true,
        0,
        &mut vec![board.hash()],
        &mut nodes,
        ReplayLimits {
            nodes: node_limit,
            max_ply,
        },
    );
    let verified = checked.is_ok();
    let stop_reason = match checked {
        Ok(()) => None,
        Err(TreeWorkStop::Limit) => Some(MateProofStop::VerificationNodes),
        Err(TreeWorkStop::Invalid) => Some(MateProofStop::VerificationFailed),
    };
    MateProofVerification {
        verified,
        invalid_reason: None,
        nodes,
        stop_reason,
    }
}

fn with_proof_child<T>(
    board: &mut Board,
    mv: Move,
    path: &mut Vec<u64>,
    step: impl FnOnce(&mut Board, &mut Vec<u64>) -> Result<T, TreeWorkStop>,
) -> Result<T, TreeWorkStop> {
    let token = board.do_move_for_search(mv);
    let hash = board.hash();
    let outcome = if path.contains(&hash) {
        Err(TreeWorkStop::Invalid)
    } else {
        path.push(hash);
        let outcome = step(board, path);
        path.pop();
        outcome
    };
    board.undo_move_for_search(token);
    outcome
}

fn extract_proof_tree(
    solver: &Solver,
    board: &mut Board,
    attacker: bool,
    ply: u32,
    path: &mut Vec<u64>,
    nodes: &mut u64,
    limit: u64,
) -> Result<MateProofTree, TreeWorkStop> {
    if *nodes >= limit {
        return Err(TreeWorkStop::Limit);
    }
    *nodes += 1;
    if ply > solver.max_ply || solver.lookup(board.hash()).0 != 0 {
        return Err(TreeWorkStop::Invalid);
    }
    if attacker {
        if ply >= solver.max_ply {
            return Err(TreeWorkStop::Invalid);
        }
        for mv in solver.children(board, true) {
            let outcome = with_proof_child(board, mv, path, |board, path| {
                extract_proof_tree(solver, board, false, ply + 1, path, nodes, limit)
            });
            match outcome {
                Ok(next) => {
                    return Ok(MateProofTree::Attack {
                        mv,
                        next: Box::new(next),
                    });
                }
                Err(TreeWorkStop::Limit) => return Err(TreeWorkStop::Limit),
                Err(TreeWorkStop::Invalid) => {}
            }
        }
        Err(TreeWorkStop::Invalid)
    } else {
        let mut legal = generate_legal_moves(board);
        legal.sort_unstable_by_key(|mv| mv.raw());
        if legal.is_empty() {
            return is_in_check(board, board.side_to_move)
                .then_some(MateProofTree::Defense {
                    replies: Vec::new(),
                })
                .ok_or(TreeWorkStop::Invalid);
        }
        if ply >= solver.max_ply {
            return Err(TreeWorkStop::Invalid);
        }
        let mut replies = Vec::with_capacity(legal.len());
        for mv in legal {
            let next = with_proof_child(board, mv, path, |board, path| {
                extract_proof_tree(solver, board, true, ply + 1, path, nodes, limit)
            })?;
            replies.push((mv, next));
        }
        Ok(MateProofTree::Defense { replies })
    }
}

fn verify_proof_tree(
    tree: &MateProofTree,
    board: &mut Board,
    attacker: bool,
    ply: u32,
    path: &mut Vec<u64>,
    nodes: &mut u64,
    limits: ReplayLimits,
) -> Result<(), TreeWorkStop> {
    if *nodes >= limits.nodes {
        return Err(TreeWorkStop::Limit);
    }
    *nodes += 1;
    if ply > limits.max_ply {
        return Err(TreeWorkStop::Invalid);
    }
    match (attacker, tree) {
        (true, MateProofTree::Attack { mv, next }) => {
            if ply >= limits.max_ply || !generate_legal_moves(board).contains(mv) {
                return Err(TreeWorkStop::Invalid);
            }
            with_proof_child(board, *mv, path, |board, path| {
                if !is_in_check(board, board.side_to_move) {
                    return Err(TreeWorkStop::Invalid);
                }
                verify_proof_tree(next, board, false, ply + 1, path, nodes, limits)
            })
        }
        (false, MateProofTree::Defense { replies }) => {
            let mut legal = generate_legal_moves(board);
            legal.sort_unstable_by_key(|mv| mv.raw());
            if legal.len() != replies.len()
                || !legal
                    .iter()
                    .zip(replies)
                    .all(|(expected, (actual, _))| expected == actual)
            {
                return Err(TreeWorkStop::Invalid);
            }
            if legal.is_empty() {
                return is_in_check(board, board.side_to_move)
                    .then_some(())
                    .ok_or(TreeWorkStop::Invalid);
            }
            if ply >= limits.max_ply {
                return Err(TreeWorkStop::Invalid);
            }
            for (mv, next) in replies {
                with_proof_child(board, *mv, path, |board, path| {
                    verify_proof_tree(next, board, true, ply + 1, path, nodes, limits)
                })?;
            }
            Ok(())
        }
        _ => Err(TreeWorkStop::Invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sfen::{move_from_usi, move_to_usi};

    const ALREADY_CHECKED_SFEN: &str = "4k4/2S3S2/3S1S3/4R4/9/9/9/9/4K4 b - 1";
    const DISTANT_ROOK_MATE_SFEN: &str = "4k4/2S3S2/2SGpGS2/9/4R4/9/9/9/4K4 b - 1";
    const SHORTEST_THREE_SFEN: &str = "4k4/9/2G3S2/5R3/2GG5/9/9/9/4K4 b - 1";
    // This position was generated by a deterministic legal playout and then
    // independently verified by the complete analyzer below.
    const UNIQUE_FIVE_SFEN: &str =
        "lnS1k2nl/3sr1G2/2p+Pb3p/p3Sp1p1/PpP2P1P1/5Gp2/LPN2gP1P/2+p+pr1G1L/Kb2S2N1 w P 110";
    const MISLABELLED_FIVE_SFEN: &str = "2S1k4/2G6/5R3/9/2RG5/9/9/9/4K4 b - 1";
    const SINGLE_KING_SEVEN_SFEN: &str = "6p1p/5g3/6kBR/9/6Ns1/9/9/9/9 b rb3g3s3n4l16p 1";
    const FUTILE_INTERPOSITION_ELEVEN_SFEN: &str =
        "6s2/7Pk/5B3/4+B4/7N1/9/9/9/K8 b 2r4g3s3n4l17p 1";
    const FUTILE_INTERPOSITION_NINE_SFEN: &str = "9/6p1k/7P1/6nB1/5R3/7L1/9/9/K8 b rb4g4s3n3l16p 1";

    fn proof_limits(max_ply: u32, search_nodes: u64) -> MateProofLimits {
        MateProofLimits {
            search_nodes,
            max_ply,
            deadline: None,
            extraction_nodes: 20_000,
            verification_nodes: 20_000,
        }
    }

    #[test]
    fn existence_proof_replays_one_ply_and_respects_separate_limits() {
        let board = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b G 1").unwrap();
        let before = crate::sfen::board_to_sfen(&board);
        let verified = prove_mate_existence(
            &board,
            proof_limits(1, 100_000),
            MatePositionPolicy::RequireBothKings,
        );
        assert_eq!(verified.stop_reason, None);
        let tree = verified.tree.unwrap();
        assert_eq!(tree.longest_line_ply(), 1);
        assert_eq!(tree.representative_line().len(), 1);
        assert!(
            verify_mate_proof_tree(&board, &tree, 1, 100, MatePositionPolicy::RequireBothKings,)
                .verified
        );
        assert_eq!(
            verify_mate_proof_tree(
                &board,
                &MateProofTree::Defense {
                    replies: Vec::new()
                },
                1,
                100,
                MatePositionPolicy::RequireBothKings,
            )
            .stop_reason,
            Some(MateProofStop::VerificationFailed),
        );
        assert_eq!(crate::sfen::board_to_sfen(&board), before);

        let mut cut = proof_limits(1, 100_000);
        cut.extraction_nodes = 0;
        let result = prove_mate_existence(&board, cut, MatePositionPolicy::RequireBothKings);
        assert_eq!(result.stop_reason, Some(MateProofStop::ExtractionNodes));
        assert!(result.tree.is_none());
        cut.extraction_nodes = 100;
        cut.verification_nodes = 0;
        let result = prove_mate_existence(&board, cut, MatePositionPolicy::RequireBothKings);
        assert_eq!(result.stop_reason, Some(MateProofStop::VerificationNodes));
        assert!(result.tree.is_none());
        cut.search_nodes = 0;
        let result = prove_mate_existence(&board, cut, MatePositionPolicy::RequireBothKings);
        assert_eq!(result.stop_reason, Some(MateProofStop::SearchNodes));
        assert_eq!(result.search_nodes, 0);
        cut.search_nodes = 1;
        let result = prove_mate_existence(&board, cut, MatePositionPolicy::RequireBothKings);
        assert_eq!(result.stop_reason, Some(MateProofStop::SearchNodes));
        assert_eq!(result.search_nodes, 1);
    }

    #[test]
    fn existence_proof_validates_problem_position_and_deadline() {
        let missing = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/9 b G 1").unwrap();
        let invalid = prove_mate_existence(
            &missing,
            proof_limits(1, 100),
            MatePositionPolicy::RequireBothKings,
        );
        assert_eq!(
            invalid.invalid_reason,
            Some(MateInOneInvalidReason::MissingAttackerKing)
        );
        assert_eq!(invalid.search_nodes, 0);
        let forged = MateProofTree::Defense {
            replies: Vec::new(),
        };
        let invalid_replay = verify_mate_proof_tree(
            &missing,
            &forged,
            1,
            100,
            MatePositionPolicy::RequireBothKings,
        );
        assert!(!invalid_replay.verified);
        assert_eq!(
            invalid_replay.invalid_reason,
            Some(MateInOneInvalidReason::MissingAttackerKing)
        );
        assert_eq!(invalid_replay.nodes, 0);

        let no_proof = prove_mate_existence(
            &Board::startpos(),
            proof_limits(1, 100),
            MatePositionPolicy::RequireBothKings,
        );
        assert_eq!(no_proof.stop_reason, Some(MateProofStop::SearchNoProof));
        assert!(no_proof.tree.is_none());

        let mut cut = proof_limits(1, 100);
        cut.deadline = Some(Instant::now());
        let timed = prove_mate_existence(
            &Board::startpos(),
            cut,
            MatePositionPolicy::RequireBothKings,
        );
        assert_eq!(timed.stop_reason, Some(MateProofStop::SearchDeadline));
        assert_eq!(timed.search_nodes, 0);
    }

    #[test]
    fn existence_proof_handles_three_public_diagrams() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                let cases = [
                    ("6knl/4+R4/8p/8g/7rb/6p2/9/9/9 b SLb3g3s3n2l16p 1", "L*3e"),
                    ("5n2k/4+b3l/4+R4/9/9/6N2/9/9/9 b BSNr4g3sn3l18p 1", "N*2c"),
                    ("9/2l6/r2G1B3/2k6/1R7/9/2+B6/9/9 b GN2g4s3n3l18p 1", "6c7c"),
                ];
                for (sfen, first) in cases {
                    let board = Board::from_sfen(sfen).unwrap();
                    let proof = prove_mate_existence(
                        &board,
                        proof_limits(17, 1_000_000),
                        MatePositionPolicy::AllowMissingAttackerKing,
                    );
                    assert_eq!(proof.stop_reason, None, "{sfen}: {proof:?}");
                    let tree = proof.tree.expect("verified proof");
                    assert_eq!(move_to_usi(tree.representative_line()[0]), first);
                    assert!(tree.longest_line_ply() <= 17);
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn existence_proof_replays_white_drop_and_promotion() {
        let white = Board::from_sfen("9/9/9/9/9/9/4p4/9/4K4 w g 1").unwrap();
        let proof = prove_mate_existence(
            &white,
            proof_limits(1, 100_000),
            MatePositionPolicy::AllowMissingAttackerKing,
        );
        assert_eq!(proof.stop_reason, None);
        let tree = proof.tree.expect("white proof");
        assert_eq!(move_to_usi(tree.representative_line()[0]), "G*5h");
        assert!(
            verify_mate_proof_tree(
                &white,
                &tree,
                1,
                100,
                MatePositionPolicy::AllowMissingAttackerKing,
            )
            .verified
        );

        let promotion = Board::from_sfen(DISTANT_ROOK_MATE_SFEN).unwrap();
        let proof = prove_mate_existence(
            &promotion,
            proof_limits(1, 100_000),
            MatePositionPolicy::RequireBothKings,
        );
        assert_eq!(proof.stop_reason, None);
        assert!(proof.tree.is_some());
        for first in ["5e5c", "5e5c+"] {
            let forged = MateProofTree::Attack {
                mv: move_from_usi(first, &promotion).unwrap(),
                next: Box::new(MateProofTree::Defense { replies: vec![] }),
            };
            assert!(
                verify_mate_proof_tree(
                    &promotion,
                    &forged,
                    1,
                    100,
                    MatePositionPolicy::RequireBothKings,
                )
                .verified,
                "{first}"
            );
        }
    }

    #[test]
    fn verification_rejects_illegal_pawn_drop_mate_and_missing_reply() {
        let pawn = Board::from_sfen("8k/9/7G1/9/9/9/9/9/9 b P 1").unwrap();
        let forged = MateProofTree::Attack {
            mv: Move::drop(crate::square::Square::from_shogi(1, 2), PieceKind::Fu),
            next: Box::new(MateProofTree::Defense { replies: vec![] }),
        };
        let result = verify_mate_proof_tree(
            &pawn,
            &forged,
            1,
            100,
            MatePositionPolicy::AllowMissingAttackerKing,
        );
        assert_eq!(result.stop_reason, Some(MateProofStop::VerificationFailed));

        let start = Board::startpos();
        let non_check = MateProofTree::Attack {
            mv: move_from_usi("7g7f", &start).unwrap(),
            next: Box::new(MateProofTree::Defense {
                replies: Vec::new(),
            }),
        };
        assert_eq!(
            verify_mate_proof_tree(
                &start,
                &non_check,
                1,
                100,
                MatePositionPolicy::RequireBothKings,
            )
            .stop_reason,
            Some(MateProofStop::VerificationFailed),
        );

        let board = Board::from_sfen(SHORTEST_THREE_SFEN).unwrap();
        let proof = prove_mate_existence(
            &board,
            proof_limits(3, 100_000),
            MatePositionPolicy::RequireBothKings,
        );
        assert_eq!(proof.stop_reason, None);
        let mut tree = proof.tree.expect("three-ply proof");
        let MateProofTree::Attack { ref mut next, .. } = tree else {
            panic!("root must attack")
        };
        let MateProofTree::Defense { ref mut replies } = **next else {
            panic!("defender must have replies")
        };
        assert!(!replies.is_empty());
        replies.pop();
        let result =
            verify_mate_proof_tree(&board, &tree, 3, 100, MatePositionPolicy::RequireBothKings);
        assert_eq!(result.stop_reason, Some(MateProofStop::VerificationFailed));
    }

    #[test]
    fn proof_replay_rejects_repeated_path_and_restores_board() {
        let mut board = Board::startpos();
        let mv = move_from_usi("7g7f", &board).unwrap();
        let before = crate::sfen::board_to_sfen(&board);
        let mut after = board.clone();
        after.do_move_for_search(mv);
        let mut path = vec![board.hash(), after.hash()];
        let outcome: Result<(), TreeWorkStop> =
            with_proof_child(&mut board, mv, &mut path, |_, _| {
                panic!("repeated position must not be replayed")
            });
        assert_eq!(outcome, Err(TreeWorkStop::Invalid));
        assert_eq!(crate::sfen::board_to_sfen(&board), before);
        assert_eq!(path.len(), 2);
    }

    fn solution_strings(sfen: &str) -> Vec<String> {
        let board = Board::from_sfen(sfen).expect("fixture must parse");
        analyze_mate_in_one(&board)
            .solutions
            .into_iter()
            .map(move_to_usi)
            .collect()
    }

    fn assert_mates(sfen: &str) {
        let board = Board::from_sfen(sfen).unwrap();
        let result = solve_mate(&board, 100_000, 15, None);
        let m = result.mate_move.expect("mate expected");
        let mut after = board.clone();
        after.do_move_for_search(m);
        assert!(is_in_check(&after, after.side_to_move));
    }

    #[test]
    fn finds_mate_in_one() {
        assert_mates("k8/2K6/9/9/4R4/9/9/9/9 b - 1");
    }

    #[test]
    fn finds_head_gold_drop_mate() {
        // Gold drop on the king's head, supported by a pawn.
        assert_mates("4k4/9/4P4/9/9/9/9/9/4K4 b G 1");
    }

    #[test]
    fn discovered_check_candidates_find_the_single_blocker() {
        // Black rook 5i behind a black silver 5e on the white king's file.
        let board = Board::from_sfen("4k4/9/9/9/4S4/9/9/9/4R4 b - 1").unwrap();
        let candidates = discovered_check_candidates(&board);
        assert!(candidates.contains(crate::square::Square::from_shogi(5, 5)));
        assert_eq!(candidates.popcount(), 1);
        // Every silver move leaves the file or stays on it; the solver's
        // check filter must agree with playing the move.
        let mut probe = board.clone();
        for m in generate_legal_moves(&mut probe) {
            let direct = move_gives_direct_check(&board, m);
            let mut after = board.clone();
            after.do_move_for_search(m);
            let check = is_in_check(&after, after.side_to_move);
            if check && !direct {
                assert!(m.from.is_some_and(|f| candidates.contains(f)));
            }
        }
    }

    #[test]
    fn attacker_moves_are_exactly_the_checks() {
        let solver = Solver {
            table: HashMap::new(),
            path: Vec::new(),
            nodes: 0,
            node_limit: 0,
            max_ply: 0,
            deadline: None,
            aborted: false,
        };
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let mut compared = 0;
        for _ in 0..100 {
            let mut board = Board::startpos();
            for _ in 0..(20 + rand(120)) {
                let moves = generate_legal_moves(&mut board);
                if moves.is_empty() {
                    break;
                }
                let mut expected: Vec<Move> = moves
                    .iter()
                    .copied()
                    .filter(|&m| {
                        let mut after = board.clone();
                        after.do_move_for_search(m);
                        is_in_check(&after, after.side_to_move)
                    })
                    .collect();
                let mut got = solver.children(&mut board, true);
                expected.sort_by_key(|m| format!("{m:?}"));
                got.sort_by_key(|m| format!("{m:?}"));
                assert_eq!(got, expected);
                compared += 1;
                board.do_move(moves[rand(moves.len())]);
            }
        }
        assert!(compared > 1000);
    }

    #[test]
    fn no_mate_without_checks() {
        let board = Board::startpos();
        let result = solve_mate(&board, 10_000, 15, None);
        assert_eq!(result.mate_move, None);
        let mut b = board.clone();
        assert!(!generate_legal_moves(&mut b).is_empty());
    }

    #[test]
    fn complete_mate_in_one_rejects_an_initial_check() {
        let board = Board::from_sfen(ALREADY_CHECKED_SFEN).unwrap();
        let analysis = analyze_mate_in_one(&board);
        assert!(!analysis.valid_position());
        assert!(analysis.defender_already_in_check());
        assert_eq!(
            analysis.invalid_reason,
            Some(MateInOneInvalidReason::DefenderAlreadyInCheck)
        );
        assert!(analysis.solutions.is_empty());
    }

    #[test]
    fn complete_mate_in_one_finds_distant_promotion_and_non_promotion() {
        let solutions = solution_strings(DISTANT_ROOK_MATE_SFEN);
        assert!(solutions.contains(&"5e5c".to_owned()));
        assert!(solutions.contains(&"5e5c+".to_owned()));
    }

    #[test]
    fn complete_mate_in_one_finds_discovered_checks() {
        // Moving the silver away from the fifth file opens the rook's line to
        // the white king. The silver itself does not give check from 4d/6d.
        let solutions = solution_strings("4k4/2S3S2/2SGSGS2/9/4R4/9/9/9/4K4 b - 1");
        assert!(
            solutions.contains(&"5c4d".to_owned()) || solutions.contains(&"5c6d".to_owned()),
            "expected a discovered mate, got {solutions:?}"
        );
    }

    #[test]
    fn complete_mate_in_one_finds_a_drop() {
        let board = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b G 1").unwrap();
        let analysis = analyze_mate_in_one(&board);
        assert!(analysis.valid_position());
        assert_eq!(
            analysis.solutions,
            vec![Move::drop(
                crate::square::Square::from_shogi(5, 2),
                PieceKind::Kin
            )]
        );
        assert!(analysis.unique_solution());
    }

    #[test]
    fn complete_mate_in_one_keeps_pawn_drop_mate_illegal() {
        // P*1b would geometrically mate, but uchifuzume makes it illegal.
        let board = Board::from_sfen("8k/9/7GK/9/9/9/9/9/9 b P 1").unwrap();
        let analysis = analyze_mate_in_one(&board);
        assert!(analysis.valid_position());
        assert!(
            analysis
                .solutions
                .iter()
                .all(|mv| !(mv.is_drop() && mv.piece_kind == PieceKind::Fu))
        );
    }

    #[test]
    fn optional_attacker_king_policy_handles_both_sides_and_uchifuzume() {
        let black = Board::from_sfen("4k4/9/4P4/9/9/9/9/9/9 b G 1").unwrap();
        let strict = analyze_mate_in_one(&black);
        assert_eq!(
            strict.invalid_reason,
            Some(MateInOneInvalidReason::MissingAttackerKing)
        );
        let optional =
            analyze_mate_in_one_with_policy(&black, MatePositionPolicy::AllowMissingAttackerKing);
        assert!(optional.valid_position());
        assert_eq!(
            optional
                .solutions
                .iter()
                .copied()
                .map(move_to_usi)
                .collect::<Vec<_>>(),
            vec!["G*5b"]
        );

        let white = Board::from_sfen("9/9/9/9/9/9/4p4/9/4K4 w g 1").unwrap();
        let optional =
            analyze_mate_in_one_with_policy(&white, MatePositionPolicy::AllowMissingAttackerKing);
        assert!(optional.valid_position());
        assert_eq!(
            optional
                .solutions
                .iter()
                .copied()
                .map(move_to_usi)
                .collect::<Vec<_>>(),
            vec!["G*5h"]
        );

        // Removing the attacking king must not make pawn-drop mate legal.
        let pawn_drop = Board::from_sfen("8k/9/7G1/9/9/9/9/9/9 b P 1").unwrap();
        let optional = analyze_mate_in_one_with_policy(
            &pawn_drop,
            MatePositionPolicy::AllowMissingAttackerKing,
        );
        assert!(optional.valid_position());
        assert!(
            optional
                .solutions
                .iter()
                .all(|mv| !(mv.is_drop() && mv.piece_kind == PieceKind::Fu))
        );
    }

    #[test]
    fn optional_attacker_king_policy_keeps_other_validation_and_two_king_legality() {
        let missing_defender = Board::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").unwrap();
        assert_eq!(
            analyze_mate_in_one_with_policy(
                &missing_defender,
                MatePositionPolicy::AllowMissingAttackerKing,
            )
            .invalid_reason,
            Some(MateInOneInvalidReason::MissingDefenderKing)
        );

        let duplicate_attacker = Board::from_sfen("4k4/9/9/9/9/9/9/4K4/4K4 b - 1").unwrap();
        assert_eq!(
            analyze_mate_in_one_with_policy(
                &duplicate_attacker,
                MatePositionPolicy::AllowMissingAttackerKing,
            )
            .invalid_reason,
            Some(MateInOneInvalidReason::MultipleAttackerKings)
        );

        let checked = Board::from_sfen(ALREADY_CHECKED_SFEN).unwrap();
        assert_eq!(
            analyze_mate_in_one_with_policy(
                &checked,
                MatePositionPolicy::AllowMissingAttackerKing,
            )
            .invalid_reason,
            Some(MateInOneInvalidReason::DefenderAlreadyInCheck)
        );

        let two_king = Board::from_sfen(DISTANT_ROOK_MATE_SFEN).unwrap();
        assert_eq!(
            analyze_mate_in_one(&two_king),
            analyze_mate_in_one_with_policy(
                &two_king,
                MatePositionPolicy::AllowMissingAttackerKing,
            )
        );
    }

    #[test]
    fn complete_mate_in_one_distinguishes_zero_one_and_multiple_solutions() {
        let none = analyze_mate_in_one(&Board::startpos());
        assert!(none.valid_position());
        assert!(none.solutions.is_empty());
        assert!(!none.unique_solution());

        let one = analyze_mate_in_one(&Board::from_sfen("4k4/9/4P4/9/9/9/9/9/4K4 b G 1").unwrap());
        assert_eq!(one.solutions.len(), 1);
        assert!(one.unique_solution());

        let multiple = analyze_mate_in_one(&Board::from_sfen(DISTANT_ROOK_MATE_SFEN).unwrap());
        assert!(multiple.solutions.len() > 1);
        assert!(!multiple.unique_solution());
    }

    #[test]
    fn complete_mate_in_one_reports_missing_and_duplicate_kings() {
        let missing = analyze_mate_in_one(&Board::from_sfen("4k4/9/9/9/9/9/9/9/9 b - 1").unwrap());
        assert_eq!(
            missing.invalid_reason,
            Some(MateInOneInvalidReason::MissingAttackerKing)
        );

        let duplicate =
            analyze_mate_in_one(&Board::from_sfen("3kk4/9/9/9/9/9/9/9/4K4 b - 1").unwrap());
        assert_eq!(
            duplicate.invalid_reason,
            Some(MateInOneInvalidReason::MultipleDefenderKings)
        );
    }

    #[test]
    fn complete_bounded_analysis_finds_all_shortest_mate_in_one_moves() {
        let board = Board::from_sfen(DISTANT_ROOK_MATE_SFEN).unwrap();
        let analysis = analyze_mate(&board, 5, 100_000);
        let solutions = analysis
            .solutions
            .iter()
            .copied()
            .map(move_to_usi)
            .collect::<Vec<_>>();
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(analysis.shortest_mate_ply, Some(1));
        assert!(solutions.contains(&"5e5c".to_owned()));
        assert!(solutions.contains(&"5e5c+".to_owned()));
        assert!(!analysis.unique_solution());
        assert!(!analysis.aborted);
    }

    #[test]
    fn complete_bounded_analysis_reports_three_instead_of_requested_five() {
        let board = Board::from_sfen(SHORTEST_THREE_SFEN).unwrap();
        let analysis = analyze_mate(&board, 5, 1_000_000);
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(analysis.shortest_mate_ply, Some(3));
        assert!(!analysis.solutions.is_empty());
        assert!(!analysis.aborted);
    }

    #[test]
    fn complete_bounded_analysis_distinguishes_completed_no_mate() {
        let analysis = analyze_mate(&Board::startpos(), 5, 100_000);
        assert_eq!(analysis.outcome, MateAnalysisOutcome::NoMate);
        assert_eq!(analysis.shortest_mate_ply, None);
        assert!(analysis.solutions.is_empty());
        assert!(!analysis.unique_solution());
        assert!(!analysis.aborted);
        assert_eq!(analysis.abort_reason, None);
    }

    #[test]
    fn complete_bounded_analysis_finds_unique_five_and_reuses_intermediate_position() {
        let board = Board::from_sfen(UNIQUE_FIVE_SFEN).unwrap();
        let analysis = analyze_mate(&board, 5, 1_000_000);
        let solutions = analysis
            .solutions
            .iter()
            .copied()
            .map(move_to_usi)
            .collect::<Vec<_>>();
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(
            analysis.shortest_mate_ply,
            Some(5),
            "unexpected solutions: {solutions:?}"
        );
        assert_eq!(solutions, vec!["7h8h"]);
        assert!(analysis.unique_solution());

        let mut intermediate = board.clone();
        for usi in ["7h8h", "9i8h"] {
            let mv = move_from_usi(usi, &intermediate).unwrap();
            intermediate.do_move(mv);
        }
        let continuation = analyze_mate(&intermediate, 3, 1_000_000);
        assert_eq!(continuation.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(continuation.shortest_mate_ply, Some(3));
        assert!(continuation.unique_solution());
        assert_eq!(
            continuation
                .solutions
                .iter()
                .copied()
                .map(move_to_usi)
                .collect::<Vec<_>>(),
            vec!["6h7h"]
        );
    }

    #[test]
    fn complete_bounded_analysis_corrects_the_mislabelled_issue_fixture() {
        // Issue #85 originally described this as a unique mate in five. A
        // complete search proves two mate-in-one moves instead, including the
        // promoted variant of the proposed first move.
        let board = Board::from_sfen(MISLABELLED_FIVE_SFEN).unwrap();
        let analysis = analyze_mate(&board, 5, 1_000_000);
        let solutions = analysis
            .solutions
            .iter()
            .copied()
            .map(move_to_usi)
            .collect::<Vec<_>>();
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(analysis.shortest_mate_ply, Some(1));
        assert_eq!(solutions, vec!["7a6b+", "7b6b"]);
        assert!(!analysis.unique_solution());
    }

    #[test]
    fn complete_bounded_analysis_never_promotes_a_partial_result() {
        let board = Board::from_sfen(UNIQUE_FIVE_SFEN).unwrap();
        let analysis = analyze_mate(&board, 5, 1);
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Unknown);
        assert_eq!(analysis.shortest_mate_ply, None);
        assert!(analysis.solutions.is_empty());
        assert!(!analysis.unique_solution());
        assert!(analysis.aborted);
        assert_eq!(
            analysis.abort_reason,
            Some(MateAnalysisAbortReason::NodeLimit)
        );
        assert_eq!(analysis.nodes, 1);
    }

    #[test]
    fn optional_attacker_king_policy_solves_the_original_seven_ply_diagram() {
        let board = Board::from_sfen(SINGLE_KING_SEVEN_SFEN).unwrap();
        let strict = analyze_mate(&board, 13, 2_000_000);
        assert_eq!(
            strict.invalid_reason,
            Some(MateInOneInvalidReason::MissingAttackerKing)
        );

        let analysis = analyze_mate_with_policy(
            &board,
            13,
            2_000_000,
            MatePositionPolicy::AllowMissingAttackerKing,
        );
        assert!(analysis.valid_position());
        assert_eq!(analysis.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(analysis.shortest_proof_ply(), Some(7));
        assert_eq!(
            analysis.distance_rule(),
            MateDistanceRule::AllLegalDefenderReplies
        );
        assert!(!analysis.solutions.is_empty());
        assert!(!analysis.aborted);
    }

    #[test]
    fn proof_distance_counts_futile_interpositions_and_keeps_bounds_distinct() {
        let eleven = Board::from_sfen(FUTILE_INTERPOSITION_ELEVEN_SFEN).unwrap();
        let bounded = analyze_mate(&eleven, 7, 2_000_000);
        assert_eq!(bounded.outcome, MateAnalysisOutcome::NoMate);
        assert_eq!(bounded.shortest_proof_ply(), None);
        assert!(!bounded.aborted);

        let proof = analyze_mate(&eleven, 11, 2_000_000);
        assert_eq!(proof.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(proof.shortest_proof_ply(), Some(11));
        assert_eq!(proof.shortest_mate_ply, proof.shortest_proof_ply());
        assert_eq!(
            proof.distance_rule(),
            MateDistanceRule::AllLegalDefenderReplies
        );

        let nine = Board::from_sfen(FUTILE_INTERPOSITION_NINE_SFEN).unwrap();
        let proof = analyze_mate(&nine, 9, 2_000_000);
        assert_eq!(proof.outcome, MateAnalysisOutcome::Mate);
        assert_eq!(proof.shortest_proof_ply(), Some(9));

        let cutoff = analyze_mate(&nine, 9, 1);
        assert_eq!(cutoff.outcome, MateAnalysisOutcome::Unknown);
        assert_eq!(cutoff.shortest_proof_ply(), None);
        assert!(cutoff.aborted);
    }
}
