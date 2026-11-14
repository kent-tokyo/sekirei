#!/usr/bin/env python3
"""Compare selected moves with a fresh-process teacher, never all-legal regret.

This bounded development diagnostic cannot authorize a match or adoption.
The plan and every input must be pinned before execution. Each teacher move
starts a new process; only complete, repeated scores at one common depth are
compared. Node-limited depths may fall back according to the frozen schedule.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import resource
import shutil
import statistics
import subprocess
import sys
import time

import run_core_floodgate_diagnostic as diagnostic
from generate_search_diagnostic import START


class InvalidSearch(ValueError):
    pass


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def write_json(path: Path, value: object) -> None:
    with path.open('x') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2, sort_keys=True)
        stream.write('\n')


def validate_plan(plan: dict) -> None:
    if (plan.get('schema') != 'sekirei.selected-move-diagnostic-plan.v2'
            or plan.get('status') != 'FROZEN_BEFORE_RUN'
            or plan.get('purpose') != 'development_diagnostic_only'):
        raise ValueError('unsupported or unfrozen plan')
    required = {'engine', 'baseline', 'candidate', 'teacher', 'roots', 'runner', 'helper', 'sfen_codec'}
    if set(plan.get('pins', {})) != required:
        raise ValueError('missing pinned diagnostic inputs')
    config = plan['config']
    required_fv_scale(config)
    bounds = {'time_ms': (1, 1000), 'nodes_per_teacher_call': (1, 1000000),
              'max_roots': (1, 32), 'wall_seconds': (1, 600), 'call_seconds': (1, 30)}
    for key, (low, high) in bounds.items():
        if type(config.get(key)) is not int or not low <= config[key] <= high:
            raise ValueError('invalid bounded ' + key)
    depths = config.get('teacher_depths')
    if (not isinstance(depths, list) or not 1 <= len(depths) <= 3
            or any(type(d) is not int or not 1 <= d <= 32 for d in depths)
            or depths != sorted(set(depths), reverse=True)):
        raise ValueError('teacher depths must be unique and descending')
    if config.get('threads') != 1 or config.get('repeats') != 2:
        raise ValueError('one thread and two teacher repeats required')
    if type(config.get('allowed_node_overrun')) is not int or config['allowed_node_overrun'] not in {0, 1}:
        raise ValueError('explicit zero-or-one node stopping tolerance required')
    if plan['pins']['runner']['sha256'] != sha(Path(__file__)):
        raise ValueError('runner differs from frozen plan')
    for pin in plan['pins'].values():
        if not Path(pin['path']).is_absolute() or sha(Path(pin['path'])) != pin['sha256']:
            raise ValueError('pinned input changed: ' + str(pin.get('path')))
    for key, source in [('helper', Path(diagnostic.__file__)),
                        ('sfen_codec', Path(__file__).with_name('generate_search_diagnostic.py'))]:
        if sha(source) != plan['pins'][key]['sha256']:
            raise ValueError('imported source differs from pinned ' + key)


def required_fv_scale(config: dict) -> int:
    value = config.get('fv_scale')
    if type(value) is not int or not 1 <= value <= 128:
        raise ValueError('explicit fv_scale in 1..=128 required')
    return value


def verify_fv_scale(result: dict, expected: int) -> None:
    if str(result.get('fv_scale')) != str(expected):
        raise InvalidSearch('FV_SCALE was not acknowledged at the registered value')


def valid_search(result: dict, requested_move: str | None = None, *, allow_interrupted_choice: bool = False) -> None:
    if (result.get('completion') != 'search_completed'
            or result.get('completed_bound') != 'exact'
            or result.get('completed_iteration_valid') != 'true'
            or result.get('history_matches_expected') != 'true'
            or result.get('pv_legal') is not True
            or result.get('pv_replay_preserves_input') is not True
            or result.get('abort_reason') not in {'none', 'budget'}
            or not isinstance(result.get('bestmove'), str)
            or result['bestmove'] in {'resign', 'win', 'none'}
            or type(result.get('depth')) is not int or result['depth'] < 1
            or type(result.get('score_cp')) is not int
            or type(result.get('nodes')) is not int or result['nodes'] < 1
            or (requested_move is not None and result['bestmove'] != requested_move)):
        raise InvalidSearch('search lacks complete legal, history-aware result')
    pv = result.get('pv_usi')
    matches = isinstance(pv, str) and bool(pv) and pv.split(',')[0] == result['bestmove']
    if (result.get('bestmove_legal') is False
            or ('pv_starts_with_bestmove' in result and result['pv_starts_with_bestmove'] is not matches)):
        raise InvalidSearch('returned-move legality or PV metadata is inconsistent')
    if not matches and not (allow_interrupted_choice and requested_move is None
                            and result['abort_reason'] == 'budget'
                            and result.get('aborted') in {True, 'true'}
                            and result.get('bestmove_legal') is True):
        raise InvalidSearch('returned move is not tied to a completed PV or independently verified legal interruption')


def verify_node_cap(result: dict, cap: int, allowed_overrun: int) -> None:
    if (result.get('effective_node_cap') != str(cap)
            or type(result.get('nodes')) is not int
            or result['nodes'] > cap + allowed_overrun
            or (result['nodes'] > cap and (result.get('abort_reason') != 'budget'
                                           or result.get('aborted') not in {True, 'true'}))):
        raise InvalidSearch('engine did not acknowledge or respect the explicit node cap')


def teacher_scores(attempts: list[dict], moves: list[str], depth: int) -> tuple[str, dict]:
    """Incomplete budgets can fall back; invalid or inconsistent outputs cannot."""
    by_move = {move: [row['result'] for row in attempts if row['move'] == move] for move in moves}
    if len(attempts) != 2 * len(moves) or any(len(values) != 2 for values in by_move.values()):
        return 'invalid', {}
    for move, values in by_move.items():
        try:
            for value in values:
                valid_search(value, move)
        except InvalidSearch:
            return 'invalid', {}
    if any(value['depth'] > depth for values in by_move.values() for value in values):
        return 'invalid', {}
    scores = {}
    for move, values in by_move.items():
        signature = lambda r: (r['score_cp'], r['depth'], r['nodes'], r.get('pv_usi'), r['abort_reason'])
        if signature(values[0]) != signature(values[1]):
            return 'inconsistent', {}
        scores[move] = values[0]['score_cp']
    if any(value['depth'] < depth and value['abort_reason'] != 'budget' and abs(value['score_cp']) <= 10000
           for values in by_move.values() for value in values):
        return 'invalid', {}
    # A valid early mate observation is nonnumeric, but cannot hide a later
    # move's inconsistent repeats or invalid ordinary-depth completion.
    if any(abs(value) > 10000 for value in scores.values()):
        return 'mate_like', {}
    if any(value['abort_reason'] == 'budget' for values in by_move.values() for value in values):
        return 'budget_incomplete', {}
    return 'complete', scores


def score_coefficients(choices: dict) -> dict[str, float]:
    """Cancel common move frequencies before requesting any teacher score."""
    counts = {arm: Counter(r['bestmove'] for r in rows) for arm, rows in choices.items()}
    differences = {move: counts['candidate'][move] / len(choices['candidate'])
                        - counts['baseline'][move] / len(choices['baseline'])
                   for move in counts['baseline'].keys() | counts['candidate'].keys()}
    return {move: coefficient for move, coefficient in differences.items() if coefficient != 0}


def comparison(choices: dict, scores: dict) -> dict:
    result = {}
    for mode in ['direct', 'timed']:
        coefficients = score_coefficients(choices[mode])
        delta = sum(coefficient * scores[move] for move, coefficient in coefficients.items())
        values = ({arm: [scores[r['bestmove']] for r in choices[mode][arm]]
                   for arm in ['baseline', 'candidate']}
                  if all(r['bestmove'] in scores for rows in choices[mode].values() for r in rows) else None)
        result[mode] = {'selected_scores_cp': values, 'score_difference_coefficients': coefficients,
                        'candidate_minus_baseline_cp': delta,
                        'candidate_worse_ge300cp': delta <= -300,
                        'candidate_better_ge300cp': delta >= 300}
    return result


def run_case(root: dict, config: dict, search) -> dict:
    choices = {'direct': {}, 'timed': {'baseline': [], 'candidate': []}}
    for arm in ['baseline', 'candidate']:
        result = search(root, arm, 'direct', None, 1)
        valid_search(result)
        if result['depth'] != 1 or result.get('abort_reason') != 'none':
            raise InvalidSearch('direct search did not finish depth one')
        choices['direct'][arm] = [result]
    for arm in ['baseline', 'candidate', 'candidate', 'baseline']:
        result = search(root, arm, 'timed', None, None)
        # Measure the actually returned move, even when a later interrupted
        # iteration changed it. Its timed score/PV is not used as its quality.
        valid_search(result, allow_interrupted_choice=True)
        choices['timed'][arm].append(result)
    moves = sorted({move for mode in choices.values() for move in score_coefficients(mode)})
    row = {'identity_sha256': root['identity_sha256'], 'pool': root.get('pool'),
           'choices': choices, 'selected_moves': moves, 'teacher_attempts': [],
           'teacher_depth': None, 'status': 'unresolved', 'comparison': None}
    row['returned_move_differs_from_completed_pv'] = {
        arm: sum(r['pv_usi'].split(',')[0] != r['bestmove'] for r in choices['timed'][arm])
        for arm in ('baseline', 'candidate')}
    row['timed_score_cp_used_for_move_quality'] = False
    if not moves:
        # Identical move frequencies cancel, even if direct and timed choices differ.
        row.update(status='equivalent_selected_choices', comparison=comparison(choices, {}))
        row['absolute_scores_known'] = False
        return row
    for depth in config['teacher_depths']:
        attempts = []
        for repeat, order in enumerate([moves, list(reversed(moves))]):
            for move in order:
                attempts.append({'move': move, 'repeat': repeat,
                                 'result': search(root, 'teacher', 'teacher', move, depth)})
        status, scores = teacher_scores(attempts, moves, depth)
        row['teacher_attempts'].append({'depth': depth, 'status': status, 'calls': attempts})
        if status == 'complete':
            row.update(status='complete', teacher_depth=depth, comparison=comparison(choices, scores),
                       absolute_scores_known=True,
                       teacher_depth_below_timed_depth=depth < max(r['depth'] for v in choices['timed'].values() for r in v))
            break
        if status != 'budget_incomplete':
            break
    return row


def summarize(rows: list[dict], expected: int) -> dict:
    resolved = [r for r in rows if r.get('comparison') is not None]
    result = {'schema': 'sekirei.selected-move-diagnostic-summary.v1',
              'status': 'COMPLETE' if len(resolved) == expected else 'INCOMPLETE',
              'expected_roots': expected, 'recorded_roots': len(rows), 'resolved_roots': len(resolved),
              'unresolved_roots': expected - len(resolved), 'games': 0,
              'match_authorized': False, 'adopted': False, 'strength_claim': False,
              'metric': 'teacher score of selected candidate move minus selected baseline move; positive favors candidate; not regret against all legal moves',
              'complete_batch_metrics': None}
    if len(resolved) != expected:
        return result  # Never hide unresolved roots in an average used for decisions.
    result['complete_batch_metrics'] = {
        mode: {'mean_candidate_minus_baseline_cp': statistics.mean(r['comparison'][mode]['candidate_minus_baseline_cp'] for r in rows),
               'candidate_worse_ge300cp': sum(r['comparison'][mode]['candidate_worse_ge300cp'] for r in rows),
               'candidate_better_ge300cp': sum(r['comparison'][mode]['candidate_better_ge300cp'] for r in rows)}
        for mode in ['direct', 'timed']}
    result['median_timed_nodes_ratio'] = statistics.median(
        statistics.mean(x['nodes'] for x in r['choices']['timed']['candidate']) /
        statistics.mean(x['nodes'] for x in r['choices']['timed']['baseline']) for r in rows)
    result['unstable_timed_roots'] = {arm: sum(r['choices']['timed'][arm][0]['bestmove'] != r['choices']['timed'][arm][1]['bestmove'] for r in rows)
                                      for arm in ['baseline', 'candidate']}
    result['teacher_shallower_than_timed_roots'] = sum(r.get('teacher_depth_below_timed_depth', False) for r in rows)
    return result


def run(plan_path: Path, output: Path) -> dict:
    plan = json.loads(plan_path.read_text())
    validate_plan(plan)
    roots = json.loads(Path(plan['pins']['roots']['path']).read_text())['positions']
    config = plan['config']
    if not 1 <= len(roots) <= config['max_roots'] or len({r['identity_sha256'] for r in roots}) != len(roots):
        raise ValueError('invalid bounded root set')
    for root in roots:
        if root['position'].split()[:2] != ['startpos', 'moves'] or not isinstance(root.get('sfen'), str):
            raise ValueError('full history and expected SFEN required')
    if shutil.disk_usage(output.parent).free < 8 * 1024**3:
        raise ValueError('disk below 8 GiB floor')
    output.mkdir()
    write_json(output / 'plan.json', plan)
    start = time.monotonic()
    rows, calls = [], 0

    def search(root, arm, mode, move, depth):
        nonlocal calls
        remaining = config['wall_seconds'] - (time.monotonic() - start)
        if remaining <= 0:
            raise TimeoutError('diagnostic wall limit')
        nodes = config['nodes_per_teacher_call'] if mode == 'teacher' else 100000
        command = diagnostic.build_command(Path(plan['pins']['engine']['path']), START, nodes,
            Path(plan['pins'][arm]['path']), root_move=move, max_depth=depth,
            history_moves_usi=root['position'].split()[2:], expected_sfen=root['sfen'], nnue_output='absolute')
        command.extend(['--fv-scale', str(required_fv_scale(config))])
        if mode == 'timed':
            command[1:3] = ['--time-ms', str(config['time_ms'])]
        else:
            # Legacy --max-depth disables --nodes; require the explicit combined cap.
            command.extend(['--node-cap', str(nodes)])
        calls += 1
        event = {'call': calls, 'root': root['identity_sha256'], 'arm': arm, 'mode': mode,
                 'requested_move': move, 'requested_depth': depth, 'command': command}
        def record(value):
            with (output / 'calls.jsonl').open('a') as log:
                log.write(json.dumps({**event, **value}, ensure_ascii=False) + '\n')
        record({'phase': 'started'})
        try:
            done = subprocess.run(command, capture_output=True, text=True, timeout=min(remaining, config['call_seconds']),
                                  env={**os.environ, 'RAYON_NUM_THREADS': '1', 'OMP_NUM_THREADS': '1'})
        except (OSError, subprocess.TimeoutExpired) as exc:
            record({'phase': 'failed', 'error': f'{type(exc).__name__}: {exc}'})
            raise
        record({'phase': 'finished', 'returncode': done.returncode, 'stdout': done.stdout, 'stderr': done.stderr})
        if done.returncode:
            raise InvalidSearch('engine exit ' + str(done.returncode) + ': ' + done.stderr[-300:])
        result = diagnostic.parse_result(done.stdout)
        verify_fv_scale(result, config['fv_scale'])
        result['completion'] = 'search_completed'
        if mode != 'timed':
            verify_node_cap(result, nodes, config['allowed_node_overrun'])
        return result

    error = None
    with (output / 'progress.jsonl').open('x') as stream:
        try:
            for root in roots:
                try:
                    row = run_case(root, config, search)
                except (ValueError, subprocess.TimeoutExpired) as exc:
                    row = {'identity_sha256': root['identity_sha256'], 'pool': root.get('pool'),
                           'status': 'unresolved_call_error', 'teacher_depth': None, 'comparison': None,
                           'error': f'{type(exc).__name__}: {exc}',
                           'partial_evidence': 'calls.jsonl'}
                rows.append(row)
                stream.write(json.dumps(row, ensure_ascii=False) + '\n')
                stream.flush()
                print(f"{len(rows)}/{len(roots)} {row['status']} depth={row['teacher_depth']}", flush=True)
                if (output / 'progress.jsonl').stat().st_size > 16 * 1024**2:
                    raise ValueError('diagnostic output limit')
        except (ValueError, OSError, TimeoutError, subprocess.TimeoutExpired) as exc:
            error = f'{type(exc).__name__}: {exc}'
    try:
        validate_plan(plan)
        if sha(plan_path) != sha(output / 'plan.json'):
            # The copied JSON has normalized formatting, so compare semantics too.
            if json.loads(plan_path.read_text()) != plan:
                raise ValueError('input plan changed during execution')
        pins_rechecked = True
    except (ValueError, OSError) as exc:
        pins_rechecked = False
        error = f'{type(exc).__name__}: {exc}'
    summary = summarize(rows, len(roots))
    child_rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    summary.update(plan_sha256=sha(output / 'plan.json'), elapsed_seconds=time.monotonic() - start,
                   calls=calls, peak_child_rss_bytes=child_rss if sys.platform == 'darwin' else child_rss * 1024,
                   resource_scope='measured child peak, not an enforced RSS limit; elapsed cost is descriptive, not a speed benchmark',
                   pinned_inputs_rechecked_after_run=pins_rechecked, error=error)
    if error:
        summary['status'] = 'INCOMPLETE'
        summary['complete_batch_metrics'] = None
    write_json(output / 'summary.json', summary)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = run(args.plan, args.output)
    print(json.dumps(result, ensure_ascii=False))
    return 0 if result['status'] == 'COMPLETE' else 1


if __name__ == '__main__':
    raise SystemExit(main())
