'use strict';

/*
 * Sekirei Explorer — static, client-only viewer.
 *
 * Nothing here computes a new analysis. This file only parses SFEN
 * position strings and USI move notation (matching Sekirei's own dialect,
 * see crates/sekirei-core/src/sfen.rs and board.rs) and renders whatever
 * a loaded JSON/JSONL file already contains.
 */

// ---- Piece tables (crates/sekirei-core/src/sfen.rs) ----

const BASE_KINDS = ['P', 'L', 'N', 'S', 'G', 'B', 'R', 'K'];
const PIECE_GLYPH = { P: '歩', L: '香', N: '桂', S: '銀', G: '金', B: '角', R: '飛', K: '玉' };
const PROMOTED_GLYPH = { P: 'と', L: '杏', N: '圭', S: '全', B: '馬', R: '龍' };
const PIECE_NAME = {
  P: 'pawn', L: 'lance', N: 'knight', S: 'silver', G: 'gold', B: 'bishop', R: 'rook', K: 'king',
};

function pieceGlyph(kind, promoted) {
  if (promoted && PROMOTED_GLYPH[kind]) return PROMOTED_GLYPH[kind];
  return PIECE_GLYPH[kind] || '?';
}

// file 1-9, rank 1-9 -> "7g" style USI square label (rank 1=a .. 9=i).
function squareLabel(file, rank) {
  return `${file}${String.fromCharCode('a'.charCodeAt(0) + (rank - 1))}`;
}

// ---- SFEN parsing ----

// Board field ("lnsgkgsnl/1r5b1/...") -> flat 81-cell array.
// cells[idx] is null (empty) or {color:'b'|'w', kind, promoted}.
// idx = (rank-1)*9 + (9-file) -- this is exactly SFEN's own emit order
// (rank 1..9 top-to-bottom, each rank read left-to-right = file 9..1),
// so parsing fills the array sequentially with no reordering math.
function parseSfenBoard(boardField) {
  const ranks = boardField.split('/');
  if (ranks.length !== 9) {
    throw new Error(`SFEN board field must have 9 ranks, got ${ranks.length}`);
  }
  const cells = new Array(81).fill(null);
  ranks.forEach((rankStr, rankIdx0) => {
    const rank = rankIdx0 + 1;
    let file = 9;
    let pendingPromote = false;
    for (let i = 0; i < rankStr.length; i += 1) {
      const ch = rankStr[i];
      if (ch === '+') {
        pendingPromote = true;
        continue;
      }
      if (ch >= '1' && ch <= '9') {
        file -= Number(ch);
        continue;
      }
      const upper = ch.toUpperCase();
      if (BASE_KINDS.indexOf(upper) === -1) {
        throw new Error(`Unrecognized SFEN piece letter '${ch}' in rank ${rank}`);
      }
      if (file < 1) {
        throw new Error(`SFEN rank ${rank} overflows 9 files`);
      }
      const idx = (rank - 1) * 9 + (9 - file);
      cells[idx] = { color: ch === upper ? 'b' : 'w', kind: upper, promoted: pendingPromote };
      pendingPromote = false;
      file -= 1;
    }
  });
  return cells;
}

// Hand/mochigoma field ("2P" / "-" / "RBGSNLPrbgsnlp...") -> {b:{kind:count}, w:{...}}.
// A digit run immediately before a piece letter is that piece's count (>=2);
// a bare letter means count 1. King never appears here (not a droppable piece).
function parseHandField(handField) {
  const hands = { b: {}, w: {} };
  if (handField === '-') return hands;
  let countStr = '';
  for (let i = 0; i < handField.length; i += 1) {
    const ch = handField[i];
    if (ch >= '0' && ch <= '9') {
      countStr += ch;
      continue;
    }
    const upper = ch.toUpperCase();
    if (BASE_KINDS.indexOf(upper) === -1 || upper === 'K') {
      throw new Error(`Unrecognized hand piece letter '${ch}'`);
    }
    const color = ch === upper ? 'b' : 'w';
    const count = countStr === '' ? 1 : Number(countStr);
    hands[color][upper] = (hands[color][upper] || 0) + count;
    countStr = '';
  }
  return hands;
}

// Full SFEN string -> {cells, sideToMove, hands, ply, raw}. Throws a
// human-readable Error on malformed input -- callers must catch this
// (an uploaded file's sfen field is untrusted).
function parseSfen(sfenString) {
  const raw = sfenString.trim();
  const parts = raw.split(/\s+/);
  if (parts.length < 3) {
    throw new Error(`SFEN needs at least 3 fields (board, side, hand), got ${parts.length}`);
  }
  const [boardField, sideField, handField, plyField] = parts;
  if (sideField !== 'b' && sideField !== 'w') {
    throw new Error(`SFEN side-to-move must be 'b' or 'w', got '${sideField}'`);
  }
  return {
    cells: parseSfenBoard(boardField),
    sideToMove: sideField,
    hands: parseHandField(handField),
    ply: plyField !== undefined && /^\d+$/.test(plyField) ? Number(plyField) : null,
    raw,
  };
}

// ---- USI move-token parsing (display only -- see note below) ----
//
// ponytail: PV is shown as a static token list only; a "step through the
// PV on the board" feature would need a full applyUsiMove(cells, token)
// (capture-to-hand, promotion, drop-from-hand, side-to-move flip) -- real
// extra logic for a feature nobody asked for. Add it if requested; it
// would reuse parseSfen's cell array unchanged.

// "7g7f" -> {type:'move', from, to, promote:false}
// "8h2b+" -> {type:'move', from, to, promote:true}
// "P*3d" -> {type:'drop', piece:'P', to}
// "resign"/"win" -> {type:'special'}
function parseUsiToken(token) {
  if (!token) return { type: 'unknown', raw: token };
  if (token === 'resign' || token === 'win') return { type: 'special', raw: token };
  if (token.length < 4) return { type: 'unknown', raw: token };
  if (token[1] === '*') {
    return { type: 'drop', piece: token[0].toUpperCase(), to: token.slice(2, 4), raw: token };
  }
  return {
    type: 'move',
    from: token.slice(0, 2),
    to: token.slice(2, 4),
    promote: token.length > 4 && token[4] === '+',
    raw: token,
  };
}

// ---- JSON import ----
//
// Record shape understood here follows analysis_record_v1 as documented
// in schemas/analysis_record_v1.schema.json and
// docs/amateur_analysis_benchmark.md on branch
// feat/amateur-analysis-benchmark-kit (PR #51, open/unmerged as of
// writing). This parser is intentionally tolerant and does not require
// that file to exist on whatever branch this viewer ships from -- see
// normalizeRecord() below, which never throws on a partial record.

// text -> {records, format:'single'|'array'|'jsonl'|'empty', errors:[{line,message}]}
// Whole-text JSON.parse is attempted FIRST -- a pretty-printed single
// object has embedded newlines and would misparse as broken JSONL if
// line-splitting ran first. Only falls back to line-by-line on failure.
function parseInput(text) {
  const trimmed = text.trim();
  if (!trimmed) return { records: [], format: 'empty', errors: [] };
  try {
    const whole = JSON.parse(trimmed);
    if (Array.isArray(whole)) return { records: whole, format: 'array', errors: [] };
    return { records: [whole], format: 'single', errors: [] };
  } catch (e) {
    // fall through to JSONL
  }
  const records = [];
  const errors = [];
  trimmed.split('\n').forEach((line, i) => {
    const t = line.trim();
    if (!t) return;
    try {
      records.push(JSON.parse(t));
    } catch (e) {
      errors.push({ line: i + 1, message: e.message });
    }
  });
  return { records, format: 'jsonl', errors };
}

// raw parsed JSON -> {record, warnings}. Never throws. Missing fields are
// left absent (rendered as "-"), not synthesized. `sfen` is the one
// load-bearing field for board rendering -- checked here only to warn;
// actual parse failure is caught where parseSfen() is called.
function normalizeRecord(raw) {
  const warnings = [];
  const record = raw && typeof raw === 'object' ? raw : {};
  if (typeof record.sfen !== 'string' || !record.sfen.trim()) {
    warnings.push('missing or invalid "sfen" -- board cannot be rendered for this record');
  }
  if (!Array.isArray(record.lines)) warnings.push('missing "lines" -- no candidate moves to show');
  if (!record.engine || typeof record.engine !== 'object') warnings.push('missing "engine" block');
  if (!record.settings || typeof record.settings !== 'object') warnings.push('missing "settings" block');
  if (!record.status) warnings.push('missing "status"');
  return { record, warnings };
}

// ---- DOM rendering ----
//
// Every piece of dynamic text below reaches the DOM via textContent
// (through el()'s createTextNode) or element.textContent directly --
// never innerHTML -- so an uploaded file's strings (sfen, sample_id,
// error_detail, etc.) can never be interpreted as markup.

function el(tag, attrs, children) {
  const node = document.createElement(tag);
  if (attrs) {
    Object.keys(attrs).forEach((k) => {
      if (k === 'class') node.className = attrs[k];
      else node.setAttribute(k, attrs[k]);
    });
  }
  (children || []).forEach((child) => {
    if (child === undefined || child === null) return;
    node.appendChild(typeof child === 'string' ? document.createTextNode(child) : child);
  });
  return node;
}

function clearChildren(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function renderBoardTable(parsed) {
  const table = el('table', { class: 'board' }, [el('caption', null, [parsed.raw])]);

  const headRow = el('tr', null, [el('th', null, [''])]);
  for (let file = 9; file >= 1; file -= 1) {
    headRow.appendChild(el('th', { scope: 'col' }, [String(file)]));
  }
  table.appendChild(el('thead', null, [headRow]));

  const tbody = el('tbody');
  for (let rank = 1; rank <= 9; rank += 1) {
    const rankLetter = String.fromCharCode('a'.charCodeAt(0) + (rank - 1));
    const tr = el('tr', null, [el('th', { scope: 'row' }, [rankLetter])]);
    for (let file = 9; file >= 1; file -= 1) {
      const idx = (rank - 1) * 9 + (9 - file);
      const piece = parsed.cells[idx];
      const label = squareLabel(file, rank);
      if (piece) {
        const span = el('span', { class: `piece ${piece.color === 'w' ? 'piece-gote' : 'piece-sente'}` }, [
          pieceGlyph(piece.kind, piece.promoted),
        ]);
        const ownerWord = piece.color === 'b' ? 'black' : 'white';
        const promoWord = piece.promoted ? 'promoted ' : '';
        tr.appendChild(el('td', { 'aria-label': `${label}: ${ownerWord} ${promoWord}${PIECE_NAME[piece.kind]}` }, [span]));
      } else {
        tr.appendChild(el('td', { 'aria-label': `${label}: empty` }, []));
      }
    }
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  return table;
}

function renderHandsSummary(hands) {
  const div = el('div', { class: 'hands' });
  [['b', 'Black in hand'], ['w', 'White in hand']].forEach(([color, label]) => {
    const entries = Object.keys(hands[color]).map((kind) => `${PIECE_NAME[kind]} ×${hands[color][kind]}`);
    div.appendChild(el('p', null, [`${label}: ${entries.length ? entries.join(', ') : 'none'}`]));
  });
  return div;
}

function renderPvList(pv) {
  const ol = el('ol', { class: 'pv-list' });
  (pv || []).forEach((token) => {
    const parsed = parseUsiToken(token);
    const li = el('li', null, [el('code', null, [token])]);
    if (parsed.type === 'drop') li.appendChild(el('span', { class: 'tag tag-drop' }, ['drop']));
    else if (parsed.type === 'move' && parsed.promote) li.appendChild(el('span', { class: 'tag tag-promote' }, ['promotes']));
    ol.appendChild(li);
  });
  return ol;
}

function fieldRow(label, value) {
  const shown = value === undefined || value === null || value === '' ? '—' : String(value);
  return el('tr', null, [el('th', { scope: 'row' }, [label]), el('td', null, [shown])]);
}

function shaCaption(hash) {
  if (!hash) return '—';
  return `${hash} (self-reported by the producing tool, not verified here)`;
}

function scoreText(line) {
  if (line.score_cp !== undefined && line.score_cp !== null) return `${line.score_cp} cp`;
  if (line.score_mate !== undefined && line.score_mate !== null) return `mate ${line.score_mate}`;
  return '—';
}

function renderLineRow(line) {
  const pvTd = el('td', null, [renderPvList(line.pv)]);
  return el('tr', null, [
    el('td', null, [line.multipv !== undefined ? String(line.multipv) : '—']),
    el('td', null, [scoreText(line)]),
    el('td', null, [line.bound || '—']),
    el('td', null, [line.bestmove || '—']),
    pvTd,
    el('td', null, [line.depth !== undefined ? String(line.depth) : '—']),
    el('td', null, [line.nodes !== undefined ? String(line.nodes) : '—']),
    el('td', null, [line.time_ms !== undefined ? `${line.time_ms} ms` : '—']),
    el('td', null, [line.nps !== undefined ? String(line.nps) : '—']),
  ]);
}

function keyValueTable(pairs) {
  const tbody = el('tbody', null, pairs.map(([label, value]) => fieldRow(label, value)));
  return el('table', { class: 'meta-table' }, [tbody]);
}

function renderRecordDetail(record, provenance, warnings) {
  const container = el('div', { class: 'record-detail' });

  container.appendChild(el('p', { class: 'disclosure-restate', role: 'note' }, [
    'Static viewer only — nothing on this page computes a new analysis.',
  ]));

  container.appendChild(
    provenance === 'sample'
      ? el('div', { class: 'provenance-badge illustrative', role: 'status' }, [
          '⚠ Illustrative — fabricated for this demo, not a measured engine run. ',
          el('a', { href: '#about' }, ['Why']),
        ])
      : el('div', { class: 'provenance-badge unverified', role: 'status' }, [
          'ℹ Loaded from your file — this page cannot verify these numbers came from a real engine run.',
        ]),
  );

  if (warnings.length) {
    container.appendChild(el('div', { class: 'warnings', role: 'status' }, [
      el('p', null, ['This record did not fully match the expected shape:']),
      el('ul', null, warnings.map((w) => el('li', null, [w]))),
    ]));
  }

  if (typeof record.sfen === 'string' && record.sfen.trim()) {
    try {
      const parsed = parseSfen(record.sfen);
      container.appendChild(renderBoardTable(parsed));
      container.appendChild(renderHandsSummary(parsed.hands));
    } catch (e) {
      container.appendChild(el('div', { class: 'error-banner', role: 'alert' }, [
        `Could not render board from sfen "${record.sfen}": ${e.message}`,
      ]));
    }
  }

  container.appendChild(keyValueTable([
    ['sample_id', record.sample_id],
    ['game_id', record.game_id],
    ['ply', record.ply],
    ['status', record.status],
    ['error_detail', record.error_detail],
    ['wall_time_ms', record.wall_time_ms],
    ['bestmove', record.bestmove],
    ['ponder', record.ponder],
  ]));

  if (record.engine) {
    container.appendChild(keyValueTable([
      ['engine.name', record.engine.name],
      ['engine.version', record.engine.version],
      ['engine.binary_sha256', shaCaption(record.engine.binary_sha256)],
      ['engine.weight_sha256', shaCaption(record.engine.weight_sha256)],
    ]));
  }

  if (record.settings) {
    container.appendChild(keyValueTable(Object.keys(record.settings).map((k) => [`settings.${k}`, record.settings[k]])));
  }

  if (Array.isArray(record.lines) && record.lines.length) {
    const headers = ['multipv', 'score', 'bound', 'candidate bestmove', 'pv', 'depth', 'nodes', 'time', 'nps'];
    const thead = el('thead', null, [el('tr', null, headers.map((h) => el('th', { scope: 'col' }, [h])))]);
    const tbody = el('tbody', null, record.lines.map(renderLineRow));
    container.appendChild(el('table', { class: 'lines-table' }, [thead, tbody]));
  }

  return container;
}

// ---- Viewer state + wiring ----

let currentRecords = [];
let currentProvenance = 'sample';

function renderSelectedRecord(idx) {
  const panel = document.getElementById('record-panel');
  clearChildren(panel);
  const item = currentRecords[idx];
  if (!item) {
    panel.appendChild(el('p', null, ['No record loaded yet.']));
    return;
  }
  try {
    panel.appendChild(renderRecordDetail(item.record, currentProvenance, item.warnings));
  } catch (e) {
    panel.appendChild(el('div', { class: 'error-banner', role: 'alert' }, [
      `This record could not be displayed: ${e.message}`,
    ]));
  }
}

function loadRecordsIntoViewer(rawRecords, opts) {
  currentProvenance = opts.provenance;
  currentRecords = rawRecords.map(normalizeRecord);
  const picker = document.getElementById('record-picker');
  clearChildren(picker);
  currentRecords.forEach((item, idx) => {
    const label = `${item.record.sample_id !== undefined ? item.record.sample_id : '(no sample_id)'} — ply ${item.record.ply !== undefined ? item.record.ply : '?'}`;
    picker.appendChild(el('option', { value: String(idx) }, [label]));
  });
  picker.hidden = currentRecords.length <= 1;
  renderSelectedRecord(0);
}

function handleParsedText(text, provenance) {
  const statusEl = document.getElementById('import-status');
  const { records, format, errors } = parseInput(text);
  if (records.length === 0) {
    statusEl.textContent = 'No valid JSON records found in this input.';
    return;
  }
  statusEl.textContent = `Loaded ${records.length} record(s) as ${format}` + (errors.length ? `; ${errors.length} line(s) failed to parse` : '');
  loadRecordsIntoViewer(records, { provenance });
}

function handleFile(file) {
  const reader = new FileReader();
  reader.onload = () => handleParsedText(String(reader.result), 'uploaded');
  reader.onerror = () => {
    document.getElementById('import-status').textContent = `Could not read file: ${reader.error}`;
  };
  reader.readAsText(file);
}

function loadSample(idx) {
  const sample = window.SEKIREI_EXPLORER_SAMPLES[idx];
  loadRecordsIntoViewer([sample.record], { provenance: 'sample' });
  document.getElementById('import-status').textContent = `Loaded sample: ${sample.meta.label}`;
}

function downloadSampleJsonl(idx) {
  const sample = window.SEKIREI_EXPLORER_SAMPLES[idx];
  const blob = new Blob([`${JSON.stringify(sample.record)}\n`], { type: 'application/jsonl' });
  const url = URL.createObjectURL(blob);
  const a = el('a', { href: url, download: `sekirei-explorer-sample-${idx + 1}.jsonl` }, []);
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

function initExplorer() {
  const fileInput = document.getElementById('file-input');
  const dropLabel = document.getElementById('file-drop-label');

  fileInput.addEventListener('change', () => {
    if (fileInput.files && fileInput.files[0]) handleFile(fileInput.files[0]);
  });

  dropLabel.addEventListener('dragover', (e) => {
    e.preventDefault();
    dropLabel.classList.add('drag-active');
  });
  ['dragleave', 'drop'].forEach((evt) => dropLabel.addEventListener(evt, () => dropLabel.classList.remove('drag-active')));
  dropLabel.addEventListener('drop', (e) => {
    e.preventDefault();
    const file = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
    if (file) handleFile(file);
  });

  document.querySelectorAll('[data-sample-load]').forEach((btn) => {
    btn.addEventListener('click', () => loadSample(Number(btn.dataset.sampleLoad)));
  });
  document.querySelectorAll('[data-sample-download]').forEach((btn) => {
    btn.addEventListener('click', () => downloadSampleJsonl(Number(btn.dataset.sampleDownload)));
  });

  document.getElementById('record-picker').addEventListener('change', (e) => {
    renderSelectedRecord(Number(e.target.value));
  });

  loadSample(0); // viewer isn't empty on arrival
}

// ---- Self-check (open index.html?selftest=1) ----

function runSelfChecks() {
  const results = [];
  function check(name, cond) {
    results.push({ name, pass: !!cond });
    console.assert(cond, `SELFTEST FAILED: ${name}`);
  }

  const startpos = 'lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1';
  const parsedStart = parseSfen(startpos);
  check('startpos: 81 cells', parsedStart.cells.length === 81);
  check('startpos: side to move is black', parsedStart.sideToMove === 'b');
  check('startpos: file9/ranka is white lance', (() => {
    const c = parsedStart.cells[0];
    return !!c && c.color === 'w' && c.kind === 'L';
  })());
  check('startpos: both hands empty', Object.keys(parsedStart.hands.b).length === 0 && Object.keys(parsedStart.hands.w).length === 0);

  // crates/sekirei-usi/src/invariant.rs:523-529 -- real game excerpt base position.
  const gameSfen = 'l4gknl/1r2g1sb1/n1pspppp1/pp1p4p/6PP1/P1PS1S3/1P1PPP2P/1B5R1/LN1GKG1NL w - 24';
  const parsedGame = parseSfen(gameSfen);
  check('game excerpt: side to move is white', parsedGame.sideToMove === 'w');
  check('game excerpt: ply parses to 24', parsedGame.ply === 24);

  // crates/sekirei-core/src/search.rs:2189 -- MATE_IN_1_SFEN.
  const parsedMate = parseSfen('k8/2K6/9/9/4R4/9/9/9/9 b - 1');
  check('mate-in-1: white king at file9/ranka', (() => {
    const c = parsedMate.cells[0];
    return !!c && c.color === 'w' && c.kind === 'K';
  })());

  check('parseUsiToken: normal move', (() => {
    const t = parseUsiToken('7g7f');
    return t.type === 'move' && t.from === '7g' && t.to === '7f' && t.promote === false;
  })());
  check('parseUsiToken: promotion', parseUsiToken('8h2b+').promote === true);
  check('parseUsiToken: drop', (() => {
    const t = parseUsiToken('P*3d');
    return t.type === 'drop' && t.piece === 'P' && t.to === '3d';
  })());

  check('parseInput: single object', parseInput('{"a":1}').format === 'single');
  check('parseInput: array', parseInput('[{"a":1},{"a":2}]').format === 'array');
  check('parseInput: jsonl', parseInput('{"a":1}\n{"a":2}').format === 'jsonl');

  const failed = results.filter((r) => !r.pass);
  console.log(`Sekirei Explorer self-check: ${results.length - failed.length}/${results.length} passed`);
  if (failed.length) console.error('Failed checks:', failed.map((f) => f.name));
  return failed.length === 0;
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', initExplorer);
} else {
  initExplorer();
}

if (new URLSearchParams(location.search).has('selftest')) {
  document.addEventListener('DOMContentLoaded', () => {
    const pass = runSelfChecks();
    document.title = `${pass ? 'PASS' : 'FAIL'} - Sekirei Explorer selftest`;
  });
}
