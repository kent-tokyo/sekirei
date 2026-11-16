"""Make the frozen O01 ordering-only variants from the pinned source."""
from pathlib import Path
import difflib
import subprocess
from prepare_phased import COUNTER, ROOT, HERE, SOURCE, OLD, replace_once

PLAIN = Path('/private/tmp/sekirei-issue291-o01')
COUNTED = Path('/private/tmp/sekirei-issue291-o01-counted')
PROOF = ROOT / 'docs/experiments/jsa_tsume_2026-11-16/proof_driver.rs'
D01_DRIVER = Path('/private/tmp/sekirei-issue291-d01/crates/sekirei-core/examples/jsa_search.rs')
PATCH = HERE / 'o01.patch'

OLD_SELECTION = '''        self.path.push(hash);
        loop {
'''
NEW_SELECTION = '''        // O01: break finite attacker pn ties by distance to the defending king.
        // Keep move generation order for all other ties and every defender node.
        let defender_king = if attacker {
            board.king_square(board.side_to_move.flip())
        } else {
            None
        };
        self.path.push(hash);
        loop {
'''
OLD_TIE = '''                if selected < min {
                    second = min;
                    min = selected;
                    best = i;
                } else if selected < second {
                    second = selected;
                }
'''
NEW_TIE = '''                if selected < min {
                    second = min;
                    min = selected;
                    best = i;
                } else {
                    if selected == min && selected < INF {
                        if let Some(king) = defender_king {
                            let distance = |index: usize| {
                                moves[index].to.file_0().abs_diff(king.file_0())
                                    .max(moves[index].to.rank_0().abs_diff(king.rank_0()))
                            };
                            if distance(i) < distance(best) {
                                best = i;
                            }
                        }
                    }
                    if selected < second {
                        second = selected;
                    }
                }
'''

def unpack(dest):
    assert not dest.exists(), f'refuse overwrite: {dest}'
    dest.mkdir()
    archive = subprocess.check_output(['git', 'archive', SOURCE, 'Cargo.toml', 'Cargo.lock', 'crates'], cwd=ROOT)
    subprocess.run(['tar', '-xf', '-', '-C', str(dest)], input=archive, check=True)
    examples = dest / 'crates/sekirei-core/examples'
    examples.mkdir(exist_ok=True)
    return examples

def main():
    assert not PATCH.exists(), 'refuse overwrite frozen patch'
    assert D01_DRIVER.is_file()
    plain_ex = unpack(PLAIN)
    mate = PLAIN / 'crates/sekirei-core/src/mate.rs'
    before = mate.read_text()
    after = replace_once(before, OLD_SELECTION, NEW_SELECTION)
    after = replace_once(after, OLD_TIE, NEW_TIE)
    mate.write_text(after)
    name = 'crates/sekirei-core/src/mate.rs'
    PATCH.write_text(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True), fromfile='a/'+name, tofile='b/'+name)))
    (plain_ex/'jsa_search.rs').write_bytes(OLD.read_bytes())
    (plain_ex/'jsa_proof.rs').write_bytes(PROOF.read_bytes())

    counted_ex = unpack(COUNTED)
    subprocess.run(['git','apply','--check',str(PATCH)],cwd=COUNTED,check=True)
    subprocess.run(['git','apply',str(PATCH)],cwd=COUNTED,check=True)
    board = COUNTED / 'crates/sekirei-core/src/board.rs'
    board_text = board.read_text()
    assert 'use crate::zobrist;\n' in board_text
    board_text = board_text.replace('use crate::zobrist;\n', 'use crate::zobrist;\n'+COUNTER, 1)
    board_text = replace_once(board_text,
        '    ) -> MoveToken {\n        self.invalidate_legality_cache();',
        '    ) -> MoveToken {\n        let phase = JSA_PHASE.with(std::cell::Cell::get);\n        JSA_COUNTS.with(|c| c.borrow_mut()[phase] += 1);\n        self.invalidate_legality_cache();')
    board.write_text(board_text)
    counted_mate = COUNTED / 'crates/sekirei-core/src/mate.rs'
    text = counted_mate.read_text()
    for old, new in [
        ('            let tok = root.do_move_for_search(m);\n            let proven = solver.lookup(root.hash(), 1).0 == 0;',
         '            let tok = crate::board::jsa_with_phase(4, || root.do_move_for_search(m));\n            let proven = solver.lookup(root.hash(), 1).0 == 0;'),
        ('                let tok = board.do_move_for_search(m);\n                let check = is_in_check(board, board.side_to_move);',
         '                let tok = crate::board::jsa_with_phase(1, || board.do_move_for_search(m));\n                let check = is_in_check(board, board.side_to_move);'),
        ('                let tok = board.do_move_for_search(m);\n                let h = board.hash();',
         '                let tok = crate::board::jsa_with_phase(2, || board.do_move_for_search(m));\n                let h = board.hash();'),
        ('            let tok = board.do_move_for_search(moves[best]);\n            self.mid(board, child_pn, child_dn, !attacker, ply + 1);',
         '            let tok = crate::board::jsa_with_phase(3, || board.do_move_for_search(moves[best]));\n            self.mid(board, child_pn, child_dn, !attacker, ply + 1);'),
    ]:
        text = replace_once(text, old, new)
    counted_mate.write_text(text)
    (counted_ex/'jsa_search.rs').write_bytes(D01_DRIVER.read_bytes())
    print('prepared',PLAIN,COUNTED)

if __name__=='__main__': main()
