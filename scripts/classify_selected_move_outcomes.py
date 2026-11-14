"""Keep ordinary selected-move scores and bounded mate exposure separate.

This is a development diagnostic, not a game-result oracle. NoMate only
excludes checking mates within the supplied bound. It never supplies a CP
score or resolves an ordinary comparison on its own.
"""
from __future__ import annotations

import math
import statistics

from compare_selected_nnue_moves import score_coefficients, teacher_scores


def mate_probe_moves(row: dict) -> list[str]:
    """Only valid, repeatable, but incomplete teacher evidence can use probes."""
    attempts = row.get('teacher_attempts', [])
    if not attempts or row.get('comparison') is not None:
        return []
    last = attempts[-1]
    status, _ = teacher_scores(last['calls'], row['selected_moves'], last['depth'])
    if status not in {'mate_like', 'budget_incomplete'}:
        return []
    return sorted({move for choices in row['choices'].values()
                   for move in score_coefficients(choices)})


def validated_probe(values: list[dict], move: str, max_ply: int, node_cap: int) -> str:
    """Reject missing, invalid, over-budget, or nonrepeatable proof results."""
    if len(values) != 2:
        return 'unknown'
    fields = ('move', 'child_hash', 'valid', 'outcome', 'nodes', 'aborted', 'shortest_ply', 'solutions')
    if any(tuple(v.get(k) for k in fields) != tuple(values[0].get(k) for k in fields) for v in values):
        return 'unknown'
    value = values[0]
    if (value.get('move') != move or value.get('valid') is not True
            or value.get('aborted') is not False
            or type(value.get('nodes')) is not int or not 0 <= value['nodes'] <= node_cap
            or not isinstance(value.get('child_hash'), str) or len(value['child_hash']) != 16):
        return 'unknown'
    outcome, distance, solutions = value.get('outcome'), value.get('shortest_ply'), value.get('solutions')
    if outcome == 'mate' and type(distance) is int and 1 <= distance <= max_ply and distance % 2 == 1 and isinstance(solutions, list) and solutions:
        return 'mate_within_bound'
    if outcome == 'no_mate' and distance is None and solutions == []:
        return 'no_checking_mate_within_bound'
    return 'unknown'


def classify_mode(row: dict, mode: str, probes: dict, *, max_ply: int, node_cap: int) -> dict:
    """Classify one mode without allowing direct-only uncertainty to mask timed data."""
    unresolved = {'kind': 'unresolved', 'cp': None}
    choices = row.get('choices')
    if not choices or mode not in choices:
        return {**unresolved, 'reason': 'missing_choices'}
    attempts = row.get('teacher_attempts', [])
    # Invalid or inconsistent teachers are an integrity failure even when the
    # affected move would cancel in this mode.
    for attempt in attempts:
        status, _ = teacher_scores(attempt['calls'], row['selected_moves'], attempt['depth'])
        if status in {'invalid', 'inconsistent'} or status != attempt['status']:
            return {**unresolved, 'reason': 'invalid_teacher_evidence'}
    coefficients = score_coefficients(choices[mode])
    if not coefficients:
        return {'kind': 'ordinary_cp', 'cp': 0.0, 'reason': 'equal_move_frequencies'}
    # Use only repeated complete scores at a common registered depth. Scores
    # for a different mode are irrelevant, but malformed evidence above is not.
    for attempt in attempts:
        calls = [call for call in attempt['calls'] if call['move'] in coefficients]
        status, scores = teacher_scores(calls, sorted(coefficients), attempt['depth'])
        if status == 'complete':
            delta = sum(weight * scores[move] for move, weight in coefficients.items())
            if not math.isfinite(delta):
                return {**unresolved, 'reason': 'nonfinite_score'}
            return {'kind': 'ordinary_cp', 'cp': delta, 'teacher_depth': attempt['depth']}
    allowed = set(mate_probe_moves(row))
    if not set(coefficients) <= allowed:
        return {**unresolved, 'reason': 'no_eligible_teacher_fallback'}
    outcomes = {move: validated_probe(probes.get(move, []), move, max_ply, node_cap) for move in coefficients}
    if 'unknown' in outcomes.values():
        return {**unresolved, 'reason': 'unknown_bounded_mate', 'outcomes': outcomes}
    # Positive means the candidate exposes its king to short checking mates
    # more often. This is a frequency delta, never a utility or CP estimate.
    exposure = sum(weight * (outcomes[move] == 'mate_within_bound') for move, weight in coefficients.items())
    if exposure == 0:
        return {**unresolved, 'reason': 'equal_bounded_exposure_leaves_quality_unknown', 'outcomes': outcomes}
    return {'kind': 'bounded_mate_exposure', 'cp': None,
            'candidate_minus_baseline_exposure': exposure, 'outcomes': outcomes,
            'scope': f'opponent checking mate within {max_ply} plies after selected move; board only, not full game result'}


def summarize_typed(rows: list[dict], expected: int, min_cp_roots: int) -> dict:
    """A positive diagnostic can advance planning, never adoption or a match."""
    timed = [row['typed']['timed'] for row in rows]
    values = [row['cp'] for row in timed if row['kind'] == 'ordinary_cp']
    hazards = [row['candidate_minus_baseline_exposure'] for row in timed if row['kind'] == 'bounded_mate_exposure']
    unresolved = sum(row['kind'] == 'unresolved' for row in timed) + max(0, expected - len(rows))
    ordinary = {'count': len(values), 'values': values,
                'mean_cp': statistics.mean(values) if values else None,
                'without_largest_gain_mean_cp': (sum(values) - max(values)) / (len(values) - 1) if len(values) > 1 else None,
                'improved': sum(v > 0 for v in values), 'worsened': sum(v < 0 for v in values),
                'equal': sum(v == 0 for v in values), 'major_worsening': sum(v <= -300 for v in values)}
    integrity = len(rows) == expected and len({row['identity_sha256'] for row in rows}) == expected
    advance = (integrity and unresolved == 0 and len(values) >= min_cp_roots
               and ordinary['mean_cp'] > 0 and ordinary['without_largest_gain_mean_cp'] >= 0
               and ordinary['improved'] >= ordinary['worsened'] and ordinary['major_worsening'] == 0
               and not any(value > 0 for value in hazards))
    return {'status': 'PROMISING_FOR_SCREEN_PLANNING' if advance else 'DO_NOT_ADVANCE',
            'expected_roots': expected, 'recorded_roots': len(rows), 'ordinary': ordinary,
            'bounded_exposure_decreased': sum(v < 0 for v in hazards),
            'bounded_exposure_increased': sum(v > 0 for v in hazards),
            'unresolved_roots': unresolved, 'identity_integrity': integrity,
            'advance_to_screen_planning': advance, 'adopted': False, 'match_authorized': False,
            'strength_claim': False, 'cp_and_mate_pooled': False}
