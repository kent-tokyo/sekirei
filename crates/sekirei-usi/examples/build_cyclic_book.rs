//! Build bounded cyclic-book artifacts from an explicit, versioned position set.
//! This example never expands missing positions or changes engine defaults.

#[cfg(feature = "cyclic-book")]
pub(crate) use enabled::run;
#[cfg(all(feature = "cyclic-book", test))]
pub(crate) use enabled::{build, verify};

#[cfg(feature = "cyclic-book")]
fn main() {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("cyclic-book builder: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(feature = "cyclic-book"))]
fn main() {
    eprintln!("build_cyclic_book requires --features cyclic-book");
    std::process::exit(2);
}

#[cfg(feature = "cyclic-book")]
mod enabled {
    use std::collections::HashMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Write};
    use std::path::Path;

    use sekirei_core::{
        board::Board,
        book_graph::{
            BookGraph, BookGraphNode, BookGraphSuccessor, BookGraphTarget, MAX_GRAPH_EDGES,
            MAX_GRAPH_NODES, MAX_PROOF_NODES, MAX_PROOF_PLIES,
        },
        color::Color,
        movegen::{generate_legal_moves, is_in_check},
        piece::PieceKind,
        sfen::{board_to_sfen, move_to_usi},
    };
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};

    const MAX_BYTES: usize = 1_048_576;
    const INPUT_SCHEMA: &str = "sekirei.cyclic-book-positions";
    const ARTIFACT_SCHEMA: &str = "sekirei.cyclic-book";
    const PROVENANCE_SCHEMA: &str = "sekirei.cyclic-book-provenance";
    const RULES: &str = "exact-history-checkmate-only-v1";

    #[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum Source {
        Synthetic,
        SekireiOwned,
    }

    #[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    struct Verification {
        max_depth: u16,
        node_limit: u64,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        schema: String,
        version: u32,
        producer: String,
        source: Source,
        source_declaration: String,
        verification: Verification,
        positions: Vec<String>,
    }

    #[derive(Serialize)]
    struct Successor {
        action: String,
        target: Option<usize>,
        gave_check: bool,
    }

    #[derive(Serialize)]
    struct Node {
        sfen: String,
        successors: Vec<Successor>,
    }

    #[derive(Serialize)]
    struct Artifact {
        schema: &'static str,
        version: u32,
        rules: &'static str,
        producer: String,
        source: Source,
        verification: Verification,
        nodes: Vec<Node>,
    }

    #[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    struct Provenance {
        schema: String,
        version: u32,
        input_schema: String,
        input_sha256: String,
        artifact_schema: String,
        artifact_file: String,
        artifact_sha256: String,
        builder_version: String,
        builder_source_sha256: String,
        input_producer: String,
        source: Source,
        source_declaration: String,
        ownership_verified: bool,
        positions: usize,
        legal_successors: usize,
        unknown_successors: usize,
        verification: Verification,
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .take((maximum + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > maximum {
            return Err(format!("{} exceeds {maximum} bytes", path.display()));
        }
        Ok(bytes)
    }

    fn encode(value: &impl Serialize) -> Result<Vec<u8>, String> {
        let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err("generated artifact exceeds 1 MiB".into());
        }
        Ok(bytes)
    }

    fn exact_key(board: &Board) -> String {
        board_to_sfen(board)
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn prepare(bytes: &[u8]) -> Result<(Vec<u8>, Provenance), String> {
        let input: Input = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if input.schema != INPUT_SCHEMA || input.version != 1 {
            return Err("unsupported position-input schema or version".into());
        }
        if input.producer.is_empty()
            || input.producer.len() > 128
            || input.source_declaration.is_empty()
            || input.source_declaration.len() > 512
        {
            return Err("producer or source declaration is empty or too long".into());
        }
        if input.positions.is_empty() || input.positions.len() > MAX_GRAPH_NODES {
            return Err("input requires 1..=256 positions".into());
        }
        if input.verification.max_depth == 0
            || input.verification.max_depth > MAX_PROOF_PLIES
            || input.verification.node_limit == 0
            || input.verification.node_limit > MAX_PROOF_NODES
        {
            return Err("verification limits exceed the loader contract".into());
        }
        let mut boards = Vec::new();
        let mut ids = HashMap::new();
        for sfen in &input.positions {
            if sfen.len() > 256 {
                return Err("input SFEN exceeds 256 bytes".into());
            }
            let mut board = Board::from_sfen(sfen)?;
            board.ply = 0;
            // Cheap shape checks precede movegen. The shared verifier below
            // owns the complete admission policy, including inventory/nifu.
            if [Color::Black, Color::White]
                .into_iter()
                .any(|side| board.pieces(side, PieceKind::Ou).popcount() != 1)
                || is_in_check(&board, board.side_to_move.flip())
            {
                return Err("invalid king presence or non-moving king in check".into());
            }
            if ids.insert(exact_key(&board), boards.len()).is_some() {
                return Err("duplicate exact position, including alternate move numbers".into());
            }
            boards.push(board);
        }
        let mut nodes = Vec::new();
        let mut total = 0usize;
        let mut unknown = 0usize;
        for board in &boards {
            let legal = generate_legal_moves(&mut board.clone());
            total = total
                .checked_add(legal.len())
                .ok_or("legal successor count overflow")?;
            if total > MAX_GRAPH_EDGES {
                return Err("generated graph exceeds 16384 legal successors".into());
            }
            let mut successors = legal
                .into_iter()
                .map(|mv| {
                    let mut after = board.clone();
                    after.do_move(mv);
                    let target = ids.get(&exact_key(&after)).copied();
                    if target.is_none() {
                        unknown += 1;
                    }
                    Successor {
                        action: move_to_usi(mv),
                        target,
                        gave_check: is_in_check(&after, after.side_to_move),
                    }
                })
                .collect::<Vec<_>>();
            successors.sort_by(|left, right| left.action.cmp(&right.action));
            nodes.push(Node {
                sfen: board_to_sfen(board),
                successors,
            });
        }
        let checked = nodes
            .iter()
            .map(|node| BookGraphNode {
                sfen: node.sfen.clone(),
                successors: node
                    .successors
                    .iter()
                    .map(|edge| BookGraphSuccessor {
                        action: edge.action.clone(),
                        target: edge
                            .target
                            .map_or(BookGraphTarget::Unknown, BookGraphTarget::Known),
                        gave_check: edge.gave_check,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        BookGraph::from_nodes(&checked)?;
        let artifact = Artifact {
            schema: ARTIFACT_SCHEMA,
            version: 1,
            rules: RULES,
            producer: format!("sekirei-cyclic-book-builder/{}", env!("CARGO_PKG_VERSION")),
            source: input.source.clone(),
            verification: input.verification.clone(),
            nodes,
        };
        let output = encode(&artifact)?;
        let provenance = Provenance {
            schema: PROVENANCE_SCHEMA.into(),
            version: 1,
            input_schema: INPUT_SCHEMA.into(),
            input_sha256: sha(bytes),
            artifact_schema: ARTIFACT_SCHEMA.into(),
            artifact_file: "book.json".into(),
            artifact_sha256: sha(&output),
            builder_version: env!("CARGO_PKG_VERSION").into(),
            builder_source_sha256: sha(include_bytes!("build_cyclic_book.rs")),
            input_producer: input.producer,
            source: input.source,
            source_declaration: input.source_declaration,
            ownership_verified: false,
            positions: boards.len(),
            legal_successors: total,
            unknown_successors: unknown,
            verification: input.verification,
        };
        Ok((output, provenance))
    }

    fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn build(input: &Path, output: &Path) -> Result<String, String> {
        let bytes = read_bounded(input, MAX_BYTES)?;
        let (artifact, provenance) = prepare(&bytes)?;
        let receipt = encode(&provenance)?;
        // No overwrite or resume mode: an existing directory is always an
        // error. On I/O failure, retain the new incomplete directory for audit.
        fs::create_dir(output)
            .map_err(|error| format!("new output directory {}: {error}", output.display()))?;
        write_new(&output.join("book.json"), &artifact)?;
        // Written last. A missing receipt means the bundle is incomplete.
        write_new(&output.join("provenance.json"), &receipt)?;
        verify(input, output)?;
        Ok(format!(
            "built {} positions, {} legal successors ({} unknown), artifact SHA-256 {}; source claim unverified",
            provenance.positions,
            provenance.legal_successors,
            provenance.unknown_successors,
            provenance.artifact_sha256
        ))
    }

    pub(crate) fn verify(input: &Path, output: &Path) -> Result<String, String> {
        let bytes = read_bounded(input, MAX_BYTES)?;
        let (expected_artifact, expected_provenance) = prepare(&bytes)?;
        let artifact = read_bounded(&output.join("book.json"), MAX_BYTES)?;
        let receipt = read_bounded(&output.join("provenance.json"), 16_384)?;
        let provenance: Provenance =
            serde_json::from_slice(&receipt).map_err(|error| error.to_string())?;
        if provenance != expected_provenance || artifact != expected_artifact {
            return Err("input, artifact, provenance, or builder identity does not match reproducible generation".into());
        }
        Ok(format!(
            "verified artifact SHA-256 {}; source claim remains unverified",
            sha(&artifact)
        ))
    }

    pub(crate) fn run(args: &[String]) -> Result<String, String> {
        match args
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["build", input, output] => build(Path::new(input), Path::new(output)),
            ["verify", input, output] => verify(Path::new(input), Path::new(output)),
            ["--help"] => Ok("build_cyclic_book build|verify INPUT.json OUTPUT_DIRECTORY".into()),
            _ => Err("usage: build_cyclic_book build|verify INPUT.json OUTPUT_DIRECTORY".into()),
        }
    }
}
