//! Versioned, bounded, opt-in artifacts for Sekirei-owned small graphs.
//! Statistical prior books have a separate format and execution path.

use std::io::Read;

use sekirei_core::board::Board;
use sekirei_core::book_graph::{
    BookGraph, BookGraphDecision, BookGraphLimits, BookGraphNode, BookGraphSuccessor,
    BookGraphTarget, MAX_GRAPH_EDGES, MAX_GRAPH_NODES, MAX_PROOF_NODES, MAX_PROOF_PLIES,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const SCHEMA: &str = "sekirei.cyclic-book";
const VERSION: u32 = 1;
const RULES: &str = "exact-history-checkmate-only-v1";
const MAX_ARTIFACT_BYTES: usize = 1_048_576;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    schema: String,
    version: u32,
    rules: String,
    producer: String,
    source: Source,
    verification: Verification,
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Source {
    SekireiOwned,
    Synthetic,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Verification {
    max_depth: u16,
    node_limit: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    sfen: String,
    successors: Vec<Successor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Successor {
    action: String,
    // A missing `target` field is an error; only explicit JSON null is Unknown.
    #[serde(deserialize_with = "required_target")]
    target: Option<usize>,
    gave_check: bool,
}

fn required_target<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<usize>, D::Error> {
    Option::<usize>::deserialize(deserializer)
}

pub struct CyclicBook {
    graph: BookGraph,
    limits: BookGraphLimits,
    sha256: String,
    positions: usize,
}

impl CyclicBook {
    pub fn load(path: &str) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        file.take((MAX_ARTIFACT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        Self::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err("cyclic-book artifact exceeds 1 MiB".into());
        }
        let artifact: Artifact =
            serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if artifact.schema != SCHEMA || artifact.version != VERSION || artifact.rules != RULES {
            return Err("unsupported cyclic-book schema, version, or rules".into());
        }
        if artifact.producer.is_empty() || artifact.producer.len() > 128 {
            return Err("cyclic-book producer must contain 1..=128 bytes".into());
        }
        // Source is a required provenance declaration, not a proof of rights.
        match artifact.source {
            Source::SekireiOwned | Source::Synthetic => {}
        }
        if artifact.nodes.is_empty() || artifact.nodes.len() > MAX_GRAPH_NODES {
            return Err("cyclic-book node count must be in 1..=256".into());
        }
        let edge_count = artifact
            .nodes
            .iter()
            .try_fold(0usize, |sum, node| sum.checked_add(node.successors.len()))
            .ok_or("cyclic-book edge count overflow")?;
        if edge_count > MAX_GRAPH_EDGES {
            return Err("cyclic-book has too many successors".into());
        }
        let limits = BookGraphLimits {
            max_depth: artifact.verification.max_depth,
            node_limit: artifact.verification.node_limit,
        };
        if limits.max_depth == 0
            || limits.max_depth > MAX_PROOF_PLIES
            || limits.node_limit == 0
            || limits.node_limit > MAX_PROOF_NODES
        {
            return Err("cyclic-book verification limits exceed the runtime contract".into());
        }
        let nodes = artifact
            .nodes
            .into_iter()
            .map(|node| BookGraphNode {
                sfen: node.sfen,
                successors: node
                    .successors
                    .into_iter()
                    .map(|successor| BookGraphSuccessor {
                        action: successor.action,
                        target: successor
                            .target
                            .map_or(BookGraphTarget::Unknown, BookGraphTarget::Known),
                        gave_check: successor.gave_check,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let graph = BookGraph::from_nodes(&nodes)?;
        Ok(Self {
            graph,
            limits,
            positions: nodes.len(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn len(&self) -> usize {
        self.positions
    }

    pub fn decision(&self, position: Option<&str>, current: &Board) -> BookGraphDecision {
        self.graph.decision(position, current, self.limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::{
        book_graph::BookGraphFallback,
        movegen::{generate_legal_moves, is_in_check},
        sfen::{board_to_sfen, move_from_usi, move_to_usi},
    };
    use serde_json::{Value, json};

    const ORIGIN: &str = "4k4/9/4P4/9/9/9/9/9/4K4 b G 1";

    fn fixture() -> Value {
        let root = Board::from_sfen(ORIGIN).unwrap();
        let mut mate = root.clone();
        mate.do_move(move_from_usi("G*5b", &root).unwrap());
        let key = |board: &Board| {
            board_to_sfen(board)
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        };
        let boards = [root, mate];
        let nodes = boards.iter().map(|board| {
            let successors = generate_legal_moves(&mut board.clone()).into_iter().map(|mv| {
                let mut after = board.clone();
                after.do_move(mv);
                json!({"action": move_to_usi(mv), "target": boards.iter().position(|target| key(target) == key(&after)), "gave_check": is_in_check(&after, after.side_to_move)})
            }).collect::<Vec<_>>();
            json!({"sfen": board_to_sfen(board), "successors": successors})
        }).collect::<Vec<_>>();
        json!({"schema": SCHEMA, "version": VERSION, "rules": RULES, "producer": "synthetic-test", "source": "synthetic", "verification": {"max_depth": 4, "node_limit": 1000}, "nodes": nodes})
    }

    #[test]
    fn versioned_owned_fixture_verifies_mate_and_hash() {
        let bytes = serde_json::to_vec(&fixture()).unwrap();
        let book = CyclicBook::from_bytes(&bytes).unwrap();
        assert_eq!(book.len(), 2);
        assert_eq!(book.sha256(), format!("{:x}", Sha256::digest(&bytes)));
        let root = Board::from_sfen(ORIGIN).unwrap();
        let BookGraphDecision::ProvenMate {
            best_move,
            proof_plies,
        } = book.decision(Some(&format!("sfen {ORIGIN}")), &root)
        else {
            panic!("verified mate")
        };
        assert_eq!(move_to_usi(best_move), "G*5b");
        assert_eq!(proof_plies, 1);
        assert!(matches!(
            book.decision(None, &root),
            BookGraphDecision::Fallback(BookGraphFallback::HistoryUnavailable)
        ));
    }

    #[test]
    fn incompatible_schema_rules_provenance_and_bounds_are_rejected() {
        for (key, replacement) in [
            ("schema", json!("lineprior")),
            ("version", json!(2)),
            ("rules", json!("unverified-evaluation")),
            ("source", json!("external")),
            ("producer", json!("")),
            ("verification", json!({"max_depth": 17, "node_limit": 1})),
            ("verification", json!({"max_depth": 1, "node_limit": 10001})),
        ] {
            let mut bad = fixture();
            bad[key] = replacement;
            assert!(
                CyclicBook::from_bytes(&serde_json::to_vec(&bad).unwrap()).is_err(),
                "{key}"
            );
        }
        assert!(CyclicBook::from_bytes(&vec![b' '; MAX_ARTIFACT_BYTES + 1]).is_err());
    }

    #[test]
    fn unknown_fields_missing_targets_and_forged_check_labels_are_rejected() {
        for mutation in 0..4 {
            let mut bad = fixture();
            match mutation {
                0 => bad["unverified_win"] = json!(true),
                1 => {
                    bad["nodes"][0]["successors"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("target");
                }
                2 => {
                    let value = bad["nodes"][0]["successors"][0]["gave_check"]
                        .as_bool()
                        .unwrap();
                    bad["nodes"][0]["successors"][0]["gave_check"] = json!(!value);
                }
                3 => {
                    bad["nodes"][0]["successors"].as_array_mut().unwrap().pop();
                }
                _ => unreachable!(),
            }
            assert!(
                CyclicBook::from_bytes(&serde_json::to_vec(&bad).unwrap()).is_err(),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn unknown_mate_target_or_exhausted_verification_does_not_select_a_move() {
        for capped in [false, true] {
            let mut input = fixture();
            if capped {
                input["verification"]["node_limit"] = json!(1);
            } else {
                for edge in input["nodes"][0]["successors"].as_array_mut().unwrap() {
                    edge["target"] = Value::Null;
                }
            }
            let book = CyclicBook::from_bytes(&serde_json::to_vec(&input).unwrap()).unwrap();
            assert!(matches!(
                book.decision(
                    Some(&format!("sfen {ORIGIN}")),
                    &Board::from_sfen(ORIGIN).unwrap()
                ),
                BookGraphDecision::Fallback(BookGraphFallback::Unknown)
            ));
        }
    }
}
