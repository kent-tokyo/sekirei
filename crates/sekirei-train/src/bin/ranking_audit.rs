//! Fail-closed audit for diagnostic NNUE root-ranking pairs.
//!
//! This binary deliberately does *not* train or write weights.  It proves that
//! a pairwise-ranking corpus describes two distinct legal moves from the same
//! reconstructed parent position before a future trainer consumes it.

use std::{collections::BTreeMap, env, fs, process};

use sekirei_core::{
    board::Board,
    eval::{NnueOutputMode, evaluate_with_weights_mode},
    nnue::{NnueActivationSummary, read_weights},
    sfen::{board_to_sfen, move_from_usi},
};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "sekirei.root-rank-pairs.v1";

fn default_pair_selection() -> String {
    "all".to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema: String,
    diagnostic_only: bool,
    strength_claim: String,
    source_contract: SourceContract,
    source_teacher: SourceTeacher,
    #[serde(default = "default_pair_selection")]
    pair_selection: String,
    pairs: Vec<RankPair>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceContract {
    depth: u32,
    threads: u32,
    spec_top_n: u32,
    root_candidate_mode: String,
    root_candidate_limit: u32,
    complete_legal_root_set: bool,
    #[serde(default)]
    candidate_source_sha256: Option<String>,
    per_category_unique_positions: u32,
    normal_score_abs_max_cp: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceTeacher {
    binary: String,
    binary_sha256: String,
    weights: String,
    weights_sha256: String,
    nnue_output: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RankPair {
    parent_id: String,
    category: String,
    initial_sfen: String,
    history_before_usi: Vec<String>,
    parent_sfen: String,
    /// Provenance is checked by the split-stage audit; retain it here so this
    /// strict decoder accepts the same immutable artifact as the trainer.
    #[serde(default)]
    source: Option<serde_json::Value>,
    higher_move_usi: String,
    lower_move_usi: String,
    teacher_score_gap_cp: i32,
}

#[derive(Debug, Serialize)]
struct AuditReport {
    schema: &'static str,
    input_schema: String,
    diagnostic_only: bool,
    strength_claim: &'static str,
    training_performed: bool,
    pairs_verified: usize,
    parent_positions: usize,
    categories: BTreeMap<String, usize>,
    teacher_gap_cp_min: i32,
    teacher_gap_cp_max: i32,
    source: AuditSource,
    model_diagnostic: Option<ModelDiagnostic>,
}

#[derive(Debug, Serialize)]
struct AuditSource {
    depth: u32,
    threads: u32,
    spec_top_n: u32,
    root_candidate_mode: String,
    complete_legal_root_set: bool,
    candidate_source_sha256: Option<String>,
    pair_selection: String,
    teacher_binary_sha256: String,
    teacher_weights_sha256: String,
}

/// Static NNUE score diagnostic for the two child positions in each pair.
/// This is intentionally not a root search and not a strength measurement.
#[derive(Debug, Serialize)]
struct ModelDiagnostic {
    checkpoint: String,
    output_mode: String,
    pairs_scored: usize,
    teacher_preferred_ordered_pairs: usize,
    teacher_preferred_ordering_rate: f64,
    mean_pairwise_logistic_loss: f64,
    mean_parent_oriented_margin_cp: f64,
    mean_parent_rank_loss_cp: f64,
    major_blunders_ge_300cp: usize,
    unique_moves_scored: usize,
    mean_activation: ActivationDiagnostic,
    parent_diagnostics: Vec<ParentRankingDiagnostic>,
    move_diagnostics: Vec<MoveDiagnostic>,
    pair_diagnostics: Vec<PairDiagnostic>,
}

#[derive(Debug, Serialize)]
struct ParentRankingDiagnostic {
    parent_id: String,
    candidate_moves: usize,
    model_chosen_move_usi: String,
    model_top_margin_cp: i32,
    teacher_rank_loss_cp: i32,
    major_blunder_ge_300cp: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
struct ActivationDiagnostic {
    ft_active_ratio: f64,
    ft_saturated_ratio: f64,
    ft_mean: f64,
    l2_active_ratio: f64,
    l2_saturated_ratio: f64,
    l2_mean: f64,
}

impl ActivationDiagnostic {
    fn from_summary(summary: NnueActivationSummary) -> Self {
        Self {
            ft_active_ratio: summary.ft_active as f64 / summary.ft_units as f64,
            ft_saturated_ratio: summary.ft_saturated as f64 / summary.ft_units as f64,
            ft_mean: summary.ft_mean,
            l2_active_ratio: summary.l2_active as f64 / summary.l2_units as f64,
            l2_saturated_ratio: summary.l2_saturated as f64 / summary.l2_units as f64,
            l2_mean: summary.l2_mean,
        }
    }

    fn add_assign(&mut self, other: Self) {
        self.ft_active_ratio += other.ft_active_ratio;
        self.ft_saturated_ratio += other.ft_saturated_ratio;
        self.ft_mean += other.ft_mean;
        self.l2_active_ratio += other.l2_active_ratio;
        self.l2_saturated_ratio += other.l2_saturated_ratio;
        self.l2_mean += other.l2_mean;
    }

    fn divided_by(mut self, count: usize) -> Self {
        let denominator = count as f64;
        self.ft_active_ratio /= denominator;
        self.ft_saturated_ratio /= denominator;
        self.ft_mean /= denominator;
        self.l2_active_ratio /= denominator;
        self.l2_saturated_ratio /= denominator;
        self.l2_mean /= denominator;
        self
    }
}

#[derive(Debug, Serialize)]
struct MoveDiagnostic {
    parent_id: String,
    move_usi: String,
    parent_score_cp: i32,
    teacher_rank_loss_cp: i32,
    teacher_top: bool,
    activation: ActivationDiagnostic,
}

/// Per-pair static result retained solely for stratified error analysis.
/// The teacher label remains an observation from the fixed-depth root search;
/// this report never promotes it to a game-result or training label.
#[derive(Debug, Serialize)]
struct PairDiagnostic {
    parent_id: String,
    category: String,
    teacher_score_gap_cp: i32,
    higher_move_usi: String,
    lower_move_usi: String,
    higher_parent_score_cp: i32,
    lower_parent_score_cp: i32,
    parent_oriented_margin_cp: i32,
    teacher_order_preserved: bool,
}

struct Cli {
    pairs: String,
    weights: Option<String>,
}

fn usage() {
    eprintln!(
        "Usage: ranking_audit --pairs <teacher_root_prefix_depth3_pairs.json> [--weights <checkpoint.bin>]"
    );
}

fn parse_cli() -> Result<Cli, String> {
    let mut args = env::args().skip(1);
    let mut pairs = None;
    let mut weights = None;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option.as_str() {
            "--pairs" => {
                if pairs.replace(value).is_some() {
                    return Err("--pairs specified more than once".to_owned());
                }
            }
            "--weights" => {
                if weights.replace(value).is_some() {
                    return Err("--weights specified more than once".to_owned());
                }
            }
            _ => return Err(format!("unknown option {option:?}")),
        }
    }
    Ok(Cli {
        pairs: pairs.ok_or_else(|| "--pairs <path> is required".to_owned())?,
        weights,
    })
}

fn validate_source(corpus: &Corpus) -> Result<(), String> {
    if corpus.schema != SCHEMA {
        return Err(format!(
            "unsupported ranking pair schema {:?}",
            corpus.schema
        ));
    }
    if !corpus.diagnostic_only || corpus.strength_claim != "not_permitted" {
        return Err(
            "ranking pair corpus must remain diagnostic-only with strength_claim=not_permitted"
                .to_owned(),
        );
    }
    if !matches!(
        corpus.pair_selection.as_str(),
        "all" | "adjacent" | "top-vs-rest"
    ) {
        return Err("unsupported root-ranking pair selection".to_owned());
    }
    let source = &corpus.source_contract;
    let source_scope_valid = match source.root_candidate_mode.as_str() {
        "complete_legal_set" => {
            source.complete_legal_root_set && source.candidate_source_sha256.is_none()
        }
        "preregistered_candidate_union" => {
            !source.complete_legal_root_set
                && source
                    .candidate_source_sha256
                    .as_ref()
                    .is_some_and(|digest| {
                        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
        }
        _ => false,
    };
    if source.depth == 0
        || source.threads != 1
        || source.spec_top_n != 0
        || source.root_candidate_limit == 0
        || source.per_category_unique_positions == 0
        || source.normal_score_abs_max_cp <= 0
        || !source_scope_valid
    {
        return Err("unsupported or unsafe root-ranking source contract".to_owned());
    }
    let teacher = &corpus.source_teacher;
    if teacher.binary.is_empty()
        || teacher.weights.is_empty()
        || teacher.binary_sha256.len() != 64
        || teacher.weights_sha256.len() != 64
        || teacher.nnue_output.is_empty()
    {
        return Err("root-ranking source teacher identity is incomplete".to_owned());
    }
    if corpus.pairs.is_empty() {
        return Err("ranking pair corpus contains no strict pairs".to_owned());
    }
    Ok(())
}

fn reconstruct_pair(
    pair: &RankPair,
    index: usize,
    normal_score_abs_max_cp: i32,
) -> Result<(Board, sekirei_core::mv::Move, sekirei_core::mv::Move), String> {
    let label = format!("pair[{index}] parent_id={:?}", pair.parent_id);
    let _source = &pair.source;
    if pair.category.is_empty()
        || pair.parent_id.is_empty()
        || pair.teacher_score_gap_cp <= 0
        || pair.teacher_score_gap_cp > normal_score_abs_max_cp.saturating_mul(2)
    {
        return Err(format!(
            "{label}: missing category/parent id or non-strict score gap"
        ));
    }
    let mut board = Board::from_sfen(&pair.initial_sfen)
        .map_err(|error| format!("{label}: invalid initial SFEN: {error}"))?;
    for move_usi in &pair.history_before_usi {
        let mv = move_from_usi(move_usi, &board)
            .map_err(|error| format!("{label}: invalid history move {move_usi:?}: {error}"))?;
        board.do_move(mv);
    }
    if board_to_sfen(&board) != pair.parent_sfen {
        return Err(format!(
            "{label}: replayed history does not reconstruct parent_sfen"
        ));
    }
    let high = move_from_usi(&pair.higher_move_usi, &board)
        .map_err(|error| format!("{label}: invalid higher move: {error}"))?;
    let low = move_from_usi(&pair.lower_move_usi, &board)
        .map_err(|error| format!("{label}: invalid lower move: {error}"))?;
    if high == low {
        return Err(format!("{label}: pair must contain distinct moves"));
    }
    Ok((board, high, low))
}

fn output_mode(value: &str) -> Result<NnueOutputMode, String> {
    match value {
        "absolute" => Ok(NnueOutputMode::Absolute),
        "residual-material" => Ok(NnueOutputMode::ResidualMaterial),
        _ => Err(format!("unsupported source teacher nnue_output {value:?}")),
    }
}

fn pairwise_loss(margin: f64) -> f64 {
    if margin >= 0.0 {
        (-margin).exp().ln_1p()
    } else {
        -margin + margin.exp().ln_1p()
    }
}

fn diagnose_model(corpus: &Corpus, checkpoint: &str) -> Result<ModelDiagnostic, String> {
    let weights = read_weights(std::path::Path::new(checkpoint))
        .map_err(|error| format!("cannot read NNUE checkpoint {checkpoint:?}: {error}"))?;
    let mode = output_mode(&corpus.source_teacher.nnue_output)?;
    let mut ordered = 0usize;
    let mut total_loss = 0.0;
    let mut total_margin = 0.0;
    let mut pair_diagnostics = Vec::with_capacity(corpus.pairs.len());
    let mut parent_move_scores: BTreeMap<String, BTreeMap<String, i32>> = BTreeMap::new();
    let mut parent_move_losses: BTreeMap<String, BTreeMap<String, i32>> = BTreeMap::new();
    let mut parent_move_activations: BTreeMap<String, BTreeMap<String, ActivationDiagnostic>> =
        BTreeMap::new();
    for (index, pair) in corpus.pairs.iter().enumerate() {
        let (mut board, high, low) =
            reconstruct_pair(pair, index, corpus.source_contract.normal_score_abs_max_cp)?;
        let high_undo = board.do_move(high);
        let high_parent_score = -evaluate_with_weights_mode(&board, &weights, mode);
        let high_activation =
            ActivationDiagnostic::from_summary(board.nnue_activation_summary_with(&weights));
        board.undo_move(high_undo);
        let low_undo = board.do_move(low);
        let low_parent_score = -evaluate_with_weights_mode(&board, &weights, mode);
        let low_activation =
            ActivationDiagnostic::from_summary(board.nnue_activation_summary_with(&weights));
        board.undo_move(low_undo);
        let margin = high_parent_score - low_parent_score;
        ordered += usize::from(margin > 0);
        total_loss += pairwise_loss(f64::from(margin));
        total_margin += f64::from(margin);
        let scores = parent_move_scores
            .entry(pair.parent_id.clone())
            .or_default();
        for (move_usi, score, activation) in [
            (&pair.higher_move_usi, high_parent_score, high_activation),
            (&pair.lower_move_usi, low_parent_score, low_activation),
        ] {
            if let Some(previous) = scores.insert(move_usi.clone(), score)
                && previous != score
            {
                return Err(format!(
                    "parent {:?} move {:?} has inconsistent static scores",
                    pair.parent_id, move_usi
                ));
            }
            let activations = parent_move_activations
                .entry(pair.parent_id.clone())
                .or_default();
            if let Some(previous) = activations.insert(move_usi.clone(), activation)
                && previous != activation
            {
                return Err(format!(
                    "parent {:?} move {:?} has inconsistent activation diagnostics",
                    pair.parent_id, move_usi
                ));
            }
        }
        let losses = parent_move_losses
            .entry(pair.parent_id.clone())
            .or_default();
        losses.entry(pair.higher_move_usi.clone()).or_insert(0);
        losses
            .entry(pair.lower_move_usi.clone())
            .and_modify(|value| *value = (*value).max(pair.teacher_score_gap_cp))
            .or_insert(pair.teacher_score_gap_cp);
        pair_diagnostics.push(PairDiagnostic {
            parent_id: pair.parent_id.clone(),
            category: pair.category.clone(),
            teacher_score_gap_cp: pair.teacher_score_gap_cp,
            higher_move_usi: pair.higher_move_usi.clone(),
            lower_move_usi: pair.lower_move_usi.clone(),
            higher_parent_score_cp: high_parent_score,
            lower_parent_score_cp: low_parent_score,
            parent_oriented_margin_cp: margin,
            teacher_order_preserved: margin > 0,
        });
    }
    let count = corpus.pairs.len() as f64;
    let mut parent_diagnostics = Vec::with_capacity(parent_move_scores.len());
    let mut move_diagnostics = Vec::new();
    let mut activation_total = ActivationDiagnostic::default();
    for (parent_id, scores) in parent_move_scores {
        let mut ranked: Vec<_> = scores.iter().collect();
        ranked.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
        let (chosen_move, chosen_score) = ranked
            .first()
            .copied()
            .ok_or_else(|| format!("parent {parent_id:?} has no model scores"))?;
        let model_top_margin_cp = ranked
            .get(1)
            .map_or(0, |(_, runner_up)| *chosen_score - **runner_up);
        let losses = parent_move_losses
            .get(&parent_id)
            .ok_or_else(|| format!("parent {parent_id:?} has no teacher losses"))?;
        let rank_loss = losses.get(chosen_move).copied().unwrap_or(0);
        let activations = parent_move_activations
            .get(&parent_id)
            .ok_or_else(|| format!("parent {parent_id:?} has no activation diagnostics"))?;
        for (move_usi, score) in &scores {
            let activation = *activations.get(move_usi).ok_or_else(|| {
                format!("parent {parent_id:?} move {move_usi:?} lacks activation diagnostics")
            })?;
            activation_total.add_assign(activation);
            let teacher_rank_loss_cp = losses.get(move_usi).copied().unwrap_or(0);
            move_diagnostics.push(MoveDiagnostic {
                parent_id: parent_id.clone(),
                move_usi: move_usi.clone(),
                parent_score_cp: *score,
                teacher_rank_loss_cp,
                teacher_top: teacher_rank_loss_cp == 0,
                activation,
            });
        }
        parent_diagnostics.push(ParentRankingDiagnostic {
            parent_id,
            candidate_moves: scores.len(),
            model_chosen_move_usi: chosen_move.clone(),
            model_top_margin_cp,
            teacher_rank_loss_cp: rank_loss,
            major_blunder_ge_300cp: rank_loss >= 300,
        });
    }
    let parent_count = parent_diagnostics.len() as f64;
    let mean_parent_rank_loss_cp = parent_diagnostics
        .iter()
        .map(|row| f64::from(row.teacher_rank_loss_cp))
        .sum::<f64>()
        / parent_count;
    let major_blunders_ge_300cp = parent_diagnostics
        .iter()
        .filter(|row| row.major_blunder_ge_300cp)
        .count();
    let unique_moves_scored = move_diagnostics.len();
    let mean_activation = activation_total.divided_by(unique_moves_scored);
    Ok(ModelDiagnostic {
        checkpoint: checkpoint.to_owned(),
        output_mode: corpus.source_teacher.nnue_output.clone(),
        pairs_scored: corpus.pairs.len(),
        teacher_preferred_ordered_pairs: ordered,
        teacher_preferred_ordering_rate: ordered as f64 / count,
        mean_pairwise_logistic_loss: total_loss / count,
        mean_parent_oriented_margin_cp: total_margin / count,
        mean_parent_rank_loss_cp,
        major_blunders_ge_300cp,
        unique_moves_scored,
        mean_activation,
        parent_diagnostics,
        move_diagnostics,
        pair_diagnostics,
    })
}

fn audit(corpus: &Corpus, weights: Option<&str>) -> Result<AuditReport, String> {
    validate_source(corpus)?;
    let mut categories = BTreeMap::new();
    let mut parents = std::collections::BTreeSet::new();
    let mut min_gap = i32::MAX;
    let mut max_gap = i32::MIN;

    for (index, pair) in corpus.pairs.iter().enumerate() {
        let (mut board, high, low) =
            reconstruct_pair(pair, index, corpus.source_contract.normal_score_abs_max_cp)?;
        let hash_before = board.hash();
        let sfen_before = board_to_sfen(&board);
        for mv in [high, low] {
            let undo = board.do_move(mv);
            board.undo_move(undo);
            if board.hash() != hash_before || board_to_sfen(&board) != sfen_before {
                return Err(format!(
                    "pair[{index}]: do/undo failed to restore parent state"
                ));
            }
        }
        *categories.entry(pair.category.clone()).or_insert(0) += 1;
        parents.insert((pair.parent_id.as_str(), pair.parent_sfen.as_str()));
        min_gap = min_gap.min(pair.teacher_score_gap_cp);
        max_gap = max_gap.max(pair.teacher_score_gap_cp);
    }
    Ok(AuditReport {
        schema: "sekirei.root-rank-pair-audit.v1",
        input_schema: corpus.schema.clone(),
        diagnostic_only: true,
        strength_claim: "not_permitted",
        training_performed: false,
        pairs_verified: corpus.pairs.len(),
        parent_positions: parents.len(),
        categories,
        teacher_gap_cp_min: min_gap,
        teacher_gap_cp_max: max_gap,
        source: AuditSource {
            depth: corpus.source_contract.depth,
            threads: corpus.source_contract.threads,
            spec_top_n: corpus.source_contract.spec_top_n,
            root_candidate_mode: corpus.source_contract.root_candidate_mode.clone(),
            complete_legal_root_set: corpus.source_contract.complete_legal_root_set,
            candidate_source_sha256: corpus.source_contract.candidate_source_sha256.clone(),
            pair_selection: corpus.pair_selection.clone(),
            teacher_binary_sha256: corpus.source_teacher.binary_sha256.clone(),
            teacher_weights_sha256: corpus.source_teacher.weights_sha256.clone(),
        },
        model_diagnostic: weights
            .map(|path| diagnose_model(corpus, path))
            .transpose()?,
    })
}

fn main() {
    let cli = parse_cli().unwrap_or_else(|error| {
        eprintln!("error: {error}");
        usage();
        process::exit(2);
    });
    let text = fs::read_to_string(&cli.pairs).unwrap_or_else(|error| {
        eprintln!("error: cannot read {:?}: {error}", cli.pairs);
        process::exit(1);
    });
    let corpus: Corpus = serde_json::from_str(&text).unwrap_or_else(|error| {
        eprintln!("error: invalid ranking pair JSON: {error}");
        process::exit(1);
    });
    let report = audit(&corpus, cli.weights.as_deref()).unwrap_or_else(|error| {
        eprintln!("error: ranking pair audit failed: {error}");
        process::exit(1);
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("report serializes")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(pair: RankPair) -> Corpus {
        Corpus {
            schema: SCHEMA.to_owned(),
            diagnostic_only: true,
            strength_claim: "not_permitted".to_owned(),
            source_contract: SourceContract {
                depth: 3,
                threads: 1,
                spec_top_n: 0,
                root_candidate_mode: "complete_legal_set".to_owned(),
                root_candidate_limit: 32,
                complete_legal_root_set: true,
                candidate_source_sha256: None,
                per_category_unique_positions: 1,
                normal_score_abs_max_cp: 10_000,
            },
            source_teacher: SourceTeacher {
                binary: "teacher".to_owned(),
                binary_sha256: "a".repeat(64),
                weights: "weights".to_owned(),
                weights_sha256: "b".repeat(64),
                nnue_output: "absolute".to_owned(),
            },
            pair_selection: "all".to_owned(),
            pairs: vec![pair],
        }
    }

    fn start_pair() -> RankPair {
        let board = Board::startpos();
        let sfen = board_to_sfen(&board);
        RankPair {
            parent_id: "start".to_owned(),
            category: "opening_control".to_owned(),
            initial_sfen: sfen.clone(),
            history_before_usi: vec![],
            parent_sfen: sfen,
            source: None,
            higher_move_usi: "7g7f".to_owned(),
            lower_move_usi: "2g2f".to_owned(),
            teacher_score_gap_cp: 1,
        }
    }

    #[test]
    fn accepts_strict_legal_pair_without_training() {
        let report = audit(&corpus(start_pair()), None).unwrap();
        assert_eq!(report.pairs_verified, 1);
        assert_eq!(report.parent_positions, 1);
        assert!(!report.training_performed);
    }

    #[test]
    fn accepts_top_vs_rest_pair_selection() {
        let mut input = corpus(start_pair());
        input.pair_selection = "top-vs-rest".to_owned();
        let report = audit(&input, None).unwrap();
        assert_eq!(report.source.pair_selection, "top-vs-rest");
    }

    #[test]
    fn accepts_only_bound_preregistered_candidate_union() {
        let mut input = corpus(start_pair());
        input.source_contract.depth = 7;
        input.source_contract.root_candidate_mode = "preregistered_candidate_union".to_owned();
        input.source_contract.complete_legal_root_set = false;
        input.source_contract.candidate_source_sha256 = Some("c".repeat(64));
        let report = audit(&input, None).unwrap();
        assert_eq!(
            report.source.candidate_source_sha256.as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );

        input.source_contract.candidate_source_sha256 = None;
        assert!(audit(&input, None).is_err());
    }

    #[test]
    fn rejects_non_strict_or_state_mismatched_pair() {
        let mut non_strict = start_pair();
        non_strict.teacher_score_gap_cp = 0;
        assert!(audit(&corpus(non_strict), None).is_err());
        let mut mismatched = start_pair();
        mismatched.parent_sfen = "invalid parent state".to_owned();
        assert!(audit(&corpus(mismatched), None).is_err());
    }

    #[test]
    fn rejects_unknown_pair_selection() {
        let mut input = corpus(start_pair());
        input.pair_selection = "unsupported".to_owned();
        assert!(audit(&input, None).is_err());
    }

    #[test]
    fn pairwise_loss_is_finite_and_improves_with_margin() {
        assert!(pairwise_loss(1_000.0).is_finite());
        assert!(pairwise_loss(-1_000.0).is_finite());
        assert!(pairwise_loss(10.0) < pairwise_loss(-10.0));
    }
}
