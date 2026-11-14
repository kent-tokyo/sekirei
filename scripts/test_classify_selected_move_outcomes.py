"""Behavioural contracts for CP comparisons and bounded mate exposure."""
import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from classify_selected_move_outcomes import (
    classify_mode,
    mate_probe_moves,
    summarize_typed,
    validated_probe,
)
from test_compare_selected_nnue_moves import result


A, B, C, D = '7g7f', '2g2f', '5g5f', '9g9f'
MAX_PLY, NODE_CAP = 7, 2000


def choices(baseline, candidate):
    return {'baseline': [result(move) for move in baseline],
            'candidate': [result(move) for move in candidate]}


def teacher_row(status='complete', scores=None, *, direct=None, timed=None):
    """Mock already-recorded search evidence; never execute an engine."""
    if scores is None:
        scores = {A: 40000 if status == 'mate_like' else 10, B: 50}
    depth = 12
    calls = []
    for repeat, order in enumerate((list(scores), list(reversed(scores)))):
        for move in order:
            calls.append({'move': move, 'repeat': repeat,
                          'result': result(move, depth=depth - (status == 'budget_incomplete'),
                                           score=scores[move],
                                           abort='budget' if status == 'budget_incomplete' else 'none')})
    return {'identity_sha256': '1' * 64,
            'choices': {'direct': direct if direct is not None else choices([A], [B]),
                        'timed': timed if timed is not None else choices([A, A], [B, B])},
            'selected_moves': sorted(scores),
            'teacher_attempts': [{'depth': depth, 'status': status, 'calls': calls}],
            'comparison': None}


def proof(move, outcome='no_mate', **changes):
    row = {'move': move, 'child_hash': ('1' if move == A else '2') * 16,
           'valid': True, 'outcome': outcome, 'nodes': 100, 'aborted': False,
           'shortest_ply': 5 if outcome == 'mate' else None,
           'solutions': ['5a5b'] if outcome == 'mate' else []}
    row.update(changes)
    return [copy.deepcopy(row), copy.deepcopy(row)]


def classify(row, mode='timed', probes=None):
    return classify_mode(row, mode, probes or {}, max_ply=MAX_PLY, node_cap=NODE_CAP)


def cp(value):
    return {'kind': 'ordinary_cp', 'cp': value}


def exposure(value):
    return {'kind': 'bounded_mate_exposure', 'cp': None,
            'candidate_minus_baseline_exposure': value}


def typed_row(index, timed, direct=None):
    return {'identity_sha256': f'{index:064x}',
            'typed': {'timed': timed, 'direct': direct if direct is not None else cp(0)}}


class SelectedMoveClassificationTests(unittest.TestCase):
    def test_ordinary_cp_uses_repeated_teacher_scores_and_move_frequencies(self):
        row = teacher_row(scores={A: 10, B: 100, C: 30},
                          timed=choices([A, C], [B, B]))
        timed = classify(row)
        direct = classify(row, 'direct')
        self.assertEqual(timed['kind'], 'ordinary_cp')
        self.assertEqual(timed['cp'], 80)
        self.assertEqual(timed['teacher_depth'], 12)
        self.assertEqual(direct['cp'], 90)
        for arm in row['choices']['timed'].values():
            for call in arm:
                call['score_cp'] = -9999
        self.assertEqual(classify(row), timed, 'timed search scores are not teacher labels')

    def test_complete_cp_is_not_replaced_by_bounded_probe_outcomes(self):
        row = teacher_row()
        classified = classify(row, probes={A: proof(A, 'mate'), B: proof(B)})
        self.assertEqual(classified['kind'], 'ordinary_cp')
        self.assertEqual(classified['cp'], 40)
        self.assertEqual(mate_probe_moves(row), [])

    def test_only_valid_incomplete_teacher_evidence_requests_mate_probes(self):
        for status in ('mate_like', 'budget_incomplete'):
            with self.subTest(status=status):
                row = teacher_row(status)
                self.assertEqual(mate_probe_moves(row), sorted([A, B]))
                row['comparison'] = {'timed': {}}
                self.assertEqual(mate_probe_moves(row), [])
        row = teacher_row()
        row['teacher_attempts'] = []
        self.assertEqual(mate_probe_moves(row), [])
        self.assertEqual(classify(row)['reason'], 'no_eligible_teacher_fallback')

    def test_mate_versus_no_mate_is_only_bounded_exposure_with_correct_sign(self):
        row = teacher_row('mate_like')
        less = classify(row, probes={A: proof(A, 'mate'), B: proof(B)})
        more = classify(row, probes={A: proof(A), B: proof(B, 'mate')})
        for classified in (less, more):
            self.assertEqual(classified['kind'], 'bounded_mate_exposure')
            self.assertIsNone(classified['cp'])
            self.assertIn('within 7 plies', classified['scope'])
            self.assertIn('not full game result', classified['scope'])
        self.assertEqual(less['candidate_minus_baseline_exposure'], -1)
        self.assertEqual(more['candidate_minus_baseline_exposure'], 1)
        self.assertEqual(less['outcomes'][B], 'no_checking_mate_within_bound')

    def test_bounded_exposure_preserves_partial_abba_frequencies(self):
        row = teacher_row('budget_incomplete', timed=choices([A, A], [A, B]))
        classified = classify(row, probes={A: proof(A), B: proof(B, 'mate')})
        self.assertEqual(classified['kind'], 'bounded_mate_exposure')
        self.assertEqual(classified['candidate_minus_baseline_exposure'], .5)
        self.assertIsNone(classified['cp'])

    def test_both_no_mate_and_both_mate_leave_move_quality_unresolved(self):
        for outcome in ('no_mate', 'mate'):
            with self.subTest(outcome=outcome):
                row = teacher_row('mate_like')
                classified = classify(row, probes={A: proof(A, outcome), B: proof(B, outcome)})
                self.assertEqual(classified['kind'], 'unresolved')
                self.assertEqual(classified['reason'], 'equal_bounded_exposure_leaves_quality_unknown')
                self.assertIsNone(classified['cp'])
                self.assertNotIn('candidate_minus_baseline_exposure', classified)

    def test_unknown_aborted_invalid_and_missing_probes_stay_unresolved(self):
        row = teacher_row('budget_incomplete')
        bad_probes = [[], proof(B, 'unknown'), proof(B, valid=False),
                      proof(B, aborted=True), proof(B, nodes=NODE_CAP + 1)]
        for values in bad_probes:
            with self.subTest(values=values):
                classified = classify(row, probes={A: proof(A, 'mate'), B: values})
                self.assertEqual(classified['kind'], 'unresolved')
                self.assertEqual(classified['reason'], 'unknown_bounded_mate')
                self.assertEqual(classified['outcomes'][B], 'unknown')
                self.assertIsNone(classified['cp'])

    def test_probe_validation_checks_bound_identity_and_complete_evidence(self):
        self.assertEqual(validated_probe(proof(A, 'mate'), A, MAX_PLY, NODE_CAP), 'mate_within_bound')
        self.assertEqual(validated_probe(proof(A), A, MAX_PLY, NODE_CAP), 'no_checking_mate_within_bound')
        invalid = [
            proof(B), proof(A, valid=1), proof(A, aborted='false'),
            proof(A, nodes=True), proof(A, nodes=-1), proof(A, nodes=NODE_CAP + 1),
            proof(A, child_hash=None), proof(A, child_hash='short'),
            proof(A, shortest_ply=1), proof(A, solutions=['5a5b']),
            proof(A, 'mate', shortest_ply=0), proof(A, 'mate', shortest_ply=2),
            proof(A, 'mate', shortest_ply=MAX_PLY + 2),
            proof(A, 'mate', shortest_ply=True), proof(A, 'mate', solutions=[]),
        ]
        for values in invalid:
            with self.subTest(values=values):
                self.assertEqual(validated_probe(values, A, MAX_PLY, NODE_CAP), 'unknown')
        for values in ([], proof(A)[:1], proof(A) + proof(A)[:1]):
            self.assertEqual(validated_probe(values, A, MAX_PLY, NODE_CAP), 'unknown')

    def test_probe_repeat_disagreement_is_not_averaged_or_selected(self):
        changes = {'move': B, 'child_hash': 'f' * 16, 'valid': False,
                   'outcome': 'unknown', 'nodes': 101, 'aborted': True,
                   'shortest_ply': 3, 'solutions': ['4a4b']}
        for key, value in changes.items():
            with self.subTest(key=key):
                values = proof(A, 'mate')
                values[1][key] = value
                self.assertEqual(validated_probe(values, A, MAX_PLY, NODE_CAP), 'unknown')

    def test_direct_only_unknown_does_not_block_equal_timed_frequencies(self):
        row = teacher_row('budget_incomplete', timed=choices([C, D], [D, C]))
        direct, timed = classify(row, 'direct'), classify(row)
        self.assertEqual(direct['kind'], 'unresolved')
        self.assertEqual(timed, {'kind': 'ordinary_cp', 'cp': 0.0,
                                 'reason': 'equal_move_frequencies'})
        rows = [typed_row(i, cp(10)) for i in range(20)] + [typed_row(20, timed, direct)]
        self.assertTrue(summarize_typed(rows, 21, 20)['advance_to_screen_planning'])

    def test_complete_timed_scores_can_survive_direct_only_budget_uncertainty(self):
        row = teacher_row('budget_incomplete', scores={A: 10, B: 50, C: 80, D: 90},
                          direct=choices([C], [D]))
        for call in row['teacher_attempts'][0]['calls']:
            if call['move'] in (A, B):
                call['result'] = result(call['move'], depth=12, score=10 if call['move'] == A else 50)
        self.assertEqual(classify(row)['cp'], 40)
        self.assertEqual(classify(row, 'direct')['kind'], 'unresolved')

    def test_invalid_teacher_is_not_hidden_by_equal_timed_choices_or_valid_probes(self):
        for change in ({'pv_legal': False}, {'history_matches_expected': 'false'},
                       {'bestmove': C}, {'completed_bound': 'lower'},
                       {'abort_reason': 'external_stop'}):
            with self.subTest(change=change):
                row = teacher_row('budget_incomplete', timed=choices([C, D], [D, C]))
                row['teacher_attempts'][0]['calls'][0]['result'].update(change)
                self.assertEqual(mate_probe_moves(row), [])
                classified = classify(row, probes={A: proof(A, 'mate'), B: proof(B)})
                self.assertEqual(classified['kind'], 'unresolved')
                self.assertEqual(classified['reason'], 'invalid_teacher_evidence')

    def test_teacher_repeat_disagreement_and_claimed_status_mismatch_block(self):
        row = teacher_row('mate_like')
        row['teacher_attempts'][0]['calls'][-1]['result']['score_cp'] += 1
        self.assertEqual(mate_probe_moves(row), [])
        self.assertEqual(classify(row)['reason'], 'invalid_teacher_evidence')
        row = teacher_row()
        row['teacher_attempts'][0]['status'] = 'budget_incomplete'
        self.assertEqual(classify(row)['reason'], 'invalid_teacher_evidence')

    def test_invalid_earlier_teacher_cannot_be_hidden_by_complete_later_depth(self):
        row = teacher_row()
        earlier = copy.deepcopy(row['teacher_attempts'][0])
        earlier['calls'][0]['result']['pv_legal'] = False
        earlier['status'] = 'invalid'
        row['teacher_attempts'].insert(0, earlier)
        self.assertEqual(classify(row)['reason'], 'invalid_teacher_evidence')

    def test_missing_mode_remains_unresolved(self):
        self.assertEqual(classify({'choices': {}})['reason'], 'missing_choices')
        row = teacher_row()
        del row['choices']['timed']
        self.assertEqual(classify(row)['reason'], 'missing_choices')


class TypedSummaryTests(unittest.TestCase):
    def test_positive_ordinary_diagnostic_only_authorizes_screen_planning(self):
        rows = [typed_row(i, cp(10)) for i in range(24)]
        summary = summarize_typed(rows, 24, 20)
        self.assertEqual(summary['status'], 'PROMISING_FOR_SCREEN_PLANNING')
        self.assertTrue(summary['advance_to_screen_planning'])
        self.assertEqual(summary['ordinary']['count'], 24)
        self.assertEqual(summary['ordinary']['mean_cp'], 10)
        for name in ('adopted', 'match_authorized', 'strength_claim', 'cp_and_mate_pooled'):
            self.assertFalse(summary[name])

    def test_candidate_mate_hazard_increase_blocks_even_with_positive_cp(self):
        rows = [typed_row(i, cp(10)) for i in range(20)]
        decreased = summarize_typed(rows + [typed_row(20, exposure(-.5))], 21, 20)
        increased = summarize_typed(rows + [typed_row(20, exposure(.5))], 21, 20)
        self.assertTrue(decreased['advance_to_screen_planning'])
        self.assertEqual(decreased['bounded_exposure_decreased'], 1)
        self.assertFalse(increased['advance_to_screen_planning'])
        self.assertEqual(increased['bounded_exposure_increased'], 1)
        for summary in (decreased, increased):
            self.assertEqual(summary['ordinary']['count'], 20)
            self.assertEqual(summary['ordinary']['mean_cp'], 10)
            self.assertFalse(summary['cp_and_mate_pooled'])

    def test_unresolved_roots_are_retained_and_block_advancement(self):
        rows = [typed_row(i, cp(10)) for i in range(23)]
        rows.append(typed_row(23, {'kind': 'unresolved', 'cp': None,
                                   'reason': 'equal_bounded_exposure_leaves_quality_unknown'}))
        summary = summarize_typed(rows, 24, 20)
        self.assertEqual(summary['ordinary']['count'], 23)
        self.assertEqual(summary['unresolved_roots'], 1)
        self.assertFalse(summary['advance_to_screen_planning'])

    def test_at_least_twenty_ordinary_roots_are_required_prospectively(self):
        rows = [typed_row(i, cp(10)) for i in range(19)]
        rows += [typed_row(i, exposure(-1)) for i in range(19, 24)]
        summary = summarize_typed(rows, 24, 20)
        self.assertEqual(summary['ordinary']['count'], 19)
        self.assertEqual(summary['unresolved_roots'], 0)
        self.assertFalse(summary['advance_to_screen_planning'])

    def test_one_large_gain_cannot_hide_negative_remaining_mean(self):
        values = [1000] + [-10] * 12 + [1] * 11
        summary = summarize_typed([typed_row(i, cp(v)) for i, v in enumerate(values)], 24, 20)
        self.assertGreater(summary['ordinary']['mean_cp'], 0)
        self.assertEqual(summary['ordinary']['improved'], summary['ordinary']['worsened'])
        self.assertEqual(summary['ordinary']['major_worsening'], 0)
        self.assertLess(summary['ordinary']['without_largest_gain_mean_cp'], 0)
        self.assertFalse(summary['advance_to_screen_planning'])

    def test_a_major_ordinary_loss_blocks_despite_broad_positive_mean(self):
        values = [50] * 23 + [-300]
        summary = summarize_typed([typed_row(i, cp(v)) for i, v in enumerate(values)], 24, 20)
        self.assertGreater(summary['ordinary']['without_largest_gain_mean_cp'], 0)
        self.assertEqual(summary['ordinary']['major_worsening'], 1)
        self.assertFalse(summary['advance_to_screen_planning'])

    def test_nonpositive_mean_or_more_losses_cannot_advance(self):
        for values in ([0] * 24, [1] * 12 + [-1] * 12, [10] * 10 + [-1] * 14):
            with self.subTest(values=values):
                summary = summarize_typed([typed_row(i, cp(v)) for i, v in enumerate(values)], 24, 20)
                self.assertFalse(summary['advance_to_screen_planning'])

    def test_missing_duplicate_and_surplus_root_identities_block(self):
        good = [typed_row(i, cp(10)) for i in range(24)]
        missing = summarize_typed(good[:-1], 24, 20)
        self.assertEqual(missing['unresolved_roots'], 1)
        self.assertFalse(missing['identity_integrity'])
        self.assertFalse(missing['advance_to_screen_planning'])
        duplicate = copy.deepcopy(good)
        duplicate[-1]['identity_sha256'] = duplicate[0]['identity_sha256']
        for rows in (duplicate, good + [typed_row(24, cp(10))]):
            with self.subTest(identities=[row['identity_sha256'] for row in rows]):
                summary = summarize_typed(rows, 24, 20)
                self.assertFalse(summary['identity_integrity'])
                self.assertFalse(summary['advance_to_screen_planning'])

    def test_empty_evidence_has_no_synthetic_cp_mean(self):
        summary = summarize_typed([], 24, 20)
        self.assertEqual(summary['unresolved_roots'], 24)
        self.assertEqual(summary['ordinary']['count'], 0)
        self.assertIsNone(summary['ordinary']['mean_cp'])
        self.assertIsNone(summary['ordinary']['without_largest_gain_mean_cp'])
        self.assertFalse(summary['advance_to_screen_planning'])


if __name__ == '__main__':
    unittest.main()
