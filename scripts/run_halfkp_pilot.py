#!/usr/bin/env python3
"""One frozen, bounded own-HalfKP generation/train/independent-gate run.

Never overwrites its initial evaluator, resumes an interrupted trial, retries
an engine failure, publishes a checkpoint or changes the engine default.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

from freeze_material_gate_openings import identity_hash, canonical_identity, load_exclusions
from split_gensfen_by_game import split_files, parse_record, validate_game_results


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        for chunk in iter(lambda: f.read(1048576), b''):
            h.update(chunk)
    return h.hexdigest()


def ref(path):
    return {'path': str(Path(path).resolve()), 'sha256': sha(path)}


def save(path, value):
    path = Path(path)
    pending = path.with_name('.' + path.name + '.pending')
    with pending.open('w') as f:
        json.dump(value, f, indent=2, allow_nan=False)
        f.write('\n'); f.flush(); os.fsync(f.fileno())
    os.replace(pending, path)


def now():
    return datetime.now(timezone.utc).isoformat()


def validate_generated_ids(path, seed, games):
    seen = set()
    for n, raw in enumerate(Path(path).read_text().splitlines(), 1):
        if not raw.strip():
            continue
        row = parse_record(raw, Path(path), n)
        prefix = f'{seed:016x}-'
        require(row.game_id.startswith(prefix), 'wrong generator seed/game identity')
        suffix = row.game_id[len(prefix):]
        require(len(suffix) == 16 and all(c in '0123456789abcdef' for c in suffix),
                'malformed generated game identity')
        require(int(suffix, 16) < games, 'generated game index beyond requested cap')
        seen.add(row.game_id)
    require(seen, 'generator produced no sampled games')
    return len(seen)


def prepare_rows(paths, excluded, output, min_ply):
    """Exclude entire games touching known holdouts; normalize move counters.

    Canonical board/side/hands identity is used before the game split. Keeping
    the original ply in column4 preserves phase metadata without treating a
    different SFEN move counter as an independent position.
    """
    records = []
    for path in paths:
        for n, line in enumerate(Path(path).read_text().splitlines(), 1):
            if line.strip():
                records.append(parse_record(line, Path(path), n))
    require(records, 'generator produced no records')
    validate_game_results(records)
    blocked = {r.game_id for r in records if identity_hash(r.sfen) in excluded}
    kept = [r for r in records if r.game_id not in blocked and r.ply >= min_ply]
    require(kept, 'no eligible games after known-holdout exclusion')
    with Path(output).open('x') as f:
        for row in kept:
            fields = row.raw.split('\t')
            fields[0] = canonical_identity(row.sfen) + ' 1'
            f.write('\t'.join(fields) + '\n')
    return {'source_rows': len(records), 'kept_rows': len(kept),
            'blocked_games': len(blocked), 'kept_games': len({r.game_id for r in kept}),
            'identities': sorted({identity_hash(r.sfen) for r in kept})}


def validate_plan(plan):
    require(plan.get('schema') == 'sekirei.own-halfkp-pilot-plan.v1', 'unknown plan schema')
    g = plan['generation']
    seeds = g['seeds']
    require(isinstance(seeds,list) and 1 <= len(seeds) <= 16 and len(set(seeds)) == len(seeds),
            'duplicate or invalid generation seeds')
    require(all(type(x) is int and 0 < x < 2**64 for x in seeds), 'invalid generation seed')
    require(1 <= g['games_per_shard'] <= 1024 and 1 <= g['depth'] <= 16 and
            1 <= g['nodes'] <= 1000000 and 0 <= g['random_plies'] <= g['min_training_ply'] < g['max_ply'] <= 512,
            'unbounded generation configuration')
    required = {str(Path(plan[k]).resolve()) for k in ['generator','packer','baseline','trainer']}
    required.update(str(Path(p).resolve()) for p in plan['exclusions']+plan['spent_manifests'])
    bound = {str(Path(x['path']).resolve()) for x in plan['bound_inputs']}
    require(required <= bound, 'unbound generator/trainer/evaluator/exclusion input')
    require(0 < plan['max_wall_hours'] <= 72, 'unbounded run wall cap')


def run(plan_path):
    base = plan_path.resolve().parent
    with (base/'run.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return run_locked(plan_path)


def run_locked(plan_path):
    base = plan_path.resolve().parent
    plan = json.loads(plan_path.read_text())
    require(not (base/'status.json').exists(), 'fresh run required; automatic restart forbidden')
    state = {'schema':'sekirei.own-halfkp-pilot-status.v1', 'status':'PREFLIGHT',
             'started_at':now(), 'pid':os.getpid(), 'plan':ref(plan_path),
             'published':False, 'default_changed':False, 'best_remains':'nn_r6'}
    started = time.monotonic()
    proc = None
    stop_requested = False
    def pins():
        require(sha(plan_path) == state['plan']['sha256'], 'frozen plan changed')
        for x in plan['bound_inputs']:
            require(sha(x['path']) == x['sha256'], 'pinned input changed: ' + x['path'])
    def snapshot():
        state['updated_at'] = now()
        state['free_mib'] = shutil.disk_usage(base).free // 1048576
        save(base/'status.json',state)
    def command(argv, name, timeout, min_free=768, stop_file=None):
        nonlocal proc, stop_requested
        pins()
        require(shutil.disk_usage(base).free // 1048576 >= min_free, 'insufficient output capacity')
        state['command'] = argv
        state['active_log'] = name
        with (base/(name+'.stdout')).open('x') as out, (base/(name+'.stderr')).open('x') as err:
            proc = subprocess.Popen(argv, stdout=out, stderr=err, stdin=subprocess.DEVNULL,
                                    start_new_session=True,
                                    env={**os.environ,'RAYON_NUM_THREADS':'1','OMP_NUM_THREADS':'1',
                                         'OPENBLAS_NUM_THREADS':'1','MKL_NUM_THREADS':'1',
                                         'VECLIB_MAXIMUM_THREADS':'1','PYTHONDONTWRITEBYTECODE':'1'})
            state['child_pid'] = proc.pid
            phase_started = time.monotonic()
            while proc.poll() is None:
                snapshot()
                if stop_file and state['free_mib'] < 768 and not stop_requested:
                    Path(stop_file).touch(exist_ok=False)
                    stop_requested = True
                    state['capacity_stop_requested'] = True
                require(time.monotonic()-phase_started < timeout, 'phase wall limit')
                require(time.monotonic()-started < plan['max_wall_hours']*3600, 'run wall limit')
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    pass
            code = proc.returncode
            state['last_child_returncode'] = code
            proc = None
            snapshot()
            require(code == 0, 'child failed with return code ' + str(code))
            require(not stop_requested, 'stopped between games for low output capacity')
        pins()
    try:
        snapshot(); pins(); validate_plan(plan)
        require(plan['baseline_sha256'] == sha(plan['baseline']), 'wrong nn_r6')
        excluded, exclusion_records = load_exclusions([Path(x) for x in plan['exclusions']])
        # Additional already-spent gate roots are identity-only inputs.
        for manifest in plan['spent_manifests']:
            data = json.loads(Path(manifest).read_text())
            require(data['identity_scheme'] == 'sha256-canonical-sfen-board-side-hands-v1', 'spent identity scheme')
            require(data.get('schema') == 'sekirei.independent-material-openings.v1' and data.get('status') == 'complete', 'spent manifest schema')
            require(data['positions'] and all(isinstance(x.get('identity_sha256'),str) and
                    re.fullmatch(r'[0-9a-f]{64}',x['identity_sha256']) for x in data['positions']),
                    'malformed spent identities')
            excluded.update(x['identity_sha256'] for x in data['positions'])
        raw = []
        for n, seed in enumerate(plan['generation']['seeds']):
            state.update(status='GENERATING', shard=n+1, total_shards=len(plan['generation']['seeds']))
            text = base/f'raw-{n:02}.txt'
            require(not text.exists(), 'raw shard already exists; no append/retry')
            require(not (base/'STOP_GENERATION').exists(), 'generation stop file already present')
            g = plan['generation']
            cmd = [plan['generator'],'--out',str(text),'--games',str(g['games_per_shard']),
                   '--seed',str(seed),'--depth',str(g['depth']),'--nodes',str(g['nodes']),
                   '--random-plies',str(g['random_plies']),'--max-ply',str(g['max_ply']),
                   '--hash-mb','16','--eval',plan['baseline'],'--fv-scale','24',
                   '--stop-file',str(base/'STOP_GENERATION')]
            command(cmd,f'generate-{n:02}',6*3600,stop_file=base/'STOP_GENERATION')
            generation_log=(base/f'generate-{n:02}.stderr').read_text()
            require('(HalfKp)' in generation_log, 'generator evaluator mismatch')
            require(not (base/'STOP_GENERATION').exists() and 'stop file found' not in generation_log,
                    'generation stopped early; frozen game count not completed')
            validate_generated_ids(text,seed,g['games_per_shard'])
            raw.append(text)
            state['generated_shards'] = [ref(x) for x in raw]
            snapshot()
        state['status']='PREPARING_DATA';snapshot()
        filtered = base/'eligible.txt'
        selected = prepare_rows(raw,excluded,filtered,plan['generation']['min_training_ply'])
        ids = selected.pop('identities')
        save(base/'known-exclusion-audit.json',dict(selected,known_exclusion_count=len(excluded),
             known_manifest_scope_only=True,exclusions=exclusion_records))
        holdout = {'schema':'sekirei.opening-identity-exclusions.v1','identity_scheme':
                   'sha256-canonical-sfen-board-side-hands-v1','role':'nnue-holdouts',
                   'complete_for_scope':True,'scope':'All retained positions of this one fresh pilot, train AND validation; no assertion about unreceived historical sets.',
                   'identities':ids,'sources':[ref(filtered)]}
        save(base/'pilot-data-exclusion.json',holdout)
        split = split_files([filtered],base/'train.txt',base/'val.txt',base/'split.json',
                            plan['training']['validation_ratio'],plan['training']['seed'],64)
        require(split['train']['positions'] >= 4096 and split['validation']['positions'] >= 256,
                'insufficient whole-game train/validation data')
        for arm in ['train','val']:
            command([plan['packer'],'pack',str(base/(arm+'.bin')),str(base/(arm+'.txt'))],
                    'pack-'+arm,300)
            count = split['train' if arm=='train' else 'validation']['positions']
            require((base/(arm+'.bin')).stat().st_size == count*158, 'packed row count mismatch')
        state['status']='TRAINING';snapshot()
        t = plan['training']
        candidate = base/'nn_r8c1.bin'
        cmd=[sys.executable,'-u','-B',plan['trainer'],'--data',str(base/'train.bin'),
             '--val-data',str(base/'val.bin'),'--init-bin',plan['baseline'],
             '--out',str(candidate),'--save',str(base/'nn_r8c1.pt'),'--ckpt',str(base/'training.ckpt'),
             '--epochs','1','--batch',str(t['batch']),'--lr',str(t['lr']),
             '--lam','1.0','--scale','400','--fv-scale','24','--device','cpu',
             '--threads','1','--seed',str(t['seed']),'--log-every','10','--ckpt-every','20']
        command(cmd,'training',6*3600,min_free=1200)
        # Load the exported file with the native inference implementation.
        command([plan['packer'],'check',str(candidate),'24',str(base/'val.txt'),'8'],
                'export-load-check',120)
        state.update(status='CANDIDATE_TRAINED_AWAITING_INDEPENDENT_GATE',candidate=ref(candidate),
                     float_checkpoint=ref(base/'nn_r8c1.pt'),completed_at=now(),
                     validation_is_diagnostic_only=True,adopted=False)
        save(base/'candidate.json',dict(state,training_split=ref(base/'split.json')))
        snapshot()
    except BaseException as error:
        if proc is not None and proc.poll() is None:
            try:
                os.killpg(proc.pid,signal.SIGTERM);proc.wait(timeout=20)
            except ProcessLookupError:
                pass
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid,signal.SIGKILL);proc.wait()
        state.update(status='STOPPED_NOT_QUALIFIED',error_type=type(error).__name__,error=str(error),
                     completed_at=now(),adopted=False)
        snapshot()
        raise


def interrupted(number, _frame):
    raise InterruptedError('received signal ' + str(number))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan',type=Path,required=True)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM,interrupted)
    signal.signal(signal.SIGINT,interrupted)
    run(args.plan)
