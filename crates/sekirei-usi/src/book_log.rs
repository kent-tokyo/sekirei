//! Opt-in, append-only opening-book decision audit log.
//!
//! The logger records deterministic identifiers and policy inputs only. It
//! never randomizes the policy, so no propensity field is emitted; paired
//! A/B analysis is the supported evaluation contract for these records.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::book::BookCandidate;

pub const POLICY_VERSION: &str = "sekirei-opening-book-policy-v1";

#[derive(Debug, Serialize)]
pub struct DecisionRecord<'a> {
    pub schema: &'static str,
    pub event: &'static str,
    pub experiment_id: &'a str,
    pub game_id: String,
    pub decision_id: String,
    pub ply: usize,
    pub state_sha256: String,
    pub use_book: bool,
    pub book_max_ply: usize,
    pub book_min_confidence: f64,
    pub book_sha256: Option<&'a str>,
    pub book_schema_version: Option<u32>,
    pub book_producer_version: Option<&'a str>,
    pub build_config_fingerprint: Option<u64>,
    pub candidates: &'a [BookCandidate],
    pub selected_action: Option<&'a str>,
    pub action_source: &'static str,
    pub fallback_reason: Option<&'a str>,
    pub policy_version: &'static str,
    pub evaluation_contract: &'static str,
}

#[derive(Debug, Serialize)]
struct TerminalRecord<'a> {
    schema: &'static str,
    event: &'static str,
    experiment_id: &'a str,
    game_id: String,
    result: &'a str,
}

pub struct DecisionContext<'a> {
    pub experiment_id: &'a str,
    pub game_counter: u64,
    pub decision_counter: u64,
    pub ply: usize,
    pub sfen: &'a str,
    pub use_book: bool,
    pub book_max_ply: usize,
    pub book_min_confidence: f64,
    pub book_sha256: Option<&'a str>,
    pub book_schema_version: Option<u32>,
    pub book_producer_version: Option<&'a str>,
    pub build_config_fingerprint: Option<u64>,
    pub candidates: &'a [BookCandidate],
    pub selected_action: Option<&'a str>,
    pub action_source: &'static str,
    pub fallback_reason: Option<&'a str>,
}

#[derive(Default)]
pub struct BookDecisionLogger {
    path: Option<PathBuf>,
    experiment_id: String,
}

impl BookDecisionLogger {
    pub fn configure(&mut self, path: &str, experiment_id: &str) {
        self.path = (!path.trim().is_empty()).then(|| PathBuf::from(path.trim()));
        self.experiment_id = experiment_id.trim().to_string();
    }

    pub fn enabled(&self) -> bool {
        self.path.is_some()
    }

    pub fn experiment_id(&self) -> &str {
        if self.experiment_id.is_empty() {
            "default"
        } else {
            &self.experiment_id
        }
    }

    pub fn set_experiment_id(&mut self, experiment_id: &str) {
        self.experiment_id = experiment_id.trim().to_string();
    }

    pub fn decision(&self, context: DecisionContext<'_>) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let state_sha256 = format!("{:x}", Sha256::digest(context.sfen.as_bytes()));
        let game_id = format!("{}-g{:06}", context.experiment_id, context.game_counter);
        let record = DecisionRecord {
            schema: "sekirei-book-decision-v1",
            event: "decision",
            experiment_id: context.experiment_id,
            game_id: game_id.clone(),
            decision_id: format!("{game_id}-d{:06}", context.decision_counter),
            ply: context.ply,
            state_sha256,
            use_book: context.use_book,
            book_max_ply: context.book_max_ply,
            book_min_confidence: context.book_min_confidence,
            book_sha256: context.book_sha256,
            book_schema_version: context.book_schema_version,
            book_producer_version: context.book_producer_version,
            build_config_fingerprint: context.build_config_fingerprint,
            candidates: context.candidates,
            selected_action: context.selected_action,
            action_source: context.action_source,
            fallback_reason: context.fallback_reason,
            policy_version: POLICY_VERSION,
            evaluation_contract: "paired-ab; no propensity because policy is deterministic",
        };
        append_json(path, &record)
    }

    pub fn terminal(&self, game_counter: u64, result: &str) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let experiment_id = self.experiment_id();
        append_json(
            path,
            &TerminalRecord {
                schema: "sekirei-book-decision-v1",
                event: "terminal",
                experiment_id,
                game_id: format!("{experiment_id}-g{game_counter:06}"),
                result,
            },
        )
    }
}

fn append_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open book decision log {path:?}: {error}"))?;
    serde_json::to_writer(&mut file, value)
        .map_err(|error| format!("cannot serialize book decision log: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("cannot append book decision log {path:?}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_logger_does_not_create_a_file() {
        let logger = BookDecisionLogger::default();
        assert!(!logger.enabled());
        logger.terminal(1, "draw").unwrap();
    }

    #[test]
    fn decision_and_terminal_share_the_game_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("book.jsonl");
        let mut logger = BookDecisionLogger::default();
        logger.configure(path.to_str().unwrap(), "heldout-a");
        let candidates = vec![BookCandidate {
            rank: 1,
            action: "7g7f".to_string(),
            prior: 0.75,
            confidence: 0.9,
            confidence_pass: true,
            legal: true,
        }];
        logger
            .decision(DecisionContext {
                experiment_id: logger.experiment_id(),
                game_counter: 3,
                decision_counter: 1,
                ply: 0,
                sfen: "state",
                use_book: true,
                book_max_ply: 30,
                book_min_confidence: 0.2,
                book_sha256: Some("abc"),
                book_schema_version: Some(1),
                book_producer_version: Some("lineprior-0.12.1"),
                build_config_fingerprint: Some(4),
                candidates: &candidates,
                selected_action: Some("7g7f"),
                action_source: "book",
                fallback_reason: None,
            })
            .unwrap();
        logger.terminal(3, "win").unwrap();
        let records: Vec<serde_json::Value> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["game_id"], records[1]["game_id"]);
        assert_eq!(records[0]["selected_action"], "7g7f");
        assert_eq!(
            records[0]["evaluation_contract"],
            "paired-ab; no propensity because policy is deterministic"
        );
    }
}
