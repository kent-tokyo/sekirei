import unittest
from copy import deepcopy
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from nnue_screen_robustness import analyze, delta_summary, teacher_depth_sensitivity


def row(identity, value, pool='early'):
    comparison = {mode: {'candidate_minus_baseline_cp': value} for mode in ('direct', 'timed')}
    return {'identity_sha256': identity, 'pool': pool, 'status': 'complete', 'comparison': comparison}


class RobustnessTests(unittest.TestCase):
    def test_one_large_gain_can_hide_many_losses(self):
        result = delta_summary([387, 128, 167, -57, -30, -73, -74, -253] + [0] * 13)
        self.assertAlmostEqual(result['mean_cp'], 195 / 21)
        self.assertEqual(result['median_cp'], 0)
        self.assertEqual(result['mean_without_largest_gain_cp'], -9.6)
        self.assertEqual(result['worse_roots'], 5)
        self.assertEqual(result['losses_ge_cp'], {'100': 1, '200': 1, '300': 0})

    def test_no_false_strength_flag_or_imputed_missing_position(self):
        rows = [row('a', 100), {'identity_sha256': 'b', 'pool': 'late', 'status': 'mate_like', 'comparison': None}]
        result = analyze(rows, 2, 'candidate', 'baseline')
        self.assertEqual(result['metrics']['timed']['mean_cp'], 100)
        self.assertEqual(result['resolved_roots'], 1)
        self.assertFalse(result['coverage_complete'])
        self.assertFalse(result['match_authorized'])
        self.assertFalse(result['strength_claim'])
        self.assertEqual(result['unresolved_roots'][0]['identity_sha256'], 'b')

    def test_singleton_does_not_invent_leave_one_out(self):
        result = delta_summary([10])
        self.assertIsNone(result['mean_without_largest_gain_cp'])
        self.assertIsNone(result['leave_one_out_mean_min_cp'])

    def test_zero_or_negative_panel_has_no_invented_gain_fraction(self):
        for values in ([0, 0], [-3, -1]):
            result = delta_summary(values)
            self.assertIsNone(result['largest_gain_fraction_of_positive_sum'])
            self.assertIsNone(result['mean_without_largest_gain_cp'])

    def test_invalid_numbers_counts_and_duplicates_are_rejected(self):
        for values in ([], [float('nan')], [float('inf')], [True], ['3']):
            with self.assertRaises(ValueError): delta_summary(values)
        for rows, count in (([row('a', 2)], 2), ([row('a', 2), row('a', 3)], 2)):
            with self.assertRaises(ValueError): analyze(rows, count, 'candidate', 'baseline')

    def test_shared_table_and_pools_remain_comparable(self):
        rows = [row('a', 10, 'early'), row('b', -20, 'late')]
        for r in rows: r['comparisons'] = {'c': {'r': r.pop('comparison')}}
        result = analyze(rows, 2, 'c', 'r')
        self.assertEqual(result['metrics']['timed']['mean_cp'], -5)
        self.assertEqual(result['by_pool']['late']['timed']['mean_cp'], -20)
        with self.assertRaises(KeyError): analyze(rows, 2, 'other', 'r')

    def test_numeric_mate_row_cannot_look_resolved(self):
        r = row('a', 900000)
        r['status'] = 'mate_like'
        with self.assertRaises(ValueError): analyze([r], 1, 'candidate', 'baseline')


def attempt(depth, values):
    return {'depth': depth, 'status': 'budget_incomplete', 'calls': [
        {'move': move, 'repeat': repeat,
         'result': {'bestmove': move, 'depth': depth, 'score_cp': score,
                    'completed_bound': 'exact', 'completed_iteration_valid': 'true',
                    'pv_replay_preserves_input': True, 'history_matches_expected': 'true',
                    'completion': 'search_completed',
                    'pv_legal': True, 'aborted': 'false', 'abort_reason': 'none'}}
        for move, score in values.items() for repeat in (0, 1)]}


class DepthSensitivityTests(unittest.TestCase):
    def fixture(self):
        r = row('known', 357.5)
        r['teacher_attempts'] = [attempt(8, {'2e4e': -1256, 'B*3e': -732}),
                                 attempt(12, {'2e4e': -901, 'B*3e': -1268})]
        return r

    def test_completed_pair_survives_third_move_budget_failure(self):
        r = self.fixture()
        failed = attempt(12, {'7c6e': -1216})['calls']
        for c in failed:
            c['result'].update(depth=11, aborted='true', abort_reason='budget')
        r['teacher_attempts'][1]['calls'].extend(failed)
        result = analyze([r], 1, 'candidate', 'baseline')
        sensitivity = result['teacher_depth_sensitivity']
        self.assertEqual(sensitivity['reversal_pairs'], 1)
        pair = sensitivity['roots'][0]['pairs'][0]
        self.assertEqual([v['b_minus_a_cp'] for v in pair['depths']], [524, -367])
        self.assertEqual(result['metrics']['timed']['mean_cp'], 357.5)
        self.assertFalse(result['old_verdict_changed'])
        self.assertFalse(result['match_authorized'])

    def test_incomplete_invalid_mate_and_mismatched_calls_are_excluded(self):
        for bad in ({'aborted':'true'}, {'depth':11}, {'bestmove':'other'},
                    {'pv_legal':False}, {'completed_bound':'lower'},
                    {'pv_replay_preserves_input':False}, {'history_matches_expected':'false'},
                    {'completion':'history_error'}, {'aborted':0},
                    {'completed_iteration_valid':'false'}, {'score_cp':899981},
                    {'score_cp':float('nan')}, {'score_cp':True}):
            r = self.fixture()
            r['teacher_attempts'][1]['calls'][0]['result'].update(bad)
            self.assertEqual(teacher_depth_sensitivity([r])['roots'], [], bad)

    def test_unstable_or_missing_repeats_do_not_invent_a_reversal(self):
        r = self.fixture()
        r['teacher_attempts'][1]['calls'][0]['result']['score_cp'] += 1
        self.assertEqual(teacher_depth_sensitivity([r])['roots'], [])
        r = self.fixture()
        r['teacher_attempts'][1]['calls'].pop()
        self.assertEqual(teacher_depth_sensitivity([r])['roots'], [])

    def test_duplicate_requested_depth_is_not_selected_posthoc(self):
        r = self.fixture()
        r['teacher_attempts'].append(deepcopy(r['teacher_attempts'][1]))
        self.assertEqual(teacher_depth_sensitivity([r])['roots'], [])

    def test_legacy_records_and_nonreversal_are_supported(self):
        self.assertEqual(teacher_depth_sensitivity([row('old', 0)])['roots'], [])
        r = self.fixture()
        r['teacher_attempts'][1] = attempt(12, {'2e4e': -901, 'B*3e': -800})
        result = teacher_depth_sensitivity([r])
        self.assertEqual(len(result['roots']), 1)
        self.assertEqual(result['reversal_roots'], 0)


if __name__ == '__main__': unittest.main()
