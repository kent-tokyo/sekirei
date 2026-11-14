#!/usr/bin/env python3
"""Describe concentration in completed NNUE move diagnostics; never issue a gate.

Read existing per-position records only. Unresolved positions stay explicit;
leave-one-out numbers are sensitivity descriptions, not replacement results.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
from itertools import combinations
import json
import math
from pathlib import Path
import statistics


def teacher_depth_sensitivity(rows: list[dict]) -> dict:
    """Compare completed, repeat-stable move pairs across requested depths.

    A third move can exhaust its budget without discarding the other two
    completed calls. These pairwise facts never replace the panel's selected
    common depth, mean, unresolved status, or registered verdict.
    """
    def flag(value, expected):
        return value is expected or type(value) is str and value == ('true' if expected else 'false')

    findings = []
    for row in rows:
        depths = {}
        for attempt in row.get('teacher_attempts', []):
            depth = attempt.get('depth')
            if type(depth) is not int or depth < 1:
                continue
            calls_by_move = {}
            for call in attempt.get('calls', []):
                move = call.get('move')
                if isinstance(move, str) and move:
                    calls_by_move.setdefault(move, []).append(call)
            stable = {}
            for move, calls in calls_by_move.items():
                if (len(calls) != 2 or any(type(c.get('repeat')) is not int for c in calls)
                        or {c.get('repeat') for c in calls} != {0, 1}):
                    continue
                scores = []
                for call in calls:
                    result = call.get('result') or {}
                    score = result.get('score_cp')
                    if (type(score) not in (int, float) or not math.isfinite(score)
                            or abs(score) >= 800000 or type(result.get('depth')) is not int
                            or result.get('depth') != depth
                            or result.get('bestmove') != move
                            or result.get('completed_bound') != 'exact'
                            or not flag(result.get('completed_iteration_valid'), True)
                            or result.get('pv_legal') is not True
                            or result.get('pv_replay_preserves_input') is not True
                            or not flag(result.get('history_matches_expected'), True)
                            or result.get('completion') != 'search_completed'
                            or not flag(result.get('aborted'), False)
                            or result.get('abort_reason') != 'none'):
                        break
                    scores.append(score)
                if len(scores) == 2 and scores[0] == scores[1]:
                    stable[move] = scores[0]
            # Duplicate requested depths are ambiguous; never choose a repeat.
            depths[depth] = stable if depth not in depths else {}
        pairs = set(pair for moves in depths.values() for pair in combinations(sorted(moves), 2))
        comparisons = []
        for first, second in sorted(pairs):
            samples = [{'depth': depth, 'a_cp': scores[first], 'b_cp': scores[second],
                        'b_minus_a_cp': scores[second] - scores[first]}
                       for depth, scores in sorted(depths.items()) if first in scores and second in scores]
            if len(samples) < 2:
                continue
            differences = [sample['b_minus_a_cp'] for sample in samples]
            comparisons.append({'move_a': first, 'move_b': second, 'depths': samples,
                                'ranking_reversal': min(differences) < 0 < max(differences)})
        if comparisons:
            findings.append({'identity_sha256': row['identity_sha256'], 'pairs': comparisons})
    return {'scope': 'Descriptive completed move pairs, two stable repeats at each requested depth; not a replacement panel result.',
            'roots': findings,
            'reversal_roots': sum(any(p['ranking_reversal'] for p in r['pairs']) for r in findings),
            'reversal_pairs': sum(p['ranking_reversal'] for r in findings for p in r['pairs'])}


def delta_summary(values: list[float]) -> dict:
    if not values or any(type(v) not in (int, float) or not math.isfinite(v) for v in values):
        raise ValueError('requires nonempty finite numeric deltas')
    n = len(values)
    total = math.fsum(values)
    leave_one_out = [(total - v) / (n - 1) for v in values] if n > 1 else []
    positive = math.fsum(v for v in values if v > 0)
    return {'roots': n, 'sum_cp': total, 'mean_cp': total / n,
            'median_cp': statistics.median(values), 'min_cp': min(values), 'max_cp': max(values),
            'better_roots': sum(v > 0 for v in values), 'worse_roots': sum(v < 0 for v in values),
            'equal_roots': sum(v == 0 for v in values),
            'positive_sum_cp': positive, 'negative_sum_cp': math.fsum(v for v in values if v < 0),
            'largest_gain_fraction_of_positive_sum': max(values) / positive if positive else None,
            'mean_without_largest_gain_cp': (total - max(values)) / (n - 1) if n > 1 and max(values) > 0 else None,
            'leave_one_out_mean_min_cp': min(leave_one_out) if leave_one_out else None,
            'leave_one_out_mean_max_cp': max(leave_one_out) if leave_one_out else None,
            'leave_one_out_positive_means': sum(v > 0 for v in leave_one_out),
            'losses_ge_cp': {str(t): sum(v <= -t for v in values) for t in (100, 200, 300)},
            'gains_ge_cp': {str(t): sum(v >= t for v in values) for t in (100, 200, 300)}}


def analyze(rows: list[dict], expected: int, candidate: str, reference: str) -> dict:
    if type(expected) is not int or expected < 1 or len(rows) != expected:
        raise ValueError('recorded root count differs from declared panel size')
    identities = [r['identity_sha256'] for r in rows]
    if any(not isinstance(k, str) or not k for k in identities) or len(set(identities)) != expected:
        raise ValueError('missing or duplicate root identities')
    resolved, unresolved = [], []
    for row in rows:
        # Support both the public two-arm diagnostic and the shared-table screen.
        if 'comparisons' in row:
            comparisons = row['comparisons']
            comparison = comparisons[candidate][reference] if comparisons is not None else None
        else:
            comparison = row.get('comparison')
        if comparison is None:
            unresolved.append({'identity_sha256': row['identity_sha256'],
                               'pool': row.get('pool'), 'status': row['status']})
            continue
        if row['status'] not in {'complete', 'equivalent_choices', 'equivalent_selected_choices'}:
            raise ValueError('numeric comparisons attached to an unresolved root')
        resolved.append({'identity_sha256': row['identity_sha256'], 'pool': row.get('pool'),
                         **{mode: comparison[mode]['candidate_minus_baseline_cp'] for mode in ('direct', 'timed')}})
    metrics = {mode: delta_summary([r[mode] for r in resolved]) for mode in ('direct', 'timed')} if resolved else None
    pools = sorted({str(r['pool']) for r in resolved})
    return {'schema': 'sekirei.nnue-screen-robustness.v1', 'status': 'DESCRIPTIVE_ONLY',
            'candidate': candidate, 'reference': reference, 'expected_roots': expected,
            'resolved_roots': len(resolved), 'unresolved_roots': unresolved,
            'coverage_complete': len(resolved) == expected,
            'metrics_scope': 'same resolved rows only; unresolved rows are never scored zero',
            'metrics': metrics,
            'by_pool': {pool: {mode: delta_summary([r[mode] for r in resolved if str(r['pool']) == pool])
                               for mode in ('direct', 'timed')} for pool in pools},
            'rows': resolved, 'unresolved_status_counts': dict(Counter(r['status'] for r in unresolved)),
            'teacher_depth_sensitivity': teacher_depth_sensitivity(rows),
            'match_authorized': False, 'adopted': False, 'strength_claim': False,
            'old_verdict_changed': False,
            'interpretation': 'Sensitivity is not a confidence interval, new pass threshold, or corrected historical result.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--progress', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expected-roots', type=int, required=True)
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--reference', required=True)
    args = parser.parse_args()
    payload = args.progress.read_bytes()
    rows = [json.loads(line) for line in payload.splitlines()]
    result = analyze(rows, args.expected_roots, args.candidate, args.reference)
    result['input'] = {'path': str(args.progress.resolve()), 'sha256': hashlib.sha256(payload).hexdigest()}
    with args.output.open('x') as out:
        json.dump(result, out, indent=2, allow_nan=False)
        out.write('\n')
    print(json.dumps({k: result[k] for k in ('resolved_roots', 'coverage_complete', 'metrics', 'by_pool')}))


if __name__ == '__main__':
    main()
