#!/usr/bin/env python3
"""One frozen, bounded own-HalfKP generation and training pilot.

Capacity pressure pauses and continues the same process automatically. Never
restarts a terminated trial, retries an engine failure, overwrites its initial
evaluator, publishes a checkpoint or changes the default.
An explicit, frozen capacity-continuation plan may retain completed games.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import fcntl
import errno
import math
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
from run_observed_process import cleanup_child, cleanup_preserving_error


class PilotShutdown(RuntimeError):
    """Explicit termination must not be mistaken for transient OSError telemetry."""


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


def candidate_name(plan):
    """Keep legacy plans compatible while binding new artifacts to one safe ID."""
    name = plan.get('candidate_id', 'nn_r8c1')
    require(isinstance(name, str) and
            re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,63}', name) is not None,
            'invalid candidate_id: expected 1-64 ASCII letters, digits, underscores or hyphens')
    require(name.casefold() not in {'train', 'val', 'emergency-reserve'},
            'candidate_id collides with pilot data or reserve')
    return name


def generation_jobs(plan):
    g = plan['generation']
    counts = g.get('games_per_seed', [g['games_per_shard']] * len(g['seeds']))
    require(isinstance(counts, list) and len(counts) == len(g['seeds']) and
            all(type(n) is int and 1 <= n <= 1024 for n in counts),
            'invalid per-seed game counts')
    return list(zip(g['seeds'], counts))


def capacity_retained(plan):
    """Validate an operator-frozen carry-forward; never restart an old process.

    Only native return0 with an explicit between-games capacity stop is eligible.
    The old experiment, every raw shard and its log remain immutable inputs.
    """
    amendment = plan.get('manual_capacity_continuation')
    if amendment is None:
        require('games_per_seed' not in plan['generation'],
                'variable game counts require a capacity-continuation amendment')
        return []
    bound = {str(Path(x['path']).resolve()): x['sha256'] for x in plan['bound_inputs']}
    def pinned(item):
        require(bound.get(str(Path(item['path']).resolve())) == item['sha256'] and
                sha(item['path']) == item['sha256'], 'unbound or changed retained evidence')
        return Path(item['path']).resolve()
    source_path = pinned(amendment['source_plan'])
    source = json.loads(source_path.read_text())
    status_path = pinned(amendment['source_status'])
    require(status_path == source_path.parent/'status.json', 'wrong source status location')
    status = json.loads(status_path.read_text())
    require('manual_capacity_continuation' not in source,
            'chained continuation requires a separately reviewed protocol')
    require(status.get('status') == 'STOPPED_NOT_QUALIFIED' and
            status.get('error') == 'stopped between games for low output capacity' and
            status.get('capacity_stop_requested') is True and
            status.get('last_child_returncode') == 0 and status.get('adopted') is False and
            status.get('plan') == amendment['source_plan'], 'ineligible capacity-stop source')
    require(not any((source_path.parent/name).exists() for name in
            ('generation-receipt.json', 'eligible.txt', 'split.json', 'train.bin',
             'val.bin', 'training.stdout', 'training.ckpt', 'candidate.json',
             candidate_name(source)+'.bin', candidate_name(source)+'.pt')),
            'source already reached data preparation or training')
    require(candidate_name(plan) == candidate_name(source) and
            plan['baseline_sha256'] == source['baseline_sha256'] and
            plan['baseline'] == source['baseline'] and
            plan['generator'] == source['generator'] and plan['packer'] == source['packer'] and
            plan['training'] == source['training'] and plan['source_base'] == source['source_base'] and
            plan['exclusions'] == source['exclusions'] and
            plan['spent_manifests'] == source['spent_manifests'],
            'capacity continuation changed frozen evaluator/training/exclusions')
    for item in source['bound_inputs']:
        require(bound.get(str(Path(item['path']).resolve())) == item['sha256'],
                'source pinned input omitted or changed')
    source_bound = {str(Path(x['path']).resolve()): x['sha256'] for x in source['bound_inputs']}
    require(sha(plan['trainer']) == source_bound.get(str(Path(source['trainer']).resolve())),
            'active trainer differs from source recipe')
    for name in ('freeze_material_gate_openings.py', 'split_gensfen_by_game.py',
                 'generate_search_diagnostic.py', 'paired_search_diagnostics.py', 'nps_threads.py',
                 'run_observed_process.py'):
        active = Path(__file__).resolve().parent/name
        prior = Path(source['trainer']).resolve().parent/name
        require(source_bound.get(str(prior)) == bound.get(str(active)) == sha(active),
                'active preparation helper differs from source recipe: ' + name)
    for key, value in source['generation'].items():
        if key not in ('seeds', 'games_per_seed'):
            require(plan['generation'].get(key) == value, 'generation recipe changed')
    old_jobs = generation_jobs(source)
    retained = amendment['retained_shards']
    require(isinstance(retained, list) and 1 <= len(retained) <= len(old_jobs) and
            status.get('shard') == len(retained), 'wrong retained source prefix')
    require(status.get('generated_shards') == [x['raw'] for x in retained[:-1]],
            'successful source shard receipt mismatch')
    seen = set()
    for n, item in enumerate(retained):
        seed, requested = old_jobs[n]
        games = item['completed_games']
        require(item['seed'] == seed and type(games) is int and 0 < games <= requested,
                'invalid retained completed game count')
        raw = pinned(item['raw']); log = pinned(item['stderr'])
        require(raw == source_path.parent/f'raw-{n:02}.txt' and
                log == source_path.parent/f'generate-{n:02}.stderr', 'wrong retained shard location')
        text = raw.read_text()
        require(text.endswith('\n'), 'truncated retained raw shard')
        validate_generated_ids(raw, seed, games)
        rows = [parse_record(line, raw, i) for i, line in enumerate(text.splitlines(), 1)]
        validate_game_results(rows)
        for row in rows:
            key = (row.game_id, row.ply)
            require(key not in seen, 'duplicate retained game/ply')
            seen.add(key)
        log_text = log.read_text()
        require('(HalfKp)' in log_text and not re.search(r'(?i)panic|os \{|error:', log_text),
                'retained generator has evaluator/error evidence')
        markers = re.findall(r'^stop file found after (\d+) games$', log_text, re.M)
        if n == len(retained)-1:
            require(markers == [str(games)], 'missing exact cooperative stop marker')
        else:
            require(not markers and games == requested, 'incomplete successful source shard')
    new_jobs = generation_jobs(plan)
    # Unstarted original seeds keep their original budgets. Exactly one new seed
    # replaces the unfinished suffix; it must be explicitly registered in advance.
    suffix = old_jobs[len(retained):]
    remainder = old_jobs[len(retained)-1][1] - retained[-1]['completed_games']
    replacement = amendment['replacement_seed']
    require(type(replacement) is int and replacement not in source['generation']['seeds'],
            'replacement seed overlaps original plan')
    expected = suffix + ([(replacement, remainder)] if remainder else [])
    require(new_jobs == expected, 'continuation does not preserve exact remaining game budget')
    require(sum(x['completed_games'] for x in retained) + sum(n for _, n in new_jobs) ==
            sum(n for _, n in old_jobs), 'continuation total game budget mismatch')
    return [Path(x['raw']['path']) for x in retained]


def quality_floors(plan):
    """Optional preregistered minima cannot weaken the original position floors."""
    specified = plan.get('quality_floors', {})
    allowed = {'train_positions', 'validation_positions', 'train_games', 'validation_games'}
    require(isinstance(specified, dict) and set(specified) <= allowed and
            all(type(value) is int and value > 0 for value in specified.values()),
            'invalid quality_floors: only positive integer position/game minima are allowed')
    floors = {'train_positions':4096, 'validation_positions':256}
    for key, value in specified.items():
        floors[key] = max(floors.get(key, 0), value)
    return floors


def validate_quality(plan, split):
    for key, minimum in quality_floors(plan).items():
        arm, field = key.split('_', 1)
        observed = split[arm].get(field)
        require(type(observed) is int and observed >= minimum,
                f'insufficient whole-game train/validation data: {key} {observed} < {minimum}')


def validate_plan(plan):
    require(plan.get('schema') == 'sekirei.own-halfkp-pilot-plan.v1', 'unknown plan schema')
    candidate_name(plan)
    quality_floors(plan)
    g = plan['generation']
    seeds = g['seeds']
    require(isinstance(seeds,list) and 1 <= len(seeds) <= 16 and len(set(seeds)) == len(seeds),
            'duplicate or invalid generation seeds')
    require(all(type(x) is int and 0 < x < 2**64 for x in seeds), 'invalid generation seed')
    require(1 <= g['games_per_shard'] <= 1024 and 1 <= g['depth'] <= 16 and
            1 <= g['nodes'] <= 1000000 and 0 <= g['random_plies'] <= g['min_training_ply'] < g['max_ply'] <= 512,
            'unbounded generation configuration')
    generation_jobs(plan)
    required = {str(Path(plan[k]).resolve()) for k in ['generator','packer','baseline','trainer']}
    required.update(str(Path(p).resolve()) for p in plan['exclusions']+plan['spent_manifests'])
    if 'parameter_diff_tool' in plan:
        require(isinstance(plan['parameter_diff_tool'], str) and plan['parameter_diff_tool'],
                'invalid parameter_diff_tool path')
        required.add(str(Path(plan['parameter_diff_tool']).resolve()))
    bound = {str(Path(x['path']).resolve()) for x in plan['bound_inputs']}
    require(required <= bound, 'unbound generator/trainer/evaluator/exclusion input')
    require(0 < plan['max_wall_hours'] <= 72, 'unbounded run wall cap')


def save_with_reserve(path, value, reserve):
    try:
        save(path, value)
    except OSError as error:
        if error.errno != errno.ENOSPC or not reserve.exists():
            raise
        reserve.unlink()
        save(path, value)


def allocate_reserve(path, mib=128):
    try:
        with path.open('xb') as stream:
            block = bytes(1048576)
            for _ in range(mib):
                stream.write(block)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def observe_host_cpu():
    """Unknown/transient process sampling is a wait condition, never admission."""
    try:
        result = subprocess.run(['/bin/ps','-Ao','pcpu='],capture_output=True,text=True,timeout=10)
        values = [float(x) for x in result.stdout.split()]
        if result.returncode != 0 or not values or not all(math.isfinite(x) and x >= 0 for x in values):
            return None
        return sum(values)
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def terminate_child(proc):
    """Boundedly terminate an owned child; an incomplete cleanup raises."""
    return cleanup_child(proc)


def pause_child_for_capacity(proc, wait_capacity, restore_reserve, record):
    """Keep the same owned process group, RNG, optimizer and open files alive.

    A failed child is never relaunched. The caller owns a Popen created with
    start_new_session=True; an unreaped direct child cannot have its PID reused.
    """
    if proc.poll() is not None:
        return 0.0
    try:
        require(os.getpgid(proc.pid) == proc.pid and proc.pid != os.getpgrp(),
                'capacity pause requires an owned independent process group')
        os.killpg(proc.pid, signal.SIGSTOP)
    except ProcessLookupError:
        if proc.poll() is not None:
            return 0.0
        raise
    paused_at = time.monotonic()
    try:
        record('paused', 0.0)
        wait_capacity()
        require(proc.poll() is None, 'capacity-paused child exited; no retry')
        restore_reserve()
        # Recheck after the real reserve allocation: it consumes output space.
        wait_capacity()
        require(proc.poll() is None, 'capacity-paused child exited; no retry')
        os.killpg(proc.pid, signal.SIGCONT)
        duration = time.monotonic() - paused_at
        record('resumed', duration)
        return duration
    except BaseException as error:
        cleanup_preserving_error(error, proc)
        raise


def prepare_data(plan, base):
    excluded, exclusion_records = load_exclusions([Path(x) for x in plan['exclusions']])
    for manifest in plan['spent_manifests']:
        data = json.loads(Path(manifest).read_text())
        require(data['identity_scheme'] == 'sha256-canonical-sfen-board-side-hands-v1', 'spent identity scheme')
        require(data.get('schema') == 'sekirei.independent-material-openings.v1' and data.get('status') == 'complete', 'spent manifest schema')
        require(data['positions'] and all(isinstance(x.get('identity_sha256'),str) and
                re.fullmatch(r'[0-9a-f]{64}',x['identity_sha256']) for x in data['positions']), 'malformed spent identities')
        excluded.update(x['identity_sha256'] for x in data['positions'])
    receipt = json.loads((base/'generation-receipt.json').read_text())
    raw = [Path(x['path']) for x in receipt]
    for x in receipt:
        require(sha(x['path']) == x['sha256'], 'raw shard changed before preparation')
    selected = prepare_rows(raw,excluded,base/'eligible.txt',plan['generation']['min_training_ply'])
    ids = selected.pop('identities')
    save(base/'known-exclusion-audit.json',dict(selected,known_exclusion_count=len(excluded),
         known_manifest_scope_only=True,exclusions=exclusion_records))
    save(base/'pilot-data-exclusion.json',{'schema':'sekirei.opening-identity-exclusions.v1',
         'identity_scheme':'sha256-canonical-sfen-board-side-hands-v1','role':'nnue-holdouts',
         'complete_for_scope':True,'scope':'All retained positions of this one fresh pilot, train AND validation; no assertion about unreceived historical sets.',
         'identities':ids,'sources':[ref(base/'eligible.txt')]})
    split=split_files([base/'eligible.txt'],base/'train.txt',base/'val.txt',base/'split.json',
                      plan['training']['validation_ratio'],plan['training']['seed'],64)
    validate_quality(plan, split)
    for x in receipt:
        require(sha(x['path']) == x['sha256'], 'raw shard changed during preparation')


def run(plan_path):
    base = plan_path.resolve().parent
    with (base/'run.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return run_locked(plan_path)


def run_locked(plan_path):
    base = plan_path.resolve().parent
    plan = json.loads(plan_path.read_text())
    name = candidate_name(plan)
    require(not (base/'status.json').exists(), 'fresh run required; automatic restart forbidden')
    state = {'schema':'sekirei.own-halfkp-pilot-status.v1', 'status':'PREFLIGHT',
             'started_at':now(), 'pid':os.getpid(), 'plan':ref(plan_path), 'candidate_id':name,
             'published':False, 'default_changed':False, 'best_remains':'nn_r6',
             'automatic_capacity_resume':True, 'capacity_pause_below_mib':2048,
             'capacity_resume_minimum_mib':3072}
    started = time.monotonic()
    proc = None
    reserve = base/'emergency-reserve.bin'
    def pins():
        require(sha(plan_path) == state['plan']['sha256'], 'frozen plan changed')
        for x in plan['bound_inputs']:
            require(sha(x['path']) == x['sha256'], 'pinned input changed: ' + x['path'])
    def snapshot():
        state['updated_at'] = now()
        state['free_mib'] = shutil.disk_usage(base).free // 1048576
        save_with_reserve(base/'status.json',state,reserve)
    def capacity(name, minimum, paused_child=None):
        previous_status=state['status']
        stable=0
        while stable < 3:
            if paused_child is not None:
                require(paused_child.poll() is None, 'capacity-paused child exited; no retry')
            snapshot()
            cpu=observe_host_cpu()
            admitted=cpu is not None and cpu<=400 and state['free_mib']>=minimum
            stable=stable+1 if admitted else 0
            state.update(status='PAUSED_CAPACITY' if paused_child is not None else 'WAITING_RESOURCES',
                         pending_phase=name,host_total_cpu_percent=cpu,
                         stable_samples=stable,minimum_free_mib=minimum)
            snapshot()
            require(time.monotonic()-started < plan['max_wall_hours']*3600,'capacity wait wall limit')
            if stable<3:
                time.sleep(5)
        state['status']=previous_status
        snapshot()
    def command(argv, name, timeout, min_free=2048):
        nonlocal proc
        pins()
        capacity(name,min_free)
        if not reserve.exists():
            allocate_reserve(reserve)
            capacity(name,min_free)
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
            paused_seconds = 0.0
            while proc.poll() is None:
                snapshot()
                if state['free_mib'] < 2048:
                    previous_status = state['status']
                    def record_pause(action, duration):
                        number = state.get('capacity_event_count', 0) + 1
                        event = {'schema':'sekirei.own-halfkp-capacity-event.v1',
                                 'at':now(), 'action':action, 'phase':name,
                                 'child_pid':proc.pid, 'same_process_continuation':True,
                                 'paused_seconds':duration, 'free_mib':state['free_mib']}
                        path = base/f'capacity-event-{number:04}.json'
                        save_with_reserve(path, event, reserve)
                        state.update(capacity_event_count=number, last_capacity_event=ref(path),
                                     capacity_paused=action == 'paused',
                                     status='PAUSED_CAPACITY' if action == 'paused' else previous_status)
                        if action == 'paused':
                            state['capacity_pause_count'] = state.get('capacity_pause_count', 0) + 1
                        snapshot()
                    def restore_reserve():
                        while not reserve.exists():
                            try:
                                allocate_reserve(reserve)
                            except OSError as error:
                                if error.errno != errno.ENOSPC:
                                    raise
                                capacity(name, 3072, paused_child=proc)
                    reserve.unlink(missing_ok=True)
                    def wait_capacity():
                        capacity(name, max(min_free,3072), paused_child=proc)
                        pins()
                    paused_seconds += pause_child_for_capacity(
                        proc, wait_capacity, restore_reserve, record_pause)
                    pins()
                require(time.monotonic()-phase_started-paused_seconds < timeout, 'phase wall limit')
                require(time.monotonic()-started < plan['max_wall_hours']*3600, 'run wall limit')
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
            code = proc.returncode
            state['last_child_returncode'] = code
            proc = None
            snapshot()
            require(code == 0, 'child failed with return code ' + str(code))
        pins()
    try:
        snapshot(); pins(); validate_plan(plan)
        require(plan['baseline_sha256'] == sha(plan['baseline']), 'wrong nn_r6')
        raw = capacity_retained(plan)
        state['retained_shards'] = [ref(x) for x in raw]
        for n, (seed, games) in enumerate(generation_jobs(plan)):
            state.update(status='GENERATING', shard=n+1, total_shards=len(plan['generation']['seeds']))
            text = base/f'raw-{n:02}.txt'
            require(not text.exists(), 'raw shard already exists; no append/retry')
            require(not (base/'STOP_GENERATION').exists(), 'generation stop file already present')
            g = plan['generation']
            cmd = [plan['generator'],'--out',str(text),'--games',str(games),
                   '--seed',str(seed),'--depth',str(g['depth']),'--nodes',str(g['nodes']),
                   '--random-plies',str(g['random_plies']),'--max-ply',str(g['max_ply']),
                   '--hash-mb','16','--eval',plan['baseline'],'--fv-scale','24',
                   '--stop-file',str(base/'STOP_GENERATION')]
            command(cmd,f'generate-{n:02}',6*3600)
            generation_log=(base/f'generate-{n:02}.stderr').read_text()
            require('(HalfKp)' in generation_log, 'generator evaluator mismatch')
            require(not (base/'STOP_GENERATION').exists() and 'stop file found' not in generation_log,
                    'generation stopped early; frozen game count not completed')
            validate_generated_ids(text,seed,games)
            raw.append(text)
            state['generated_shards'] = [ref(x) for x in raw]
            snapshot()
        state['status']='PREPARING_DATA';snapshot()
        save(base/'generation-receipt.json',state['generated_shards'])
        command([sys.executable,'-u','-B',str(Path(__file__).resolve()),'--plan',str(plan_path),
                 '--prepare-only'],'prepare-data',1800,min_free=2048)
        split=json.loads((base/'split.json').read_text())
        for arm in ['train','val']:
            command([plan['packer'],'pack',str(base/(arm+'.bin')),str(base/(arm+'.txt'))],
                    'pack-'+arm,300,min_free=1536)
            count = split['train' if arm=='train' else 'validation']['positions']
            require((base/(arm+'.bin')).stat().st_size == count*158, 'packed row count mismatch')
        state['status']='TRAINING';snapshot()
        t = plan['training']
        candidate = base/(name+'.bin')
        checkpoint = base/(name+'.pt')
        cmd=[sys.executable,'-u','-B',plan['trainer'],'--data',str(base/'train.bin'),
             '--val-data',str(base/'val.bin'),'--init-bin',plan['baseline'],
             '--out',str(candidate),'--save',str(checkpoint),'--ckpt',str(base/'training.ckpt'),
             '--epochs','1','--batch',str(t['batch']),'--lr',str(t['lr']),
             '--lam','1.0','--scale','400','--fv-scale','24','--device','cpu',
             '--threads','1','--seed',str(t['seed']),'--log-every','10','--ckpt-every','20']
        command(cmd,'training',6*3600,min_free=2048)
        if 'parameter_diff_tool' in plan:
            report = base/'integer-parameter-diff.json'
            command([sys.executable,'-B',plan['parameter_diff_tool'],plan['baseline'],
                     str(candidate),'--output',str(report)],
                    'parameter-diff',300,min_free=768)
            state['integer_parameter_diff'] = ref(report)
        # Load the exported file with the native inference implementation.
        command([plan['packer'],'check',str(candidate),'24',str(base/'val.txt'),'8'],
                'export-load-check',120,min_free=768)
        state.update(status='CANDIDATE_TRAINED_AWAITING_INDEPENDENT_GATE',candidate=ref(candidate),
                     float_checkpoint=ref(checkpoint),completed_at=now(),
                     validation_is_diagnostic_only=True,adopted=False)
        save(base/'candidate.json',dict(state,training_split=ref(base/'split.json')))
        reserve.unlink(missing_ok=True)
        snapshot()
    except BaseException as error:
        if proc is not None:
            cleanup_preserving_error(error, proc)
        cleanup_errors = list(getattr(error, 'cleanup_errors', []))
        try:
            reserve.unlink(missing_ok=True)
        except OSError as cleanup_error:
            cleanup_errors.append({'action':'reserve-unlink',
                                   'error_type':type(cleanup_error).__name__,
                                   'error':str(cleanup_error)})
        state['capacity_paused'] = False
        state.update(status='STOPPED_NOT_QUALIFIED',error_type=type(error).__name__,error=str(error),
                     completed_at=now(),adopted=False)
        if cleanup_errors:
            state['cleanup_errors'] = cleanup_errors
        if getattr(error, 'cleanup_incomplete', False):
            state['cleanup_incomplete'] = True
        try:
            snapshot()
        except OSError as persistence_error:
            # If even the exit journal cannot be written, stderr still retains
            # the primary failure and independently observed cleanup failures.
            try:
                print(json.dumps(dict(state, terminal_persistence_error=str(persistence_error))),
                      file=sys.stderr, flush=True)
            except (OSError, ValueError):
                # Even diagnostic output may share the exhausted filesystem.
                pass
        raise


def interrupted(number, _frame):
    raise PilotShutdown('received signal ' + str(number))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan',type=Path,required=True)
    parser.add_argument('--prepare-only',action='store_true')
    args = parser.parse_args()
    signal.signal(signal.SIGTERM,interrupted)
    signal.signal(signal.SIGINT,interrupted)
    if args.prepare_only:
        prepare_data(json.loads(args.plan.read_text()),args.plan.resolve().parent)
    else:
        run(args.plan)
