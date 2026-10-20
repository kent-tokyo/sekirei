#!/usr/bin/env python3
"""Integrity tests for the bounded own-NNUE pilot before any real training."""
import json
import errno
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
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

    def test_disk_full_status_write_releases_reserve_before_retry(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);reserve=p/'reserve';reserve.write_bytes(b'allocated')
            calls=[]
            def fake_save(path,value):
                calls.append(reserve.exists())
                if len(calls)==1:
                    raise OSError(errno.ENOSPC,'disk full')
            with patch.object(pilot,'save',side_effect=fake_save):
                pilot.save_with_reserve(p/'status.json',{'status':'failure'},reserve)
            self.assertEqual(calls,[True,False])

    def test_partial_reserve_allocation_failure_removes_only_reserve(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);reserve=p/'reserve';keep=p/'evidence';keep.write_text('preserve')
            with patch.object(pilot.os,'fsync',side_effect=OSError(errno.ENOSPC,'disk full')):
                with self.assertRaises(OSError):
                    pilot.allocate_reserve(reserve,mib=1)
            self.assertFalse(reserve.exists())
            self.assertEqual(keep.read_text(),'preserve')

    def test_non_capacity_io_error_does_not_consume_reserve(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);reserve=p/'reserve';reserve.write_text('keep')
            with patch.object(pilot,'save',side_effect=PermissionError(errno.EACCES,'denied')):
                with self.assertRaises(PermissionError):
                    pilot.save_with_reserve(p/'status',{},reserve)
            self.assertTrue(reserve.exists())

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
