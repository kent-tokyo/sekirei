#!/usr/bin/env python3
"""Integrity tests for the bounded own-NNUE pilot before any real training."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run_halfkp_pilot as pilot
from split_gensfen_by_game import load_records, connected_games

A = '4k4/9/9/9/9/9/9/9/4K4 b -'
B = '4k4/9/9/9/9/9/9/4G4/4K4 b -'
C = '4k4/9/9/9/9/9/4G4/9/4K4 b -'


def row(sfen, number, game, ply=30):
    return f'{sfen} {number}\t25\t0\t{ply}\t5i6i\t{game}\n'


class PilotIntegrityTest(unittest.TestCase):
    def test_known_holdout_blocks_entire_game_including_nonmatching_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);source=p/'raw.txt';target=p/'eligible.txt'
            source.write_text(row(A,1,'blocked')+row(B,2,'blocked')+row(C,3,'clean'))
            result=pilot.prepare_rows([source],{pilot.identity_hash(A+' 99')},target,24)
            self.assertEqual(result['blocked_games'],1)
            self.assertEqual(result['kept_rows'],1)
            self.assertIn('clean',target.read_text())
            self.assertNotIn('blocked',target.read_text())

    def test_move_counter_cannot_split_identical_states_across_games(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);source=p/'raw.txt';target=p/'eligible.txt'
            source.write_text(row(A,1,'first')+row(A,55,'second')+row(C,2,'third'))
            pilot.prepare_rows([source],set(),target,24)
            records,_=load_records([target]);components=connected_games(records)
            self.assertIn({'first','second'},components)
            self.assertEqual(len(components),2)
            self.assertTrue(all(r.sfen.endswith(' 1') for r in records))
            self.assertEqual({r.ply for r in records},{30})

    def test_inconsistent_game_result_and_empty_filtered_data_refuse(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);source=p/'raw.txt';target=p/'eligible.txt'
            source.write_text(row(A,1,'one')+row(B,2,'one').replace('\t0\t','\t1\t'))
            with self.assertRaisesRegex(ValueError,'inconsistent'):
                pilot.prepare_rows([source],set(),target,24)
            self.assertFalse(target.exists())
            source.write_text(row(A,1,'one',10))
            with self.assertRaisesRegex(ValueError,'no eligible'):
                pilot.prepare_rows([source],set(),target,24)
            self.assertFalse(target.exists())

    def test_generated_ids_reject_wrong_seed_or_extra_game(self):
        with tempfile.TemporaryDirectory() as tmp:
            source=Path(tmp)/'raw.txt'
            for game in ('0000000000000008-0000000000000000',
                         '0000000000000007-0000000000000002',
                         '0000000000000007-not-hex'):
                source.write_text(row(A,1,game))
                with self.subTest(game=game),self.assertRaises(ValueError):
                    pilot.validate_generated_ids(source,7,2)
            source.write_text(row(A,1,'0000000000000007-0000000000000001'))
            self.assertEqual(pilot.validate_generated_ids(source,7,2),1)

    def test_pin_failure_records_terminal_reason_without_starting_child(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);source=p/'input';source.write_text('changed')
            plan=p/'plan.json';plan.write_text(json.dumps({'bound_inputs':[{'path':str(source),'sha256':'0'*64}]}))
            with patch.object(pilot.subprocess,'Popen') as launch:
                with self.assertRaisesRegex(ValueError,'pinned input changed'):
                    pilot.run(plan)
            launch.assert_not_called()
            status=json.loads((p/'status.json').read_text())
            self.assertEqual(status['status'],'STOPPED_NOT_QUALIFIED')
            self.assertFalse(status['adopted'])
            with self.assertRaisesRegex(ValueError,'fresh run required'):
                pilot.run(plan)


if __name__=='__main__':
    unittest.main()
