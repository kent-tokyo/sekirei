"""Registered 6 x 3 phase-count diagnostic; preserve original raw logs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CORPUS = ROOT / 'data/runs/jsa-tsume-20261116/corpus.json'
OLD = ROOT / 'data/runs/jsa-kh-20261116-r2/receipt.json'
DRIVER = ROOT / 'docs/experiments/jsa_tsume_2026-11-16/komoring_comparison/run_comparison.py'
BINARY = Path('/private/tmp/sekirei-issue291-d01/target/release/examples/jsa_search')
RAW = ROOT / 'data/runs/issue291-d01-20261116'
REG = HERE / 'd01-registration.json'
RECEIPT = HERE / 'd01-receipt.json'
SUMMARY = HERE / 'd01-summary.json'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')

def main():
    for path in (RAW, REG, RECEIPT, SUMMARY):
        assert not path.exists(), f'refuse overwrite: {path}'
    assert sha(CORPUS) == '0e5c60502aadd4b07acd9b726324851cd8b1f994f0ef9e152e3140db23a6e474'
    assert sha(OLD) == '12d06b022475d2ad973a0dcce88ae6c217f0496f1115b07006bf9da2c3bc256a'
    assert BINARY.is_file()
    reg = dict(id='291-D01-20261116', source_commit='14586edaf3a5e9e03739c2fa39c9e3f48e2cd56c',
               source_patch_sha256=sha(HERE/'d01-instrumentation.patch'),
               prepare_sha256=sha(HERE/'prepare_phased.py'), runner_sha256=sha(Path(__file__)),
               reused_runner_sha256=sha(DRIVER), binary_sha256=sha(BINARY),
               corpus_sha256=sha(CORPUS), prior_receipt_sha256=sha(OLD),
               hypothesis='Child hash probes account for a material share of full move applications.',
               rule='All six cases x three repeats match prior first/nodes/total; five disjoint phases sum to total and unclassified=0.',
               limits=dict(cases=6, repeats=3, runs=18, threads=1, node_limit=999999,
                           max_ply=25, solver_deadline_s=8, external_deadline_s=12,
                           rss_bytes=2*1024**3, output_bytes_per_run=2*1024**2,
                           total_seconds=300, stop='first mismatch or failed run; no retries'))
    save(REG, reg)
    RAW.mkdir()
    spec=importlib.util.spec_from_file_location('jsa_comparison', DRIVER)
    mod=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    mod.OUT=RAW
    mod.BINARIES['sekirei_phased']=BINARY
    prior=json.loads(OLD.read_text())['results']
    expected={(r['id'],r['repeat']): r for r in prior if r['engine']=='sekirei_counted'}
    assert len(expected)==18
    cases=json.loads(CORPUS.read_text())['cases']
    rows=[]
    start=time.monotonic()
    for case in cases:
        for repeat in (1,2,3):
            if time.monotonic()-start>=300:
                raise RuntimeError('total wall limit before next run')
            row=mod.run_one('sekirei_phased',case,repeat)
            rows.append(row)
            save(RECEIPT,dict(registration_sha256=sha(REG),rows=rows))
            if row['status']!='complete':
                raise RuntimeError(f"run failed: {case['id']} repeat {repeat}: {row.get('error')}")
            fields=dict(x.split('=',1) for x in row['answer'].split()[1:])
            phases={k:int(fields['phase_'+k]) for k in ('unclassified','check','hash','descent','answer')}
            row['phases']=phases
            old=expected[case['id'],repeat]
            assert (row['first'],row['nodes'],row['make_moves']) == (old['first'],old['nodes'],old['make_moves']), (row,old)
            assert sum(phases.values())==row['make_moves'] and phases['unclassified']==0
            save(RECEIPT,dict(registration_sha256=sha(REG),rows=rows))
    summary=[]
    for case in cases:
        three=[r for r in rows if r['id']==case['id']]
        assert all(r['phases']==three[0]['phases'] for r in three)
        summary.append(dict(id=case['id'],first=three[0]['first'],nodes=three[0]['nodes'],
                            make_moves=three[0]['make_moves'],phases=three[0]['phases']))
    save(SUMMARY,dict(registration_sha256=sha(REG),receipt_sha256=sha(RECEIPT),
                      elapsed_s=round(time.monotonic()-start,3),cases=summary))
    print(json.dumps(summary, ensure_ascii=False))

if __name__=='__main__':
    main()
