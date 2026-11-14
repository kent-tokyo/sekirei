"""Behavioural checks for bounded, comparable selected-move teacher evidence."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from compare_selected_nnue_moves import InvalidSearch, comparison, run_case, summarize, teacher_scores, valid_search, verify_node_cap, score_coefficients
import compare_selected_nnue_moves as runner


def result(move='7g7f', depth=20, score=10, nodes=100, abort='none'):
    return {'completion': 'search_completed', 'bestmove': move, 'depth': depth,
            'score_cp': score, 'nodes': nodes, 'abort_reason': abort,
            'aborted': 'false' if abort == 'none' else 'true',
            'completed_bound': 'exact', 'completed_iteration_valid': 'true',
            'history_matches_expected': 'true', 'pv_legal': True,
            'pv_replay_preserves_input': True, 'pv_usi': move}


class SelectedMoveTest(unittest.TestCase):
    def test_scale_must_be_registered_and_acknowledged(self):
        self.assertEqual(runner.required_fv_scale({'fv_scale':24}),24)
        for value in [None, 0, 129, True, '24', 24.0]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                runner.required_fv_scale({'fv_scale':value})
        for result in [{}, {'fv_scale':'16'}, {'fv_scale':None}]:
            with self.subTest(result=result), self.assertRaises(InvalidSearch):
                runner.verify_fv_scale(result,24)
        runner.verify_fv_scale({'fv_scale':'24'},24)
        runner.verify_fv_scale({'fv_scale':24},24)

    def test_interrupted_choice_needs_separate_legality_and_is_not_a_completed_score(self):
        row = {**result('2g2f', abort='budget'), 'pv_usi': '7g7f,3c3d',
               'bestmove_legal': True, 'pv_starts_with_bestmove': False}
        valid_search(row, allow_interrupted_choice=True)
        with self.assertRaises(InvalidSearch):
            valid_search(row)
        with self.assertRaises(InvalidSearch):
            valid_search(row, '2g2f', allow_interrupted_choice=True)
        for change in ({'bestmove_legal': False}, {'bestmove_legal': None},
                       {'abort_reason': 'none'}, {'pv_starts_with_bestmove': True}):
            with self.subTest(change=change), self.assertRaises(InvalidSearch):
                valid_search({**row, **change}, allow_interrupted_choice=True)

    def test_pv_legality_does_not_override_an_illegal_returned_move(self):
        with self.assertRaises(InvalidSearch):
            valid_search({**result(), 'bestmove_legal': False})

    def test_timed_choice_quality_uses_the_returned_move_not_its_stale_score_or_pv(self):
        def search(root, arm, mode, move, depth):
            if mode == 'teacher':
                return result(move, depth=depth, score=90 if move=='2g2f' else 10)
            if mode == 'timed' and arm == 'candidate':
                return {**result('2g2f', score=-999, abort='budget'), 'pv_usi':'7g7f',
                        'bestmove_legal':True, 'pv_starts_with_bestmove':False}
            return result(depth=depth or 10)
        row=run_case({'identity_sha256':'x'}, {'teacher_depths':[20]}, search)
        self.assertEqual(row['comparison']['timed']['candidate_minus_baseline_cp'],80)
        self.assertEqual(row['returned_move_differs_from_completed_pv']['candidate'],2)
        changed=copy.deepcopy(row['choices'])
        for result_row in changed['timed']['candidate']:
            result_row['score_cp']=12345
        self.assertEqual(comparison(changed,{'7g7f':10,'2g2f':90}),row['comparison'])

    def test_explicit_node_cap_tolerance_is_small_and_requires_budget_stop(self):
        row = {**result(nodes=101, abort='budget'), 'effective_node_cap': '100'}
        verify_node_cap(row, 100, 1)
        for changed, tolerance in [(row, 0), ({**row, 'nodes': 102}, 1),
                                   ({**row, 'abort_reason': 'none'}, 1),
                                   ({**row, 'aborted': 'false'}, 1),
                                   ({**row, 'effective_node_cap': 'none'}, 1)]:
            with self.assertRaises(InvalidSearch):
                verify_node_cap(changed, 100, tolerance)

    def test_missing_history_illegal_pv_wrong_root_and_lower_bound_are_rejected(self):
        for key, value in [('history_matches_expected', 'false'), ('pv_legal', False),
                           ('pv_replay_preserves_input', False), ('completed_bound', 'lower'),
                           ('completed_iteration_valid', 'false'), ('nodes', 0), ('bestmove', 'resign')]:
            row = result(); row[key] = value
            with self.subTest(key=key), self.assertRaises(InvalidSearch):
                valid_search(row)
        with self.assertRaises(InvalidSearch):
            valid_search(result(), '2g2f')

    def test_common_depth_is_required_and_budget_only_can_fall_back(self):
        calls = [{'move': move, 'result': result(move, depth=depth, abort=abort)}
                 for move, depth, abort in [('7g7f', 20, 'none'), ('7g7f', 20, 'none'),
                                            ('2g2f', 18, 'budget'), ('2g2f', 18, 'budget')]]
        self.assertEqual(teacher_scores(calls, ['7g7f', '2g2f'], 20)[0], 'budget_incomplete')
        calls[-1]['result']['pv_legal'] = False
        self.assertEqual(teacher_scores(calls, ['7g7f', '2g2f'], 20)[0], 'invalid')

    def test_repeat_disagreement_and_mate_like_scores_are_not_averaged(self):
        calls = [{'move': '7g7f', 'result': result()} for _ in range(2)]
        calls[1]['result']['score_cp'] = 11
        self.assertEqual(teacher_scores(calls, ['7g7f'], 20)[0], 'inconsistent')
        for row in calls:
            row['result']['score_cp'] = 30000
        self.assertEqual(teacher_scores(calls, ['7g7f'], 20)[0], 'mate_like')

    def test_abba_moves_are_averaged_including_different_moves(self):
        choices = {'direct': {'baseline': [result('a')], 'candidate': [result('b')]},
                   'timed': {'baseline': [result('a'), result('c')], 'candidate': [result('b'), result('b')]}}
        got = comparison(choices, {'a': 20, 'b': 100, 'c': 60})
        self.assertEqual(got['direct']['candidate_minus_baseline_cp'], 80)
        self.assertEqual(got['timed']['candidate_minus_baseline_cp'], 60)

    def test_selected_move_union_and_reverse_repeat_order(self):
        calls = []
        def search(root, arm, mode, move, depth):
            calls.append((arm, mode, move, depth))
            if mode == 'teacher':
                return result(move, depth, score=30 if move == '2g2f' else 10)
            selected = '7g7f' if arm == 'baseline' else '2g2f'
            return result(selected, depth or 12)
        row = run_case({'identity_sha256': 'a'}, {'teacher_depths': [20, 16]}, search)
        self.assertEqual([c[0] for c in calls[2:6]], ['baseline', 'candidate', 'candidate', 'baseline'])
        self.assertEqual([c[2] for c in calls[6:]], ['2g2f', '7g7f', '7g7f', '2g2f'])
        self.assertEqual(row['comparison']['timed']['candidate_minus_baseline_cp'], 20)
        self.assertEqual(row['teacher_depth'], 20)

    def test_same_move_needs_no_teacher_and_has_no_invented_absolute_score(self):
        def search(root, arm, mode, move, depth):
            self.assertNotEqual(mode, 'teacher')
            return result(depth=depth or 10)
        row = run_case({'identity_sha256': 'a'}, {'teacher_depths': [20]}, search)
        self.assertEqual(row['status'], 'equivalent_selected_choices')
        self.assertIsNone(row['comparison']['timed']['selected_scores_cp'])
        self.assertEqual(row['comparison']['timed']['candidate_minus_baseline_cp'], 0)

    def test_fallback_rechecks_all_moves_at_common_depth(self):
        depths = []
        def search(root, arm, mode, move, depth):
            if mode == 'teacher':
                depths.append((move, depth))
                if depth == 20 and move == '2g2f':
                    return result(move, depth=19, abort='budget')
                return result(move, depth=depth)
            return result('7g7f' if arm == 'baseline' else '2g2f', depth=depth or 12)
        row = run_case({'identity_sha256': 'a'}, {'teacher_depths': [20, 16]}, search)
        self.assertEqual(row['teacher_depth'], 16)
        self.assertEqual(depths.count(('7g7f', 16)), 2)
        self.assertEqual(depths.count(('2g2f', 16)), 2)

    def test_shared_direct_move_does_not_require_teacher_for_different_timed_choices(self):
        taught = []
        def search(root, arm, mode, move, depth):
            if mode == 'teacher':
                taught.append(move)
                return result(move, depth=depth, score=20 if move == '2g2f' else 10)
            if mode == 'direct':
                return result('shared-direct', depth=1)
            return result('7g7f' if arm == 'baseline' else '2g2f', depth=12)
        row = run_case({'identity_sha256': 'a'}, {'teacher_depths': [20]}, search)
        self.assertNotIn('shared-direct', taught)
        self.assertEqual(row['comparison']['direct']['candidate_minus_baseline_cp'], 0)
        self.assertIsNone(row['comparison']['direct']['selected_scores_cp'])
        self.assertEqual(row['comparison']['timed']['candidate_minus_baseline_cp'], 10)

    def test_equal_timed_frequencies_cancel_even_when_repeat_order_differs(self):
        choices = {'baseline': [result('a'), result('b')], 'candidate': [result('b'), result('a')]}
        self.assertEqual(score_coefficients(choices), {})
        choices['candidate'][1] = result('b')
        self.assertEqual(score_coefficients(choices), {'a': -.5, 'b': .5})

    def test_inconsistent_teacher_never_tries_easier_depth(self):
        count = 0
        def search(root, arm, mode, move, depth):
            nonlocal count
            if mode == 'teacher':
                self.assertEqual(depth, 20)
                count += 1
                return result(move, depth=depth, score=count)
            return result('7g7f' if arm == 'baseline' else '2g2f', depth=depth or 12)
        row = run_case({'identity_sha256': 'a'}, {'teacher_depths': [20, 16]}, search)
        self.assertEqual(row['status'], 'unresolved')
        self.assertEqual(len(row['teacher_attempts']), 1)
        self.assertEqual(row['teacher_attempts'][0]['status'], 'inconsistent')

    def test_missing_root_never_produces_complete_batch_average(self):
        self.assertIsNone(summarize([{'comparison': None}], 2)['complete_batch_metrics'])
        self.assertEqual(summarize([{'comparison': None}], 2)['unresolved_roots'], 2)

    def test_budget_exhaustion_cannot_hide_inconsistent_other_move(self):
        calls = [{'move': move, 'result': result(move, depth=depth, score=score, abort=abort)}
                 for move, depth, score, abort in [('7g7f', 20, 10, 'none'), ('7g7f', 20, 11, 'none'),
                                                   ('2g2f', 18, 10, 'budget'), ('2g2f', 18, 10, 'budget')]]
        self.assertEqual(teacher_scores(calls, ['7g7f', '2g2f'], 20)[0], 'inconsistent')

    def test_unknown_stop_and_impossible_depth_never_fall_back(self):
        for depth, abort in [(20, 'external_stop'), (20, 'unknown'), (21, 'none'), (19, 'none')]:
            calls = [{'move': '7g7f', 'result': result(depth=depth, abort=abort)} for _ in range(2)]
            with self.subTest(depth=depth, abort=abort):
                self.assertEqual(teacher_scores(calls, ['7g7f'], 20)[0], 'invalid')

    def test_mate_observation_cannot_hide_later_repeat_disagreement(self):
        calls = [{'move': move, 'result': result(move, depth=depth, score=score)}
                 for move, depth, score in [('a', 13, 899987), ('a', 13, 899987),
                                           ('b', 20, 10), ('b', 20, 11)]]
        self.assertEqual(teacher_scores(calls, ['a', 'b'], 20)[0], 'inconsistent')

    def test_mate_observation_cannot_hide_invalid_ordinary_short_depth(self):
        calls = [{'move': move, 'result': result(move, depth=depth, score=score)}
                 for move, depth, score in [('a', 13, 899987), ('a', 13, 899987),
                                           ('b', 19, 10), ('b', 19, 10)]]
        self.assertEqual(teacher_scores(calls, ['a', 'b'], 20)[0], 'invalid')
        self.assertEqual(teacher_scores(calls[:2], ['a'], 20)[0], 'mate_like')

    def test_changed_pin_after_complete_search_prevents_complete_verdict(self):
        row = run_case({'identity_sha256': 'a'}, {'teacher_depths': [20]},
                       lambda root, arm, mode, move, depth: result(depth=depth or 10))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'roots.json').write_text(json.dumps({'positions': [
                {'identity_sha256': 'a', 'position': 'startpos moves 7g7f', 'sfen': 'fixture'}]}))
            (root / 'plan.json').write_text(json.dumps({'pins': {'roots': {'path': str(root / 'roots.json')}},
                                                      'config': {'max_roots': 1}}))
            with patch.object(runner, 'validate_plan', side_effect=[None, ValueError('pin changed')]), \
                    patch.object(runner, 'run_case', return_value=row), \
                    patch.object(runner.shutil, 'disk_usage', return_value=type('Disk', (), {'free': 16 * 1024**3})()):
                summary = runner.run(root / 'plan.json', root / 'out')
            self.assertEqual(summary['status'], 'INCOMPLETE')
            self.assertIsNone(summary['complete_batch_metrics'])
            self.assertIn('pin changed', summary['error'])

    def test_one_failed_root_keeps_later_roots_and_suppresses_batch_average(self):
        good = run_case({'identity_sha256': 'b'}, {'teacher_depths': [20]},
                        lambda root, arm, mode, move, depth: result(depth=depth or 10))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'roots.json').write_text(json.dumps({'positions': [
                {'identity_sha256': key, 'position': 'startpos moves 7g7f', 'sfen': 'fixture'} for key in ['a', 'b']]}))
            (root / 'plan.json').write_text(json.dumps({'pins': {'roots': {'path': str(root / 'roots.json')}},
                                                      'config': {'max_roots': 2}}))
            with patch.object(runner, 'validate_plan'), \
                    patch.object(runner, 'run_case', side_effect=[InvalidSearch('cap not acknowledged'), good]), \
                    patch.object(runner.shutil, 'disk_usage', return_value=type('Disk', (), {'free': 16 * 1024**3})()):
                summary = runner.run(root / 'plan.json', root / 'out')
            self.assertEqual(summary['recorded_roots'], 2)
            self.assertEqual(summary['resolved_roots'], 1)
            self.assertIsNone(summary['complete_batch_metrics'])
            self.assertTrue(summary['pinned_inputs_rechecked_after_run'])


if __name__ == '__main__':
    unittest.main()
