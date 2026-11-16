"""One registered nine-input confirmation of the frozen H01 candidate."""
from pathlib import Path
import hashlib
import importlib.util
import json
import re
import statistics
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CORPUS = HERE / 'h01-confirmation-corpus.json'
COMPARE = ROOT / 'docs/experiments/jsa_tsume_2026-11-16/komoring_comparison/run_comparison.py'
BASE = Path('/private/tmp/sekirei-jsa-20261116/target/release/examples/jsa_search')
CAND = Path('/private/tmp/sekirei-issue291-h01/target/release/examples/jsa_search')
PROOF = Path('/private/tmp/sekirei-issue291-h01/target/release/examples/jsa_proof')
RAW = ROOT / 'data/runs/issue291-h01-confirm-20261116'
REG = HERE / 'h01-confirm-registration.json'
RECEIPT = HERE / 'h01-confirm-receipt.json'
SUMMARY = HERE / 'h01-confirm-summary.json'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')


def main():
    for path in (RAW, REG, RECEIPT, SUMMARY):
        assert not path.exists(), path
    corpus = json.loads(CORPUS.read_text())
    cases = corpus['cases']
    assert len(cases) == 9 and len({x['sfen'] for x in cases}) == 9
    assert sha(CORPUS) == 'cb43f45a3b866088a2e3405ad46c6829568476869a16dc8eb327e1ee0cc8cc68'
    for item in corpus['sources']:
        assert sha(ROOT / item['path']) == item['sha256']
    for path in (BASE, CAND, PROOF, HERE/'h01.patch', HERE/'h01-summary.json'):
        assert path.is_file(), path
    assert json.loads((HERE/'h01-summary.json').read_text())['verdict'] == 'ADVANCE_TO_INDEPENDENT_CONFIRMATION'
    registration = dict(
        id='291-H01-CONFIRM-20261116',
        source_commit='14586edaf3a5e9e03739c2fa39c9e3f48e2cd56c',
        candidate_patch_sha256=sha(HERE/'h01.patch'),
        development_summary_sha256=sha(HERE/'h01-summary.json'),
        corpus_sha256=sha(CORPUS), runner_sha256=sha(Path(__file__)),
        reused_runner_sha256=sha(COMPARE),
        binaries_sha256={'base': sha(BASE), 'candidate': sha(CAND), 'proof': sha(PROOF)},
        rule='All 9 cases x 3 paired runs same first/nodes and complete. At least 2 mate answers, all new proofs verified. Median sum >=10% faster, per-case time/RSS within registered regression limits.',
        limits=dict(cases=9, repeats=3, timing_runs=54, proof_runs_max=9,
                    total_runs_max=63, threads=1, node_limit=999999, max_ply=25,
                    solver_deadline_s=8, external_deadline_s=12,
                    rss_bytes=2*1024**3, output_bytes_per_run=2*1024**2,
                    total_seconds=600, stop='first failed run or resource cap; preserve receipt; no retry'),
    )
    save(REG, registration)
    RAW.mkdir()
    spec = importlib.util.spec_from_file_location('comparison', COMPARE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    rows = []
    proofs = []
    start = time.monotonic()

    def record():
        save(RECEIPT, dict(registration_sha256=sha(REG), rows=rows, proofs=proofs))

    def check_cap():
        assert time.monotonic()-start < 600, 'total wall cap'
        assert len(rows)+len(proofs) < 63, 'run count cap'

    def run_one(binary, engine, case, repeat):
        session = mod.Session([binary, case['sfen']])
        row = dict(id=case['id'], engine=engine, repeat=repeat, status='complete')
        try:
            session.until('ready')
            sent = session.send('go')
            answer, received = session.until('answer ', seconds=12)
            fields = dict(word.split('=', 1) for word in answer.split()[1:])
            row.update(first=fields['first'], nodes=int(fields['nodes']),
                       native_elapsed_ns=int(fields['elapsed_ns']),
                       within_deadline=fields['within_deadline']=='true',
                       response_ms=(received-sent)/1e6,
                       returncode=session.finish())
            rss = re.search(r'(\d+)\s+maximum resident set size', session.text['stderr'])
            row['peak_rss_bytes'] = int(rss[1]) if rss else None
            assert row['returncode']==0 and row['within_deadline']
            assert row['peak_rss_bytes'] is not None and row['peak_rss_bytes'] <= 2*1024**3
        except Exception as exc:
            row.update(status='failed', error=repr(exc))
            session.kill()
        finally:
            row.update(process_reaped=session.p.poll() is not None,
                       sampled_group_rss_kib=session.sampled_rss)
            log = RAW / f"{case['id']}-{engine}-{repeat}.json"
            save(log, dict(events=session.events, stdout=session.text['stdout'],
                           stderr=session.text['stderr']))
            row['log_sha256'] = sha(log)
            session.close()
        rows.append(row)
        record()
        assert row['status']=='complete', row
        return row

    for index, case in enumerate(cases):
        for repeat in (1, 2, 3):
            order = (('base', BASE), ('candidate', CAND))
            if (index+repeat)%2 == 0:
                order = order[::-1]
            pair = {}
            for engine, binary in order:
                check_cap()
                pair[engine] = run_one(binary, engine, case, repeat)
            assert (pair['base']['first'], pair['base']['nodes']) == (
                pair['candidate']['first'], pair['candidate']['nodes'])

    for case in cases:
        candidate_rows = [r for r in rows if r['id']==case['id'] and r['engine']=='candidate']
        assert len(candidate_rows)==3
        assert len({r['first'] for r in candidate_rows})==1
        if candidate_rows[0]['first']=='none':
            continue
        check_cap()
        tree = RAW / f"{case['id']}-tree.json"
        session = mod.Session([PROOF, case['sfen'], tree])
        proof = dict(id=case['id'], status='complete')
        try:
            session.until('reverified=', seconds=12)
            proof['returncode'] = session.finish()
            proof['stdout'] = session.text['stdout']
            proof['stderr'] = session.text['stderr']
            rss = re.search(r'(\d+)\s+maximum resident set size', proof['stderr'])
            proof['peak_rss_bytes'] = int(rss[1]) if rss else None
            assert proof['returncode']==0 and proof['peak_rss_bytes'] is not None
            assert proof['peak_rss_bytes'] <= 2*1024**3
            assert 'verified=true invalid=None stop=None' in proof['stdout']
            assert 'reverified=true' in proof['stdout']
            assert tree.is_file() and tree.stat().st_size <= 2*1024**2
            proof['tree_sha256'] = sha(tree)
        except Exception as exc:
            proof.update(status='failed', error=repr(exc))
            session.kill()
        finally:
            proof['process_reaped'] = session.p.poll() is not None
            session.close()
            proofs.append(proof)
            record()
        assert proof['status']=='complete', proof

    table = []
    for case in cases:
        base = [r for r in rows if r['id']==case['id'] and r['engine']=='base']
        cand = [r for r in rows if r['id']==case['id'] and r['engine']=='candidate']
        table.append(dict(id=case['id'], first=base[0]['first'], nodes=base[0]['nodes'],
            base_ms=statistics.median(r['native_elapsed_ns'] for r in base)/1e6,
            candidate_ms=statistics.median(r['native_elapsed_ns'] for r in cand)/1e6,
            base_range_ms=[min(r['native_elapsed_ns'] for r in base)/1e6,
                           max(r['native_elapsed_ns'] for r in base)/1e6],
            candidate_range_ms=[min(r['native_elapsed_ns'] for r in cand)/1e6,
                                max(r['native_elapsed_ns'] for r in cand)/1e6],
            base_rss_bytes=max(r['peak_rss_bytes'] for r in base),
            candidate_rss_bytes=max(r['peak_rss_bytes'] for r in cand)))
    base_sum = sum(r['base_ms'] for r in table)
    cand_sum = sum(r['candidate_ms'] for r in table)
    time_regressions = [r['id'] for r in table
        if r['candidate_ms']-r['base_ms'] > max(r['base_ms']*.1, .1)]
    rss_regressions = [r['id'] for r in table
        if r['candidate_rss_bytes']-r['base_rss_bytes'] > max(r['base_rss_bytes']*.1, 1024**2)]
    worst = max(table, key=lambda r:r['base_ms'])
    no_worst = [r for r in table if r is not worst]
    passed = (len(proofs)>=2 and cand_sum <= base_sum*.9
              and not time_regressions and not rss_regressions)
    save(SUMMARY, dict(registration_sha256=sha(REG), receipt_sha256=sha(RECEIPT),
        elapsed_s=round(time.monotonic()-start, 3), cases=table,
        mate_answers=len(proofs), no_answer=9-len(proofs),
        median_sum_ms=dict(base=base_sum, candidate=cand_sum),
        improved_cases=sum(r['candidate_ms']<r['base_ms'] for r in table),
        worsened_cases=sum(r['candidate_ms']>r['base_ms'] for r in table),
        omit_worst_case=dict(id=worst['id'],
            base_ms=sum(r['base_ms'] for r in no_worst),
            candidate_ms=sum(r['candidate_ms'] for r in no_worst)),
        time_regressions=time_regressions, rss_regressions=rss_regressions,
        verdict='PASS' if passed else 'NO_PASS'))
    print(json.dumps(json.loads(SUMMARY.read_text()), ensure_ascii=False))


if __name__ == '__main__':
    main()
