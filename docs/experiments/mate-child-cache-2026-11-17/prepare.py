"""Build fixed baseline, shadow measurement or single cache candidate in /tmp."""
import argparse
import difflib
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUT = ROOT / 'data/runs/mate-child-cache-20261117'
BASE = Path('/private/tmp/sekirei-child-cache-20261117-base')

COUNTERS = '''
std::thread_local! {
    static KC_COUNTS: std::cell::Cell<[u64; 4]> = const { std::cell::Cell::new([0; 4]) };
}
/// Diagnostic-only counters: complete moves, cache hits, hit edges, calls.
pub fn kc_counts() -> [u64; 4] { KC_COUNTS.with(std::cell::Cell::get) }
/// Reset diagnostic-only counters.
pub fn kc_reset() { KC_COUNTS.with(|c| c.set([0; 4])); }
pub(crate) fn kc_add(index: usize, amount: u64) {
    KC_COUNTS.with(|c| { let mut a = c.get(); a[index] += amount; c.set(a); });
}
'''


def once(text, old, new):
    assert text.count(old) == 1, (old, text.count(old))
    return text.replace(old, new, 1)


def prepare(mode):
    dest = BASE if mode == 'base' else Path('/private/tmp/sekirei-child-cache-20261117-' + mode)
    if mode != 'base':
        assert not dest.exists(), dest
        shutil.copytree(BASE, dest, ignore=shutil.ignore_patterns('target'))
    board_path = dest / 'crates/sekirei-core/src/board.rs'
    mate_path = dest / 'crates/sekirei-core/src/mate.rs'
    board_before, mate_before = board_path.read_text(), mate_path.read_text()
    board, mate = board_before, mate_before
    if mode != 'base':
        board = once(board, 'mod quiet_halfkp;', '''pub(crate) type PositionIdentity = ([Option<Piece>; Square::NUM], [u32; 2], Color);

mod quiet_halfkp;''')
        board = once(board, 'impl Board {', '''impl Board {
    pub(crate) fn position_identity(&self) -> PositionIdentity {
        (self.mailbox, [self.hand[0].packed(), self.hand[1].packed()], self.side_to_move)
    }
''')
        mate = once(mate, 'struct Solver {', (HERE / 'cache.inc.rs').read_text() + '\nstruct Solver {\n    child_cache: ChildCache,')
        mate = mate.replace('table: MateMap::default(),', 'table: MateMap::default(),\n        child_cache: ChildCache::default(),')
        # The cache struct itself uses entries, so only Solver constructors match.
        if mode == 'shadow':
            needle = '            .collect();\n        self.path.push(hash);'
            replacement = '''            .collect();
        crate::board::kc_add(3, 1);
        if let Some(hit) = self.child_cache.get(board, attacker) {
            assert_eq!(hit.moves, moves);
            assert_eq!(hit.hashes, child_hashes);
            crate::board::kc_add(1, 1);
            crate::board::kc_add(2, child_hashes.len() as u64);
        } else {
            self.child_cache.insert(board, attacker, std::rc::Rc::new(ChildList {
                moves: moves.clone(), hashes: child_hashes.clone(),
            }));
        }
        self.path.push(hash);'''
            mate = once(mate, needle, replacement)
        else:
            mate = once(mate, '        let moves = self.children(board, attacker);\n        if moves.is_empty()', '''        let cached = self.child_cache.get(board, attacker);
        let fresh_moves = cached.is_none().then(|| self.children(board, attacker));
        let moves = cached.as_ref().map_or_else(
            || fresh_moves.as_ref().unwrap().as_slice(),
            |list| list.moves.as_slice(),
        );
        if moves.is_empty()''')
            old = '''        let child_hashes: Vec<u64> = moves
            .iter()
            .map(|&m| {
                let tok = board.do_move_for_search(m);
                let h = board.hash();
                board.undo_move_for_search(tok);
                h
            })
            .collect();'''
            new = '''        let list = if let Some(cached) = cached {
            cached
        } else {
            let moves = fresh_moves.unwrap();
            let hashes = moves.iter().map(|&m| {
                let tok = board.do_move_for_search(m);
                let h = board.hash();
                board.undo_move_for_search(tok);
                h
            }).collect();
            let list = std::rc::Rc::new(ChildList { moves, hashes });
            self.child_cache.insert(board, attacker, std::rc::Rc::clone(&list));
            list
        };
        let moves = &list.moves;
        let child_hashes = &list.hashes;'''
            mate = once(mate, old, new)
            tests = (HERE / 'cache_tests.inc.rs').read_text().replace('&sfen.trim_end_matches(\'1\').to_owned().add("99")', '&format!("{}99", sfen.trim_end_matches(\'1\'))')
            mate = once(mate, '    #[test]\n    fn mate_hasher_byte_writes_preserve_stream_order()', tests + '    #[test]\n    fn mate_hasher_byte_writes_preserve_stream_order()')
    if mode in ('shadow', 'counted'):
        board = once(board, '\nuse crate::zobrist;\n', '\nuse crate::zobrist;\n' + COUNTERS)
        board = once(board, '    ) -> MoveToken {\n        self.invalidate_legality_cache();', '    ) -> MoveToken {\n        kc_add(0, 1);\n        self.invalidate_legality_cache();')
        if mode == 'counted':
            mate = once(mate, '        Some(std::rc::Rc::clone(&entry.list))', '        crate::board::kc_add(1, 1);\n        crate::board::kc_add(2, entry.list.moves.len() as u64);\n        Some(std::rc::Rc::clone(&entry.list))')
            mate = once(mate, '        let cached = self.child_cache.get(board, attacker);', '        crate::board::kc_add(3, 1);\n        let cached = self.child_cache.get(board, attacker);')
        driver_path = dest / 'crates/sekirei-core/examples/jsa_search.rs'
        driver = driver_path.read_text()
        driver = once(driver, '    let start = Instant::now();', '    sekirei_core::board::kc_reset();\n    let start = Instant::now();')
        driver = once(driver, '    let elapsed = start.elapsed();', '    let elapsed = start.elapsed();\n    let counts = sekirei_core::board::kc_counts();')
        driver = once(driver, 'elapsed_ns={} within_deadline={}', 'elapsed_ns={} within_deadline={} make_moves={} cache_hits={} hit_edges={} cache_calls={}')
        driver = once(driver, '        elapsed <= Duration::from_secs(8)', '        elapsed <= Duration::from_secs(8),\n        counts[0], counts[1], counts[2], counts[3]')
        driver_path.write_text(driver)
    board_path.write_text(board)
    mate_path.write_text(mate)
    if mode != 'base':
        subprocess.run(['rustfmt', '--edition', '2024', str(board_path), str(mate_path)], check=True)
    patch = ''
    for name, before, path in [('board.rs', board_before, board_path), ('mate.rs', mate_before, mate_path)]:
        patch += ''.join(difflib.unified_diff(before.splitlines(True), path.read_text().splitlines(True), fromfile='a/crates/sekirei-core/src/' + name, tofile='b/crates/sekirei-core/src/' + name))
    (OUT / (mode + '.patch')).write_text(patch)
    with (OUT / (mode + '-build.log')).open('w') as log:
        subprocess.run(['cargo', 'build', '--release', '--offline', '-p', 'sekirei-core', '--example', 'jsa_search', '--example', 'jsa_proof', '--example', 'jsa_check', '-j', '1'], cwd=dest, stdout=log, stderr=subprocess.STDOUT, timeout=300, check=True)
    print('Built', mode, dest, flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['base', 'shadow', 'candidate', 'counted'])
    prepare(parser.parse_args().mode)
