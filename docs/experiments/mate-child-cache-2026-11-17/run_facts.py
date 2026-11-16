"""Preregister one F01 screen; never extend a completed or rejected attempt."""
import json
import re
import statistics
import subprocess
import time
from pathlib import Path
from datetime import datetime, timezone
from run import HERE, ROOT, OUT, INPUT, OLD, binary, sha, save, module


def main():
    raw=OUT/'facts-screen'
    assert not raw.exists()
    raw.mkdir()
    mod=module('common_runner',OLD/'komoring_comparison/run_comparison.py');mod.OUT=raw
    modes=['base','facts','facts-counted','facts-base-counted']
    for m in modes:mod.BINARIES['sekirei_'+m]=binary(m)
    files=[Path(__file__),HERE/'run.py',HERE/'prepare_facts.py',HERE/'prepare.py',HERE/'F01_PLAN.md',
           INPUT/'corpus.json',OLD/'komoring_comparison/run_comparison.py',OUT/'facts-tests.log',
           OUT/'profile/receipt.json',binary('facts','jsa_proof'),binary('base','jsa_check')]
    files += [binary(m) for m in modes]+[OUT/(m+'.patch') for m in modes]
    reg=dict(id='MATE-F01-20261117',registered_utc=datetime.now(timezone.utc).isoformat(),
             source='9f3539f29d6ab85fbe75a733953f271e633eca7f',
             hashes={str(p):sha(p) for p in files},threads=1,evaluator='none',
             search_nodes=999999,max_ply=25,search_seconds=8,outer_seconds=12,rss_bytes=2*1024**3,
             solver_runs_cap=78,checker_runs_cap=6,total_seconds=1200,no_extension=True)
    save(raw/'registration.json',reg)
    cases=json.loads((INPUT/'corpus.json').read_text())['cases']
    prior={r['id']:r for r in json.loads((ROOT/'docs/experiments/issue291-mate-cost-2026-11-16/d01-summary.json').read_text())['cases']}
    rows=[];proofs=[];start=time.monotonic()
    receipt=dict(registration_sha256=sha(raw/'registration.json'),rows=rows,proofs=proofs)
    def cap():
        assert time.monotonic()-start<1200 and len(rows)+len(proofs)<78
    def run(m,case,rep):
        cap();row=mod.run_one('sekirei_'+m,case,rep);rows.append(row)
        save(raw/'receipt.json',receipt)
        assert row['status']=='complete',row
        assert row['first']==prior[case['id']]['first'] and row['nodes']==prior[case['id']]['nodes']
        fields=dict(w.split('=',1) for w in row['answer'].split()[1:])
        for key in ['lookup_calls','map_reads']:
            if key in fields:row[key]=int(fields[key])
        if 'make_moves' in row:assert row['make_moves']==prior[case['id']]['make_moves']
        save(raw/'receipt.json',receipt)
        print(case['id'],m,rep,row['nodes'],row.get('map_reads'),round(row['native_elapsed_ns']/1e6,3),flush=True)
    for case in cases:
        for m in ['facts-base-counted','facts-counted']:run(m,case,1)
        pair=rows[-2:];assert pair[0]['lookup_calls']==pair[1]['lookup_calls']
        assert pair[1]['map_reads']<=pair[0]['map_reads']
    for i,case in enumerate(cases):
        for rep in range(1,6):
            order=['base','facts'] if (i+rep)%2 else ['facts','base']
            for m in order:run(m,case,rep)
    helper=module('proof_reader',OLD/'check_artifacts.py')
    for case in cases:
        cap();tree=raw/(case['id']+'-tree.json')
        session=mod.Session([binary('facts','jsa_proof'),case['sfen'],tree])
        proof=dict(id=case['id'],status='complete')
        try:
            session.until('reverified=',seconds=12)
            proof.update(returncode=session.finish(),stdout=session.text['stdout'],stderr=session.text['stderr'])
            assert proof['returncode']==0 and 'verified=true invalid=None stop=None' in proof['stdout']
            rss=re.search(r'(\d+)\s+maximum resident set size',proof['stderr'])
            assert rss and int(rss[1])<=2*1024**3
            assert tree.stat().st_size<=2*1024**2
            data=' '.join(helper.tree_tokens(json.loads(tree.read_text())))+'\n'
            replay=subprocess.run([str(binary('base','jsa_check')),case['sfen']],input=data,text=True,capture_output=True,timeout=3)
            assert replay.returncode==0,replay.stderr
            proof.update(tree_sha256=sha(tree),replay_output=replay.stdout,
                         original_tree_sha256=sha(INPUT/(case['id']+'-tree.json')))
        except Exception as exc:
            proof.update(status='failed',error=repr(exc));session.kill()
        finally:
            proof['process_reaped']=session.p.poll() is not None
            session.close();proofs.append(proof);save(raw/'receipt.json',receipt)
        assert proof['status']=='complete',proof
    table=[]
    for case in cases:
        selected={m:[r for r in rows if r['id']==case['id'] and r['engine']=='sekirei_'+m] for m in modes}
        a,b=selected['base'],selected['facts'];assert len(a)==len(b)==5
        counted_base,counted_candidate=selected['facts-base-counted'][0],selected['facts-counted'][0]
        table.append(dict(id=case['id'],baseline_ms=statistics.median(r['native_elapsed_ns'] for r in a)/1e6,
                          candidate_ms=statistics.median(r['native_elapsed_ns'] for r in b)/1e6,
                          baseline_range_ms=[min(r['native_elapsed_ns'] for r in a)/1e6,max(r['native_elapsed_ns'] for r in a)/1e6],
                          candidate_range_ms=[min(r['native_elapsed_ns'] for r in b)/1e6,max(r['native_elapsed_ns'] for r in b)/1e6],
                          baseline_rss=max(r['peak_rss_bytes'] for r in a),candidate_rss=max(r['peak_rss_bytes'] for r in b),
                          make_moves=counted_base['make_moves'],lookup_calls=counted_base['lookup_calls'],
                          baseline_map_reads=counted_base['map_reads'],candidate_map_reads=counted_candidate['map_reads']))
    bs=sum(r['baseline_ms'] for r in table);cs=sum(r['candidate_ms'] for r in table)
    regressions=[r['id'] for r in table if r['candidate_ms']-r['baseline_ms']>max(r['baseline_ms']*.1,.1)]
    rss=[r['id'] for r in table if r['candidate_rss']-r['baseline_rss']>max(r['baseline_rss']*.1,1024**2)]
    worst=max(table,key=lambda r:r['baseline_ms'])
    summary=dict(cases=table,time_ms=dict(base=bs,candidate=cs),
                 map_reads=dict(base=sum(r['baseline_map_reads'] for r in table),candidate=sum(r['candidate_map_reads'] for r in table)),
                 improved=sum(r['candidate_ms']<r['baseline_ms'] for r in table),worsened=sum(r['candidate_ms']>r['baseline_ms'] for r in table),
                 without_worst=dict(id=worst['id'],base_ms=bs-worst['baseline_ms'],candidate_ms=cs-worst['candidate_ms']),
                 time_regressions=regressions,rss_regressions=rss,
                 verdict='ADVANCE' if cs<=bs*.9 and not regressions and not rss else 'NO_PASS',
                 receipt_sha256=sha(raw/'receipt.json'))
    save(raw/'summary.json',summary);print(json.dumps(summary,ensure_ascii=False,indent=2))


if __name__=='__main__':main()
