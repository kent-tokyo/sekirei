"""Bounded shadow diagnostic and separately registered candidate screen."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import statistics
import subprocess
import time
from datetime import datetime, timezone

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
OUT = ROOT / 'data/runs/mate-child-cache-20261117'
INPUT = ROOT / 'data/runs/jsa-tsume-20261116'
OLD = ROOT / 'docs/experiments/jsa_tsume_2026-11-16'
STEM = '/private/tmp/sekirei-child-cache-20261117-'


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def save(p, v):
    Path(p).write_text(json.dumps(v, ensure_ascii=False, indent=2) + '\n')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    obj = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj


def binary(mode, name='jsa_search'):
    return Path(STEM + mode) / 'target/release/examples' / name


def main(stage):
    raw = OUT / stage
    assert not raw.exists(), 'No overwrite of registered attempts'
    raw.mkdir()
    mod = module('common_runner', OLD / 'komoring_comparison/run_comparison.py')
    mod.OUT = raw
    modes = ['shadow'] if stage == 'diagnostic' else ['base', 'candidate', 'counted']
    for mode in modes:
        mod.BINARIES['sekirei_' + mode] = binary(mode)
    files = [HERE / 'PLAN.md', Path(__file__), HERE / 'prepare.py', HERE / 'cache.inc.rs',
             HERE / 'cache_tests.inc.rs', INPUT / 'corpus.json',
             OLD / 'komoring_comparison/run_comparison.py']
    files += [binary(m) for m in modes] + [OUT / (m + '.patch') for m in modes]
    if stage == 'screen':
        files += [binary('candidate','jsa_proof'), binary('base','jsa_check'),
                  OUT / 'diagnostic/summary.json', OUT / 'candidate-tests.log']
        assert json.loads((OUT / 'diagnostic/summary.json').read_text())['advance']
    registration = dict(id='CHILD-CACHE-20261117-' + stage.upper(),
                        registered_utc=datetime.now(timezone.utc).isoformat(),
                        source='9f3539f29d6ab85fbe75a733953f271e633eca7f',
                        hashes={str(p): sha(p) for p in files},
                        threads=1, evaluator='none', search_nodes=999999, max_ply=25,
                        search_seconds=8, outer_seconds=12, rss_bytes=2*1024**3,
                        solver_runs_cap=6 if stage == 'diagnostic' else 72,
                        total_seconds=1200, no_extension=True)
    save(raw / 'registration.json', registration)
    cases = json.loads((INPUT / 'corpus.json').read_text())['cases']
    prior = {r['id']: r for r in json.loads((ROOT / 'docs/experiments/issue291-mate-cost-2026-11-16/d01-summary.json').read_text())['cases']}
    rows, proofs = [], []
    receipt = dict(registration_sha256=sha(raw / 'registration.json'), rows=rows, proofs=proofs)
    start = time.monotonic()

    def cap():
        assert time.monotonic() - start < 1200
        assert len(rows) + len(proofs) < registration['solver_runs_cap']

    def run(mode, case, rep):
        cap()
        row = mod.run_one('sekirei_' + mode, case, rep)
        rows.append(row)
        save(raw / 'receipt.json', receipt)
        assert row['status'] == 'complete', row
        assert row['first'] == prior[case['id']]['first']
        assert row['nodes'] == prior[case['id']]['nodes']
        fields = dict(w.split('=',1) for w in row['answer'].split()[1:])
        for key in ['cache_hits', 'hit_edges', 'cache_calls']:
            if key in fields:
                row[key] = int(fields[key])
        save(raw / 'receipt.json', receipt)
        print(case['id'], mode, rep, row['nodes'], row.get('make_moves'),
              row.get('cache_hits'), round(row['native_elapsed_ns']/1e6, 3), flush=True)
        return row

    if stage == 'diagnostic':
        for case in cases:
            row = run('shadow', case, 1)
            assert row['make_moves'] == prior[case['id']]['make_moves']
        moves = sum(r['make_moves'] for r in rows)
        repeated = sum(r['hit_edges'] for r in rows)
        save(raw / 'summary.json', dict(make_moves=moves, repeated_child_hash_moves=repeated,
                                         fraction=repeated/moves, advance=repeated >= moves*0.2,
                                         receipt_sha256=sha(raw/'receipt.json')))
        print((raw / 'summary.json').read_text())
        return

    for case in cases:
        run('counted', case, 1)
    for i, case in enumerate(cases):
        for rep in range(1, 6):
            order = ['base','candidate'] if (i+rep)%2 else ['candidate','base']
            for mode in order:
                run(mode,case,rep)
    helper = module('proof_reader', OLD / 'check_artifacts.py')
    for case in cases:
        cap()
        tree = raw / (case['id'] + '-tree.json')
        session = mod.Session([binary('candidate','jsa_proof'), case['sfen'], tree])
        proof = dict(id=case['id'], status='complete')
        try:
            session.until('reverified=', seconds=12)
            code = session.finish()
            proof.update(returncode=code, stdout=session.text['stdout'], stderr=session.text['stderr'])
            assert code == 0 and 'verified=true invalid=None stop=None' in proof['stdout']
            assert tree.stat().st_size <= 2*1024**2
            data = ' '.join(helper.tree_tokens(json.loads(tree.read_text())))+'\n'
            replay = subprocess.run([str(binary('base','jsa_check')),case['sfen']],input=data,
                                    text=True,capture_output=True,timeout=3)
            assert replay.returncode == 0, replay.stderr
            proof.update(tree_sha256=sha(tree), replay_output=replay.stdout,
                         original_tree_sha256=sha(INPUT/(case['id']+'-tree.json')))
        except Exception as exc:
            proof.update(status='failed',error=repr(exc))
            session.kill()
        finally:
            proof['process_reaped'] = session.p.poll() is not None
            session.close()
            proofs.append(proof)
            save(raw / 'receipt.json', receipt)
        assert proof['status'] == 'complete', proof
    table = []
    for case in cases:
        samples = {m:[r for r in rows if r['id']==case['id'] and r['engine']=='sekirei_'+m] for m in modes}
        a,b = samples['base'],samples['candidate']
        assert len(a)==len(b)==5
        table.append(dict(id=case['id'],baseline_ms=statistics.median(r['native_elapsed_ns'] for r in a)/1e6,
                          candidate_ms=statistics.median(r['native_elapsed_ns'] for r in b)/1e6,
                          baseline_range_ms=[min(r['native_elapsed_ns'] for r in a)/1e6,max(r['native_elapsed_ns'] for r in a)/1e6],
                          candidate_range_ms=[min(r['native_elapsed_ns'] for r in b)/1e6,max(r['native_elapsed_ns'] for r in b)/1e6],
                          baseline_rss=max(r['peak_rss_bytes'] for r in a),candidate_rss=max(r['peak_rss_bytes'] for r in b),
                          original_moves=prior[case['id']]['make_moves'],candidate_moves=samples['counted'][0]['make_moves'],
                          cache_hits=samples['counted'][0]['cache_hits'],cache_calls=samples['counted'][0]['cache_calls']))
    base_ms=sum(r['baseline_ms'] for r in table); candidate_ms=sum(r['candidate_ms'] for r in table)
    base_moves=sum(r['original_moves'] for r in table); candidate_moves=sum(r['candidate_moves'] for r in table)
    regressions=[r['id'] for r in table if r['candidate_ms']-r['baseline_ms']>max(r['baseline_ms']*.1,.1)]
    rss=[r['id'] for r in table if r['candidate_rss']-r['baseline_rss']>8*1024**2]
    passed=candidate_ms <= base_ms*.9 and candidate_moves <= base_moves*.8 and not regressions and not rss
    worst=max(table,key=lambda r:r['baseline_ms'])
    summary=dict(cases=table,time_ms=dict(base=base_ms,candidate=candidate_ms),
                 make_moves=dict(base=base_moves,candidate=candidate_moves),
                 improved=sum(r['candidate_ms']<r['baseline_ms'] for r in table),
                 worsened=sum(r['candidate_ms']>r['baseline_ms'] for r in table),
                 without_worst=dict(id=worst['id'],base_ms=base_ms-worst['baseline_ms'],candidate_ms=candidate_ms-worst['candidate_ms']),
                 time_regressions=regressions,rss_regressions=rss,
                 verdict='ADVANCE' if passed else 'NO_PASS',receipt_sha256=sha(raw/'receipt.json'))
    save(raw/'summary.json',summary)
    print(json.dumps(summary,ensure_ascii=False,indent=2))


if __name__ == '__main__':
    parser=argparse.ArgumentParser();parser.add_argument('stage',choices=['diagnostic','screen'])
    main(parser.parse_args().stage)
