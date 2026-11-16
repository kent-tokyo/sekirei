"""Build a phase-counted copy of the frozen mate solver (never edit shared core)."""
from pathlib import Path
import difflib
import subprocess

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
DEST = Path('/private/tmp/sekirei-issue291-d01')
SOURCE = '14586edaf3a5e9e03739c2fa39c9e3f48e2cd56c'
OLD = ROOT / 'docs/experiments/jsa_tsume_2026-11-16/komoring_comparison/search_driver.rs'

COUNTER = '''
// Diagnostic-only phase counter. 0=unclassified, 1=check, 2=child hash,
// 3=descent, 4=answer. Count every complete do_move_impl call exactly once.
std::thread_local! {
    static JSA_PHASE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static JSA_COUNTS: std::cell::RefCell<[u64; 5]> = const { std::cell::RefCell::new([0; 5]) };
}
pub fn jsa_reset_counts() {
    JSA_PHASE.with(|p| p.set(0));
    JSA_COUNTS.with(|c| *c.borrow_mut() = [0; 5]);
}
pub fn jsa_counts() -> [u64; 5] { JSA_COUNTS.with(|c| *c.borrow()) }
pub fn jsa_with_phase<T>(phase: usize, f: impl FnOnce() -> T) -> T {
    JSA_PHASE.with(|p| {
        let prior = p.replace(phase);
        let result = f();
        p.set(prior);
        result
    })
}
'''

def replace_once(text, old, new):
    assert text.count(old) == 1, (old, text.count(old))
    return text.replace(old, new, 1)

def main():
    assert not DEST.exists(), 'Refuse to overwrite isolated build'
    DEST.mkdir()
    archive = subprocess.check_output(['git', 'archive', SOURCE, 'Cargo.toml', 'Cargo.lock', 'crates'], cwd=ROOT)
    subprocess.run(['tar', '-xf', '-', '-C', str(DEST)], input=archive, check=True)
    board = DEST / 'crates/sekirei-core/src/board.rs'
    original_board = board.read_text()
    assert 'use crate::zobrist;\n' in original_board
    updated_board = original_board.replace('use crate::zobrist;\n', 'use crate::zobrist;\n' + COUNTER, 1)
    updated_board = replace_once(updated_board,
        '    ) -> MoveToken {\n        self.invalidate_legality_cache();',
        '    ) -> MoveToken {\n        let phase = JSA_PHASE.with(std::cell::Cell::get);\n        JSA_COUNTS.with(|c| c.borrow_mut()[phase] += 1);\n        self.invalidate_legality_cache();')
    board.write_text(updated_board)
    mate = DEST / 'crates/sekirei-core/src/mate.rs'
    original_mate = mate.read_text()
    updated_mate = original_mate
    updated_mate = replace_once(updated_mate,
        '            let tok = root.do_move_for_search(m);\n            let proven = solver.lookup(root.hash(), 1).0 == 0;',
        '            let tok = crate::board::jsa_with_phase(4, || root.do_move_for_search(m));\n            let proven = solver.lookup(root.hash(), 1).0 == 0;')
    updated_mate = replace_once(updated_mate,
        '                let tok = board.do_move_for_search(m);\n                let check = is_in_check(board, board.side_to_move);',
        '                let tok = crate::board::jsa_with_phase(1, || board.do_move_for_search(m));\n                let check = is_in_check(board, board.side_to_move);')
    updated_mate = replace_once(updated_mate,
        '                let tok = board.do_move_for_search(m);\n                let h = board.hash();',
        '                let tok = crate::board::jsa_with_phase(2, || board.do_move_for_search(m));\n                let h = board.hash();')
    updated_mate = replace_once(updated_mate,
        '            let tok = board.do_move_for_search(moves[best]);\n            self.mid(board, child_pn, child_dn, !attacker, ply + 1);',
        '            let tok = crate::board::jsa_with_phase(3, || board.do_move_for_search(moves[best]));\n            self.mid(board, child_pn, child_dn, !attacker, ply + 1);')
    mate.write_text(updated_mate)
    def diff(path, old, new):
        name = 'crates/sekirei-core/src/' + path
        return ''.join(difflib.unified_diff(old.splitlines(True), new.splitlines(True), fromfile='a/'+name, tofile='b/'+name))
    (HERE / 'd01-instrumentation.patch').write_text(diff('board.rs', original_board, updated_board) + diff('mate.rs', original_mate, updated_mate))
    driver = OLD.read_text()
    driver = replace_once(driver, '    let start = Instant::now();', '    sekirei_core::board::jsa_reset_counts();\n    let start = Instant::now();')
    driver = replace_once(driver, '    let elapsed = start.elapsed();', '    let elapsed = start.elapsed();\n    let counts = sekirei_core::board::jsa_counts();')
    driver = replace_once(driver,
        'elapsed_ns={} within_deadline={}',
        'elapsed_ns={} within_deadline={} phase_unclassified={} phase_check={} phase_hash={} phase_descent={} phase_answer={} make_moves={}')
    driver = replace_once(driver,
        '        elapsed <= Duration::from_secs(8)',
        '        elapsed <= Duration::from_secs(8),\n        counts[0], counts[1], counts[2], counts[3], counts[4], counts.iter().sum::<u64>()')
    examples = DEST / 'crates/sekirei-core/examples'
    examples.mkdir(exist_ok=True)
    (examples / 'jsa_search.rs').write_text(driver)
    print(DEST)

if __name__ == '__main__':
    main()
