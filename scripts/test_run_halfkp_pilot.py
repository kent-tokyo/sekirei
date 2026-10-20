#!/usr/bin/env python3
"""Integrity tests for the bounded own-NNUE pilot before any real training."""
import json
import copy
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


def continuation_fixture(root, returncode=0):
    source = root/'source'; source.mkdir()
    original = {'schema':'sekirei.own-halfkp-pilot-plan.v1',
                'generation':{'seeds':[7,8], 'games_per_shard':2, 'depth':8},
                'training':{'seed':42}, 'baseline':'r6.bin', 'baseline_sha256':'r6',
                'generator':'gensfen', 'packer':'pack', 'source_base':'fixed-source',
                'exclusions':[], 'spent_manifests':[], 'bound_inputs':[]}
    trainer=source/'train_halfkp.py';trainer.write_text('frozen trainer')
    original['trainer']=str(trainer.resolve())
    helpers=[Path(pilot.__file__).resolve().parent/name for name in
             ('freeze_material_gate_openings.py', 'split_gensfen_by_game.py',
              'generate_search_diagnostic.py', 'paired_search_diagnostics.py', 'nps_threads.py')]
    for helper in helpers:
        (source/helper.name).write_bytes(helper.read_bytes())
    original['bound_inputs']=[pilot.ref(x) for x in [trainer,*[source/h.name for h in helpers]]]
    oldplan=source/'plan.json';pilot.save(oldplan,original)
    retained=[]
    for n,(seed,games) in enumerate([(7,2),(8,1)]):
        raw=source/f'raw-{n:02}.txt'
        raw.write_text(''.join(row(A,1,f'{seed:016x}-{game:016x}') for game in range(games)))
        log=source/f'generate-{n:02}.stderr'
        log.write_text('eval r6.bin (HalfKp)\n'+('stop file found after 1 games\n' if n else ''))
        retained.append({'raw':pilot.ref(raw),'stderr':pilot.ref(log),
                         'seed':seed,'completed_games':games})
    status=source/'status.json'
    pilot.save(status,{'status':'STOPPED_NOT_QUALIFIED',
                       'error':'stopped between games for low output capacity',
                       'capacity_stop_requested':True,'last_child_returncode':returncode,
                       'adopted':False,'plan':pilot.ref(oldplan),'shard':2,
                       'generated_shards':[retained[0]['raw']]})
    plan=copy.deepcopy(original)
    plan['generation'].update(seeds=[9],games_per_seed=[1])
    plan['manual_capacity_continuation']={
        'source_plan':pilot.ref(oldplan),'source_status':pilot.ref(status),
        'retained_shards':retained,'replacement_seed':9}
    plan['bound_inputs'] += [pilot.ref(x) for x in
                          [oldplan,status,*source.glob('raw-*.txt'),*source.glob('*.stderr'),*helpers]]
    return plan,source


def rebind(plan,path):
    replacement=pilot.ref(path)
    plan['bound_inputs']=[replacement if x['path']==str(path.resolve()) else x
                          for x in plan['bound_inputs']]
    for item in plan['manual_capacity_continuation']['retained_shards']:
        for key in ('raw','stderr'):
            if item[key]['path']==str(path.resolve()):
                item[key]=replacement


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

    def test_capacity_continuation_retains_prefix_with_exact_remaining_budget(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp);plan,source=continuation_fixture(p)
            before={x.name:x.read_bytes() for x in source.iterdir()}
            retained=pilot.capacity_retained(plan)
            self.assertEqual(retained,[(source/'raw-00.txt').resolve(),(source/'raw-01.txt').resolve()])
            self.assertEqual(pilot.generation_jobs(plan),[(9,1)])
            self.assertEqual(before,{x.name:x.read_bytes() for x in source.iterdir()})

    def test_nonzero_child_or_post_training_source_cannot_be_retained(self):
        with tempfile.TemporaryDirectory() as tmp:
            plan,source=continuation_fixture(Path(tmp),returncode=1)
            with self.assertRaisesRegex(ValueError,'ineligible'):
                pilot.capacity_retained(plan)
        with tempfile.TemporaryDirectory() as tmp:
            plan,source=continuation_fixture(Path(tmp))
            (source/'training.stdout').touch()
            with self.assertRaisesRegex(ValueError,'already reached'):
                pilot.capacity_retained(plan)

    def test_changed_or_truncated_or_out_of_prefix_raw_refuses_continuation(self):
        for mutation in ('changed','truncated','outside','duplicate'):
            with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as tmp:
                plan,source=continuation_fixture(Path(tmp));raw=source/'raw-01.txt'
                text=raw.read_text()
                if mutation=='changed':
                    raw.write_text(text+'changed')
                elif mutation=='truncated':
                    raw.write_text(text.rstrip('\n'));rebind(plan,raw)
                elif mutation=='outside':
                    raw.write_text(text.replace('0000000000000008-0000000000000000',
                                                '0000000000000008-0000000000000001'))
                    rebind(plan,raw)
                else:
                    raw.write_text(text+text);rebind(plan,raw)
                with self.assertRaises(ValueError):
                    pilot.capacity_retained(plan)

    def test_capacity_continuation_rejects_panic_or_ambiguous_stop_log(self):
        for extra in ('panic: write failed\n','stop file found after 2 games\n'):
            with self.subTest(extra=extra),tempfile.TemporaryDirectory() as tmp:
                plan,source=continuation_fixture(Path(tmp));log=source/'generate-01.stderr'
                log.write_text(log.read_text()+extra);rebind(plan,log)
                with self.assertRaises(ValueError):
                    pilot.capacity_retained(plan)

    def test_capacity_amendment_cannot_change_recipe_or_remaining_count(self):
        for mutation in ('training','count','seed','missing_input','trainer'):
            with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as tmp:
                plan,source=continuation_fixture(Path(tmp))
                if mutation=='training':
                    plan['training']['seed']=43
                elif mutation=='count':
                    plan['generation']['games_per_seed']=[2]
                elif mutation=='seed':
                    plan['generation']['seeds']=[8]
                    plan['manual_capacity_continuation']['replacement_seed']=8
                elif mutation=='trainer':
                    altered=Path(tmp)/'altered.py';altered.write_text('different trainer')
                    plan['trainer']=str(altered);plan['bound_inputs'].append(pilot.ref(altered))
                else:
                    plan['bound_inputs'].pop()
                with self.assertRaises(ValueError):
                    pilot.capacity_retained(plan)

    def test_changed_active_split_helper_cannot_hide_behind_old_bound_copy(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);plan,source=continuation_fixture(root)
            active=root/'active';active.mkdir()
            for old in source.glob('*.py'):
                target=active/old.name;target.write_bytes(old.read_bytes())
                plan['bound_inputs'].append(pilot.ref(target))
            altered=active/'split_gensfen_by_game.py'
            altered.write_text('different selection algorithm')
            rebind(plan,altered)
            with patch.object(pilot,'__file__',str(active/'run_halfkp_pilot.py')):
                with self.assertRaisesRegex(ValueError,'active preparation helper differs'):
                    pilot.capacity_retained(plan)

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
