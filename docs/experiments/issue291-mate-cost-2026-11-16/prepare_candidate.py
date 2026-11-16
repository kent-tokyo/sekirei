"""Make isolated C01 variants from a frozen, reviewed shared-core patch."""
from pathlib import Path
import subprocess
from prepare_phased import COUNTER, ROOT, HERE, SOURCE, OLD, replace_once

PATCH = HERE / 'c01.patch'
PLAIN = Path('/private/tmp/sekirei-issue291-c01')
COUNTED = Path('/private/tmp/sekirei-issue291-c01-counted')
PROOF = ROOT / 'docs/experiments/jsa_tsume_2026-11-16/proof_driver.rs'
D01_DRIVER = Path('/private/tmp/sekirei-issue291-d01/crates/sekirei-core/examples/jsa_search.rs')

def unpack(dest):
    assert not dest.exists(), f'refuse overwrite: {dest}'
    dest.mkdir()
    archive = subprocess.check_output(['git','archive',SOURCE,'Cargo.toml','Cargo.lock','crates'],cwd=ROOT)
    subprocess.run(['tar','-xf','-','-C',str(dest)],input=archive,check=True)
    subprocess.run(['git','apply','--check',str(PATCH)],cwd=dest,check=True)
    subprocess.run(['git','apply',str(PATCH)],cwd=dest,check=True)
    ex = dest/'crates/sekirei-core/examples'
    ex.mkdir(exist_ok=True)
    return ex

def main():
    assert PATCH.is_file() and PATCH.stat().st_size > 0
    assert D01_DRIVER.is_file()
    plain_ex=unpack(PLAIN)
    (plain_ex/'jsa_search.rs').write_bytes(OLD.read_bytes())
    (plain_ex/'jsa_proof.rs').write_bytes(PROOF.read_bytes())
    counted_ex=unpack(COUNTED)
    board=COUNTED/'crates/sekirei-core/src/board.rs'
    board_text=board.read_text()
    assert 'use crate::zobrist;\n' in board_text
    board_text=board_text.replace('use crate::zobrist;\n','use crate::zobrist;\n'+COUNTER,1)
    board_text=replace_once(board_text,
        '    ) -> MoveToken {\n        self.invalidate_legality_cache();',
        '    ) -> MoveToken {\n        let phase = JSA_PHASE.with(std::cell::Cell::get);\n        JSA_COUNTS.with(|c| c.borrow_mut()[phase] += 1);\n        self.invalidate_legality_cache();')
    board.write_text(board_text)
    mate=COUNTED/'crates/sekirei-core/src/mate.rs'
    text=mate.read_text()
    text=replace_once(text,
        '                let tok = board.do_move_for_search(m);\n                let check = is_in_check(board, board.side_to_move);',
        '                let tok = crate::board::jsa_with_phase(1, || board.do_move_for_search(m));\n                let check = is_in_check(board, board.side_to_move);')
    text=replace_once(text,
        '            let tok = board.do_move_for_search(moves[best]);\n            self.mid(board, child_pn, child_dn, !attacker, ply + 1);',
        '            let tok = crate::board::jsa_with_phase(3, || board.do_move_for_search(moves[best]));\n            self.mid(board, child_pn, child_dn, !attacker, ply + 1);')
    mate.write_text(text)
    (counted_ex/'jsa_search.rs').write_bytes(D01_DRIVER.read_bytes())
    print('prepared',PLAIN,COUNTED)

if __name__=='__main__': main()
