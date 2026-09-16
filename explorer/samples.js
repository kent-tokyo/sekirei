'use strict';

/*
 * Bundled sample records for Sekirei Explorer.
 *
 * These are ILLUSTRATIVE, not measured: no real engine analysis was run
 * to produce the score_cp/score_mate/pv/bestmove values below. Only the
 * positions themselves are real, sourced from this project's own tracked
 * Rust source (see each entry's meta.citation) -- no bulk SFEN corpus is
 * committed to this repository (data/ is gitignored), so these three
 * hand-picked positions are what's actually available to demo with.
 *
 * engine.version is the deliberately non-real "0.0.0-illustrative", and
 * every *_sha256 is 64 zero characters -- a real SHA-256 digest is never
 * all-zero, so these can't be mistaken for a plausible-looking real hash.
 */

const ZERO_SHA256 = '0'.repeat(64);

window.SEKIREI_EXPLORER_SAMPLES = [
  {
    meta: {
      label: 'Startpos (illustrative)',
      citation: "Sekirei's own opening SFEN — crates/sekirei-core/src/sfen.rs",
    },
    record: {
      schema_version: '1',
      sample_id: 'sample-startpos',
      game_id: 'sekirei-explorer-samples',
      ply: 0,
      sfen: 'lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1',
      engine: {
        name: 'sekirei',
        version: '0.0.0-illustrative',
        build_info: null,
        binary_sha256: ZERO_SHA256,
        weight_sha256: null,
      },
      settings: { threads: 1, hash_mb: 64, multipv: 1, depth: 6 },
      lines: [
        {
          multipv: 1,
          score_cp: 42,
          bestmove: '7g7f',
          pv: ['7g7f'],
          depth: 6,
          nodes: 12345,
          time_ms: 15,
          nps: 823000,
        },
      ],
      status: 'ok',
      error_detail: null,
      wall_time_ms: 21,
      bestmove: '7g7f',
    },
  },
  {
    meta: {
      label: 'Real game excerpt, ply 24 (illustrative analysis)',
      citation:
        'Base position from a real completed engine game this project played — ' +
        'crates/sekirei-usi/src/invariant.rs:523-529, test replay_handles_capture_promotion_and_drop',
    },
    record: {
      schema_version: '1',
      sample_id: 'sample-game-excerpt-ply24',
      game_id: 'sekirei-explorer-samples',
      ply: 24,
      sfen: 'l4gknl/1r2g1sb1/n1pspppp1/pp1p4p/6PP1/P1PS1S3/1P1PPP2P/1B5R1/LN1GKG1NL w - 24',
      engine: {
        name: 'sekirei',
        version: '0.0.0-illustrative',
        build_info: null,
        binary_sha256: ZERO_SHA256,
        weight_sha256: null,
      },
      settings: { threads: 1, hash_mb: 64, multipv: 3, depth: 8 },
      lines: [
        {
          multipv: 1,
          score_cp: -35,
          bestmove: '8e8f',
          pv: ['8e8f', '6f7e'],
          depth: 8,
          nodes: 210443,
          time_ms: 180,
          nps: 1169000,
        },
        {
          multipv: 2,
          score_cp: -80,
          bestmove: '7c7d',
          pv: ['7c7d'],
          depth: 8,
          nodes: 210443,
          time_ms: 180,
          nps: 1169000,
        },
        {
          multipv: 3,
          score_cp: -120,
          bound: 'upperbound',
          bestmove: '3a4b',
          pv: ['3a4b'],
          depth: 8,
          nodes: 210443,
          time_ms: 180,
          nps: 1169000,
        },
      ],
      status: 'ok',
      error_detail: null,
      wall_time_ms: 205,
      bestmove: '8e8f',
    },
  },
  {
    meta: {
      label: 'Mate-in-1 (illustrative)',
      citation: 'crates/sekirei-core/src/search.rs:2189, MATE_IN_1_SFEN',
    },
    record: {
      schema_version: '1',
      sample_id: 'sample-mate-in-1',
      game_id: 'sekirei-explorer-samples',
      ply: 1,
      sfen: 'k8/2K6/9/9/4R4/9/9/9/9 b - 1',
      engine: {
        name: 'sekirei',
        version: '0.0.0-illustrative',
        build_info: null,
        binary_sha256: ZERO_SHA256,
        weight_sha256: null,
      },
      settings: { threads: 1, hash_mb: 64, multipv: 1, depth: 1 },
      lines: [
        {
          multipv: 1,
          score_mate: 1,
          bound: 'exact',
          bestmove: '5e9e',
          pv: ['5e9e'],
          depth: 1,
          nodes: 50,
          time_ms: 1,
          nps: 50000,
        },
      ],
      status: 'ok',
      error_detail: null,
      wall_time_ms: 3,
      bestmove: '5e9e',
    },
  },
];
