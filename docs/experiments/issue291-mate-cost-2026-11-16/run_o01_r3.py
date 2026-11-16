"""One registered O01-R3 comparison after R2 monitor permission failure."""
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import time

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[2]
INPUT=ROOT/'data/runs/jsa-tsume-20261116/corpus.json'
OLD=ROOT/'data/runs/jsa-tsume-20261116/receipt.json'
COMPARE=ROOT/'docs/experiments/jsa_tsume_2026-11-16/komoring_comparison/run_comparison.py'
BASE=Path('/private/tmp/sekirei-jsa-20261116/target/release/examples/jsa_search')
CAND=Path('/private/tmp/sekirei-issue291-o01-r2/target/release/examples/jsa_search')
COUNTED=Path('/private/tmp/sekirei-issue291-o01-r2-counted/target/release/examples/jsa_search')
PROOF=Path('/private/tmp/sekirei-issue291-o01-r2/target/release/examples/jsa_proof')
RAW=ROOT/'data/runs/issue291-o01-r3-20261116'
REG=HERE/'o01-r3-registration.json'
RECEIPT=HERE/'o01-r3-receipt.json'
SUMMARY=HERE/'o01-r3-summary.json'

def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def save(p,d): p.write_text(json.dumps(d,ensure_ascii=False,indent=2)+'\n')

def main():
    for p in (RAW,REG,RECEIPT,SUMMARY): assert not p.exists(),f'refuse overwrite: {p}'
    assert sha(INPUT)=='0e5c60502aadd4b07acd9b726324851cd8b1f994f0ef9e152e3140db23a6e474'
    for p in (BASE,CAND,COUNTED,PROOF,HERE/'o01-r2.patch',HERE/'d01-summary.json'): assert p.is_file(),p
    reg=dict(id='291-O01-R3-20261116',source_commit='14586edaf3a5e9e03739c2fa39c9e3f48e2cd56c',
             candidate_patch_sha256=sha(HERE/'o01-r2.patch'),prepare_sha256=sha(HERE/'prepare_o01.py'),
             prepare_r2_sha256=sha(HERE/'prepare_o01_r2.py'),
             runner_sha256=sha(Path(__file__)),reused_runner_sha256=sha(COMPARE),
             binaries_sha256={name:sha(p) for name,p in [('base',BASE),('candidate',CAND),('candidate_counted',COUNTED),('candidate_proof',PROOF)]},
             corpus_sha256=sha(INPUT),d01_summary_sha256=sha(HERE/'d01-summary.json'),
             old_proof_receipt_sha256=sha(OLD),
             rule='Six new verified proof trees; total full moves improve >=10%, no case moves rise >25%; median-sum time improves >=10%, no per-case time regression >max(10%,0.1ms), RSS rise <=max(10%,1MiB).',
             limits=dict(threads=1,search_nodes=999999,max_ply=25,solver_deadline_s=8,
                         external_deadline_s=12,rss_bytes=2*1024**3,output_bytes_per_run=2*1024**2,
                         counted_runs=18,timing_runs=60,proof_runs=6,total_runs=84,total_seconds=1200,
                         no_extension=True,stop='first error or cap; preserve receipts'))
    save(REG,reg)
    RAW.mkdir()
    spec=importlib.util.spec_from_file_location('jsa_comparison',COMPARE)
    mod=importlib.util.module_from_spec(spec); spec.loader.exec_module(mod)
    mod.OUT=RAW
    mod.BINARIES.update(sekirei_baseline=BASE,sekirei_o01=CAND,sekirei_o01_counted=COUNTED)
    cases=json.loads(INPUT.read_text())['cases']
    d01={r['id']:r for r in json.loads((HERE/'d01-summary.json').read_text())['cases']}
    rows=[]; proofs=[]; start=time.monotonic()
    def check_cap():
        if time.monotonic()-start>=1200: raise RuntimeError('total wall cap')
        if len(rows)+len(proofs)>=84: raise RuntimeError('run count cap')
    def record(row):
        rows.append(row); save(RECEIPT,dict(registration_sha256=sha(REG),rows=rows,proofs=proofs))
        if row['status']!='complete': raise RuntimeError(f"failed {row['id']} {row['engine']}: {row.get('error')}")
    # Counted candidate; no timing claim from this binary.
    for case in cases:
        prior=d01[case['id']]
        for repeat in (1,2,3):
            check_cap(); row=mod.run_one('sekirei_o01_counted',case,repeat)
            record(row)
            fields=dict(x.split('=',1) for x in row['answer'].split()[1:])
            phases={k:int(fields['phase_'+k]) for k in ('unclassified','check','hash','descent','answer')}
            assert phases['unclassified']==0
            assert row['make_moves']==sum(phases.values())
            row['phases']=phases; save(RECEIPT,dict(registration_sha256=sha(REG),rows=rows,proofs=proofs))
    # Pair uninstrumented binaries, alternating execution order by repeat and case.
    for case_i,case in enumerate(cases):
        for repeat in (1,2,3,4,5):
            order=('sekirei_baseline','sekirei_o01') if (case_i+repeat)%2 else ('sekirei_o01','sekirei_baseline')
            for engine in order:
                check_cap(); row=mod.run_one(engine,case,repeat); record(row)
                if engine=='sekirei_baseline':
                    assert row['first']==d01[case['id']]['first'] and row['nodes']==d01[case['id']]['nodes']
                else:
                    counted_rows=[x for x in rows if x['id']==case['id'] and x['engine']=='sekirei_o01_counted']
                    assert row['first']==counted_rows[0]['first'] and row['nodes']==counted_rows[0]['nodes']
    # Candidate proof extraction/replay is separate from answer-time measurement.
    old={x['id']:x for x in json.loads(OLD.read_text())['cases']}
    for case in cases:
        check_cap()
        tree=RAW/(case['id']+'-o01-tree.json')
        s=mod.Session([PROOF,case['sfen'],tree]); row=dict(id=case['id'],status='complete')
        try:
            s.until('reverified=',seconds=12)
            row['returncode']=s.finish()
            row['stdout']=s.text['stdout']; row['stderr']=s.text['stderr']
            rss=re.search(r'(\d+)\s+maximum resident set size',row['stderr'])
            row['peak_rss_bytes']=int(rss[1]) if rss else None
            assert row['returncode']==0 and row['peak_rss_bytes'] is not None and row['peak_rss_bytes']<=2*1024**3
            assert 'verified=true invalid=None stop=None' in row['stdout'] and 'reverified=true' in row['stdout']
            assert tree.is_file() and tree.stat().st_size<=2*1024**2
            row['tree_sha256']=sha(tree)
            row['baseline_tree_sha256']=old[case['id']]['files_sha256'][case['id']+'-tree.json']
        except Exception as exc:
            row.update(status='failed',error=repr(exc)); s.kill()
        finally:
            row['process_reaped']=s.p.poll() is not None
            s.close(); proofs.append(row)
            save(RECEIPT,dict(registration_sha256=sha(REG),rows=rows,proofs=proofs))
        if row['status']!='complete': raise RuntimeError('proof failed '+case['id']+': '+row['error'])
    import statistics
    table=[]
    for case in cases:
        x=[r for r in rows if r['id']==case['id'] and r['engine']=='sekirei_baseline']
        y=[r for r in rows if r['id']==case['id'] and r['engine']=='sekirei_o01']
        assert len(x)==len(y)==5
        table.append(dict(id=case['id'],baseline_ms=statistics.median(r['native_elapsed_ns'] for r in x)/1e6,
                          candidate_ms=statistics.median(r['native_elapsed_ns'] for r in y)/1e6,
                          baseline_range_ms=[min(r['native_elapsed_ns'] for r in x)/1e6,max(r['native_elapsed_ns'] for r in x)/1e6],
                          candidate_range_ms=[min(r['native_elapsed_ns'] for r in y)/1e6,max(r['native_elapsed_ns'] for r in y)/1e6],
                          baseline_rss_bytes=max(r['peak_rss_bytes'] for r in x),
                          candidate_rss_bytes=max(r['peak_rss_bytes'] for r in y),
                          original_moves=d01[case['id']]['make_moves'],
                          candidate_moves=next(r['make_moves'] for r in rows if r['id']==case['id'] and r['engine']=='sekirei_o01_counted')))
    base_sum=sum(r['baseline_ms'] for r in table)
    candidate_sum=sum(r['candidate_ms'] for r in table)
    base_moves=sum(r['original_moves'] for r in table)
    candidate_moves=sum(r['candidate_moves'] for r in table)
    moves_pass=candidate_moves<=base_moves*0.9
    move_regressions=[r['id'] for r in table if r['candidate_moves']>r['original_moves']*1.25]
    time_pass=candidate_sum<=base_sum*0.9
    regressions=[r['id'] for r in table if r['candidate_ms']-r['baseline_ms']>max(r['baseline_ms']*0.1,0.1)]
    rss_regressions=[r['id'] for r in table if r['candidate_rss_bytes']-r['baseline_rss_bytes']>max(r['baseline_rss_bytes']*0.1,1024**2)]
    worst=max(table,key=lambda r:r['baseline_ms'])
    no_worst=[r for r in table if r is not worst]
    save(SUMMARY,dict(registration_sha256=sha(REG),receipt_sha256=sha(RECEIPT),
                      elapsed_s=round(time.monotonic()-start,3),cases=table,
                      median_sum_ms=dict(base=base_sum,candidate=candidate_sum),
                      improved_cases=sum(r['candidate_ms']<r['baseline_ms'] for r in table),
                      worsened_cases=sum(r['candidate_ms']>r['baseline_ms'] for r in table),
                      omit_worst_case=dict(id=worst['id'],baseline_ms=sum(r['baseline_ms'] for r in no_worst),
                                           candidate_ms=sum(r['candidate_ms'] for r in no_worst)),
                      full_moves=dict(base=base_moves,candidate=candidate_moves),
                      moves_pass=moves_pass,move_regressions=move_regressions,
                      time_pass=time_pass,time_regressions=regressions,rss_regressions=rss_regressions,
                      verdict='ADVANCE_TO_INDEPENDENT_CONFIRMATION' if moves_pass and not move_regressions and time_pass and not regressions and not rss_regressions else 'NO_PASS'))
    print(json.dumps(table,ensure_ascii=False))

if __name__=='__main__': main()
