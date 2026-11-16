"""One new F01 layout change and separate diagnostic counters."""
import argparse
import difflib
from pathlib import Path
import shutil
import subprocess
from prepare import BASE, OUT, HERE, COUNTERS, once


def main(mode):
    dest=Path('/private/tmp/sekirei-child-cache-20261117-'+mode)
    assert not dest.exists()
    shutil.copytree(BASE,dest,ignore=shutil.ignore_patterns('target'))
    mp=dest/'crates/sekirei-core/src/mate.rs';bp=dest/'crates/sekirei-core/src/board.rs'
    before=mp.read_text();mate=before;bb=bp.read_text();board=bb
    candidate=mode!='facts-base-counted'
    if candidate:
        mate=once(mate,'struct Solver {','''#[derive(Default)]
struct PositionFacts {
    terminal: Option<(u64, u64)>,
    bounds: SolvedBounds,
}

struct Solver {''')
        mate=once(mate,'    solved: MateMap<u64, SolvedBounds>,\n    terminals: MateMap<u64, (u64, u64)>,','    facts: MateMap<u64, PositionFacts>,')
        # All constructor initializers use default values; preserve every other field.
        import re
        mate=re.sub(r'(?m)^(\s*)terminals: MateMap::default\(\),\n\s*solved: MateMap::default\(\),',r'\1facts: MateMap::default(),',mate)
        mate=re.sub(r'(?m)^(\s*)solved: MateMap::default\(\),\n\s*terminals: MateMap::default\(\),',r'\1facts: MateMap::default(),',mate)
        mate=once(mate,'        if let Some(&(pn, dn)) = self.terminals.get(&hash) {','        let facts = self.facts.get(&hash);\n        if let Some((pn, dn)) = facts.and_then(|entry| entry.terminal) {')
        mate=once(mate,'        let bounds = self.solved.get(&hash);','        let bounds = facts.map(|entry| &entry.bounds);')
        mate=once(mate,'        let entry = self.solved.entry(hash).or_default();','        let entry = &mut self.facts.entry(hash).or_default().bounds;')
        mate=once(mate,'            self.terminals.insert(hash, value);','            self.facts.entry(hash).or_default().terminal = Some(value);')
        mate=mate.replace('solver.solved.contains_key','solver.facts.contains_key')
        mate=mate.replace('solver.solved.get(&21).unwrap().conflicted','solver.facts.get(&21).unwrap().bounds.conflicted')
        tests='''    #[test]
    fn shared_facts_keep_terminal_priority_and_both_fields() {
        let mut solver = bounds_test_solver(7);
        solver.save_result(123, 2, 0, INF, false);
        let bound = solver.facts.get(&123).unwrap().bounds.proof_min_remaining;
        solver.facts.entry(123).or_default().terminal = Some((INF, 0));
        assert_eq!(solver.lookup_with_dependency(123, 2), (INF, 0, false));
        assert_eq!(solver.facts.get(&123).unwrap().bounds.proof_min_remaining, bound);
        solver.save_result(123, 2, INF, 0, false);
        assert!(solver.facts.get(&123).unwrap().bounds.conflicted);
        assert_eq!(solver.lookup_with_dependency(123, 2), (INF, 0, false));
    }

'''
        mate=once(mate,'    #[test]\n    fn mate_hasher_byte_writes_preserve_stream_order()',tests+'    #[test]\n    fn mate_hasher_byte_writes_preserve_stream_order()')
        assert 'self.terminals' not in mate and 'self.solved' not in mate
    if 'counted' in mode:
        board=once(board,'\nuse crate::zobrist;\n','\nuse crate::zobrist;\n'+COUNTERS)
        board=once(board,'    ) -> MoveToken {\n        self.invalidate_legality_cache();','    ) -> MoveToken {\n        kc_add(0, 1);\n        self.invalidate_legality_cache();')
        marker='    fn lookup_with_dependency(&self, hash: u64, ply: u32) -> (u64, u64, bool) {'
        mate=once(mate,marker,marker+'\n        crate::board::kc_add(1, 1);\n        crate::board::kc_add(2, 1);')
        if not candidate:
            mate=once(mate,'        let bounds = self.solved.get(&hash);','        crate::board::kc_add(2, 1);\n        let bounds = self.solved.get(&hash);')
        mate=once(mate,'        let (pn, dn, path_dependent) = self\n            .table','        crate::board::kc_add(2, 1);\n        let (pn, dn, path_dependent) = self\n            .table')
        dp=dest/'crates/sekirei-core/examples/jsa_search.rs';d=dp.read_text()
        d=once(d,'    let start = Instant::now();','    sekirei_core::board::kc_reset();\n    let start = Instant::now();')
        d=once(d,'    let elapsed = start.elapsed();','    let elapsed = start.elapsed();\n    let counts = sekirei_core::board::kc_counts();')
        d=once(d,'elapsed_ns={} within_deadline={}','elapsed_ns={} within_deadline={} make_moves={} lookup_calls={} map_reads={}')
        d=once(d,'        elapsed <= Duration::from_secs(8)','        elapsed <= Duration::from_secs(8),\n        counts[0], counts[1], counts[2]')
        dp.write_text(d)
    mp.write_text(mate);bp.write_text(board)
    subprocess.run(['rustfmt','--edition','2024',str(mp),str(bp)],check=True)
    patch=''
    for name,old,p in [('mate.rs',before,mp),('board.rs',bb,bp)]:
        patch+=''.join(difflib.unified_diff(old.splitlines(True),p.read_text().splitlines(True),fromfile='a/crates/sekirei-core/src/'+name,tofile='b/crates/sekirei-core/src/'+name))
    (OUT/(mode+'.patch')).write_text(patch)
    with (OUT/(mode+'-build.log')).open('w') as log:
        subprocess.run(['cargo','build','--release','--offline','-p','sekirei-core','--example','jsa_search','--example','jsa_proof','-j','1'],cwd=dest,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=300)
    if mode=='facts':
        with (OUT/'facts-tests.log').open('w') as log:
            for args in [['cargo','test','--release','--offline','-p','sekirei-core','--lib','mate::','-j','1','--','--test-threads=1'],
                         ['cargo','test','--release','--offline','-p','sekirei-core','--lib','existence_proof_handles_fourth_public_diagram_within_fixed_budget','-j','1','--','--ignored','--test-threads=1']]:
                subprocess.run(args,cwd=dest,stdout=log,stderr=subprocess.STDOUT,check=True,timeout=300)
    print('Built and checked',mode,flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('mode',choices=['facts','facts-counted','facts-base-counted'])
    main(p.parse_args().mode)
