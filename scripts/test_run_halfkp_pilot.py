#!/usr/bin/env python3
"""Integrity tests for the bounded own-NNUE pilot before any real training."""
import json
import copy
import errno
import contextlib
import io
import os
import signal
import subprocess
import time
from types import SimpleNamespace
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
              'generate_search_diagnostic.py', 'paired_search_diagnostics.py', 'nps_threads.py',
              'run_observed_process.py')]
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
    def test_cleanup_denial_still_saves_primary_error_and_stopped_status(self):
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            paths = [root/name for name in ('generator', 'packer', 'trainer', 'baseline')]
            for path in paths:
                path.write_text('fixture')
            generator, packer, trainer, baseline = paths
            plan = {'schema':'sekirei.own-halfkp-pilot-plan.v1',
                    'generator':str(generator), 'packer':str(packer), 'trainer':str(trainer),
                    'baseline':str(baseline), 'baseline_sha256':pilot.sha(baseline),
                    'generation':{'seeds':[7], 'games_per_shard':1, 'depth':1, 'nodes':1,
                                  'random_plies':0, 'min_training_ply':24, 'max_ply':64},
                    'training':{}, 'max_wall_hours':1, 'exclusions':[], 'spent_manifests':[],
                    'bound_inputs':[pilot.ref(x) for x in paths]}
            pilot.save(root/'plan.json', plan)
            primary = pilot.PilotShutdown('received signal 15')
            child = Mock(pid=424242, returncode=None)
            child.poll.return_value = None
            child.send_signal.side_effect = PermissionError(errno.EPERM, 'child denied')
            child.wait.side_effect = [primary, subprocess.TimeoutExpired('owned child', .01)]
            with patch.object(pilot.subprocess, 'Popen', return_value=child) as spawn, \
                 patch.object(pilot, 'observe_host_cpu', return_value=0.0), \
                 patch.object(pilot.shutil, 'disk_usage',
                              return_value=SimpleNamespace(free=5000*1048576)), \
                 patch.object(pilot.time, 'sleep'), \
                 patch.object(pilot, 'allocate_reserve', side_effect=lambda path:path.write_bytes(b'x')), \
                 patch.object(pilot.os, 'killpg',
                              side_effect=PermissionError(errno.EPERM, 'group denied')):
                with self.assertRaises(pilot.PilotShutdown) as caught:
                    pilot.run(root/'plan.json')
            self.assertIs(caught.exception, primary)
            spawn.assert_called_once()
            saved = json.loads((root/'status.json').read_text())
            self.assertEqual(saved['status'], 'STOPPED_NOT_QUALIFIED')
            self.assertEqual(saved['error'], 'received signal 15')
            self.assertTrue(saved['cleanup_incomplete'])
            self.assertTrue(any(x['error_type'] == 'PermissionError' for x in saved['cleanup_errors']))

    def test_capacity_wait_enospc_keeps_terminal_cause_with_live_child(self):
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            inputs = [root/name for name in ('generator', 'packer', 'trainer', 'baseline')]
            for path in inputs:
                path.write_text('fixture')
            generator, packer, trainer, baseline = inputs
            plan = {'schema':'sekirei.own-halfkp-pilot-plan.v1',
                    'generator':str(generator), 'packer':str(packer), 'trainer':str(trainer),
                    'baseline':str(baseline), 'baseline_sha256':pilot.sha(baseline),
                    'generation':{'seeds':[7], 'games_per_shard':1, 'depth':1, 'nodes':1,
                                  'random_plies':0, 'min_training_ply':24, 'max_ply':64},
                    'training':{}, 'max_wall_hours':1, 'exclusions':[], 'spent_manifests':[],
                    'bound_inputs':[pilot.ref(path) for path in inputs]}
            pilot.save(root/'plan.json', plan)
            child = Mock(pid=424242, returncode=None)
            child.poll.return_value = None
            child.send_signal.side_effect = PermissionError(errno.EPERM, 'child denied')
            reserve_path = root/'.pilot-terminal-reserve'
            actual_save = pilot.save
            paused_writes = 0
            launches = 0

            def launch(*args, **kwargs):
                nonlocal launches
                launches += 1
                self.assertTrue(reserve_path.exists())
                self.assertGreaterEqual(reserve_path.stat().st_blocks * 512, 8 * 1048576)
                return child

            def disk(_):
                return SimpleNamespace(free=(1024 if launches else 5000)*1048576)

            def write(path, document):
                nonlocal paused_writes
                path = Path(path)
                if path.name == 'status.json' and document.get('status') == 'PAUSED_CAPACITY':
                    paused_writes += 1
                    if paused_writes >= 2:
                        raise OSError(errno.ENOSPC, 'capacity wait status full')
                if path.name == 'status.json' and document.get('status') == 'STOPPED_NOT_QUALIFIED':
                    raise OSError(errno.ENOSPC, 'terminal status full')
                if path.name == 'pilot-terminal.json' and reserve_path.exists():
                    raise OSError(errno.ENOSPC, 'terminal receipt needs reserve')
                actual_save(path, document)

            def signal_group(_pid, number):
                if number != signal.SIGSTOP:
                    raise PermissionError(errno.EPERM, 'group denied')

            output = io.StringIO()
            with patch.object(pilot.subprocess, 'Popen', side_effect=launch) as spawn, \
                 patch.object(pilot.shutil, 'disk_usage', side_effect=disk), \
                 patch.object(pilot, 'observe_host_cpu', return_value=0.0), \
                 patch.object(pilot.time, 'sleep'), \
                 patch.object(pilot, 'allocate_reserve',
                              side_effect=lambda path:path.write_bytes(b'x')), \
                 patch.object(pilot.os, 'getpgid', return_value=child.pid), \
                 patch.object(pilot.os, 'getpgrp', return_value=child.pid+1), \
                 patch.object(pilot.os, 'killpg', side_effect=signal_group), \
                 patch.object(pilot, 'save', side_effect=write), \
                 contextlib.redirect_stderr(output):
                with self.assertRaises(OSError) as caught:
                    pilot.run(root/'plan.json')
            self.assertEqual(caught.exception.errno, errno.ENOSPC)
            self.assertIn('capacity wait status full', str(caught.exception))
            spawn.assert_called_once()
            self.assertEqual(paused_writes, 2)
            self.assertFalse((root/'emergency-reserve.bin').exists())
            self.assertFalse(reserve_path.exists())
            self.assertEqual(json.loads((root/'status.json').read_text())['status'],
                             'PAUSED_CAPACITY')
            receipt = json.loads((root/'pilot-terminal.json').read_text())
            self.assertEqual(receipt['error'], str(caught.exception))
            self.assertEqual(receipt['child_pid'], child.pid)
            self.assertIsNone(receipt['child_returncode'])
            self.assertTrue(receipt['cleanup_incomplete'])
            self.assertTrue(any(x['error_type'] == 'PermissionError'
                                for x in receipt['cleanup_errors']))
            self.assertFalse(receipt['automatic_retry'])
            self.assertEqual(receipt['persistence_recovery']['reason'], 'ENOSPC')

    def test_terminal_reserve_allocation_failure_prevents_owned_launch(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pilot.save(root/'plan.json', {'schema':'bad', 'bound_inputs':[]})
            with patch.object(pilot.EmergencyReserve, 'allocate',
                              side_effect=OSError(errno.ENOSPC, 'terminal reserve full')), \
                 patch.object(pilot.subprocess, 'Popen') as spawn:
                with self.assertRaisesRegex(OSError, 'terminal reserve full'):
                    pilot.run(root/'plan.json')
            spawn.assert_not_called()
            receipt = json.loads((root/'pilot-terminal.json').read_text())
            self.assertEqual(receipt['error_type'], 'OSError')
            self.assertIsNone(receipt['child_pid'])
            self.assertFalse(receipt['automatic_retry'])

    def test_terminal_persistence_failure_does_not_replace_primary_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pilot.save(root/'plan.json', {'schema':'bad', 'bound_inputs':[]})
            save = pilot.save

            def fail_terminal(path, document):
                if document.get('status') == 'STOPPED_NOT_QUALIFIED':
                    raise OSError(errno.ENOSPC, 'terminal journal full')
                save(path, document)

            output = io.StringIO()
            with patch.object(pilot, 'save', side_effect=fail_terminal), \
                 contextlib.redirect_stderr(output):
                with self.assertRaisesRegex(ValueError, 'unknown plan schema'):
                    pilot.run(root/'plan.json')
            record = json.loads(output.getvalue())
            self.assertEqual(record['error'], 'unknown plan schema')
            self.assertIn('terminal journal full', record['terminal_persistence_error'])

    def test_failed_terminal_stderr_does_not_replace_primary_error(self):
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            pilot.save(root/'plan.json', {'schema':'bad', 'bound_inputs':[]})
            save = pilot.save

            def fail_terminal(path, document):
                if document.get('status') == 'STOPPED_NOT_QUALIFIED':
                    raise OSError(errno.ENOSPC, 'terminal journal full')
                save(path, document)

            stderr = Mock()
            stderr.write.side_effect = OSError(errno.ENOSPC, 'stderr full')
            with patch.object(pilot, 'save', side_effect=fail_terminal), \
                 patch.object(pilot.sys, 'stderr', stderr):
                with self.assertRaisesRegex(ValueError, 'unknown plan schema'):
                    pilot.run(root/'plan.json')

    def test_candidate_id_is_bounded_ascii_and_cannot_escape_or_replace_data(self):
        self.assertEqual(pilot.candidate_name({}), 'nn_r8c1')
        self.assertEqual(pilot.candidate_name({'candidate_id':'nn_r8c3'}), 'nn_r8c3')
        self.assertEqual(pilot.candidate_name({'candidate_id':'a'*64}), 'a'*64)
        for name in ('', None, 3, '../nn_r8c3', '/tmp/nn_r8c3', 'a/b', 'a\\b',
                     '.', '..', 'a.pt', 'x\n', 'ｎｎ_r8c3', 'a'*65,
                     'train', 'val', 'emergency-reserve', 'Train', 'VAL', 'Emergency-Reserve'):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'candidate_id'):
                pilot.candidate_name({'candidate_id':name})

    def test_second_candidate_wires_trainer_loader_and_status_to_same_identifier(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve()
            paths=[root/name for name in ('generator', 'packer', 'trainer', 'baseline')]
            for path in paths:
                path.write_text('frozen fixture '+path.name)
            generator,packer,trainer,baseline=paths
            diff_tool=root/'parameter_diff_tool.py';diff_tool.write_text('frozen diagnostic')
            plan={'schema':'sekirei.own-halfkp-pilot-plan.v1','candidate_id':'nn_r8c3',
                  'generator':str(generator),'packer':str(packer),'trainer':str(trainer),
                  'baseline':str(baseline),'baseline_sha256':pilot.sha(baseline),
                  'generation':{'seeds':[7],'games_per_shard':1,'depth':1,'nodes':1,
                      'random_plies':0,'min_training_ply':24,'max_ply':64},
                  'training':{'batch':256,'lr':1e-4,'seed':42},'max_wall_hours':1,
                  'exclusions':[],'spent_manifests':[],
                  'parameter_diff_tool':str(diff_tool),
                  'bound_inputs':[pilot.ref(p) for p in [*paths,diff_tool]]}
            pilot.save(root/'plan.json',plan)
            commands=[]
            def launch(argv, **kwargs):
                commands.append(argv)
                if argv[0]==str(generator):
                    Path(argv[argv.index('--out')+1]).write_text(
                        row(A,1,'0000000000000007-0000000000000000'))
                    kwargs['stderr'].write('eval baseline (HalfKp)\n')
                    kwargs['stderr'].flush()
                elif '--prepare-only' in argv:
                    for arm in ('train','val'):
                        (root/(arm+'.txt')).write_text('fixture')
                    pilot.save(root/'split.json',
                               {'train':{'positions':2},'validation':{'positions':1}})
                elif argv[0]==str(packer) and argv[1]=='pack':
                    count=2 if Path(argv[2]).name=='train.bin' else 1
                    Path(argv[2]).write_bytes(bytes(count*158))
                elif str(trainer) in argv:
                    Path(argv[argv.index('--out')+1]).write_text('candidate fixture')
                    Path(argv[argv.index('--save')+1]).write_text('checkpoint fixture')
                elif str(diff_tool) in argv:
                    self.assertEqual(argv, [sys.executable,'-B',str(diff_tool),str(baseline),
                                          str(root/'nn_r8c3.bin'),'--output',
                                          str(root/'integer-parameter-diff.json')])
                    pilot.save(root/'integer-parameter-diff.json',{'diagnostic_only':True})
                elif argv[0]==str(packer):
                    self.assertEqual(argv[1:4], ['check',str(root/'nn_r8c3.bin'),'24'])
                else:
                    self.fail('unexpected command '+repr(argv))
                return SimpleNamespace(pid=12345,returncode=0,poll=lambda:0)
            def reserve(path):
                path.write_bytes(b'reserve fixture')
            with patch.object(pilot.subprocess,'Popen',side_effect=launch), \
                 patch.object(pilot,'observe_host_cpu',return_value=0.0), \
                 patch.object(pilot.shutil,'disk_usage',return_value=SimpleNamespace(free=5000*1048576)), \
                 patch.object(pilot.time,'sleep'), \
                 patch.object(pilot,'allocate_reserve',side_effect=reserve):
                pilot.run(root/'plan.json')
            training=next(cmd for cmd in commands if str(trainer) in cmd)
            self.assertEqual(training[training.index('--out')+1],str(root/'nn_r8c3.bin'))
            self.assertEqual(training[training.index('--save')+1],str(root/'nn_r8c3.pt'))
            self.assertEqual(training[training.index('--batch')+1],'256')
            for filename in ('candidate.json','status.json'):
                status=json.loads((root/filename).read_text())
                self.assertEqual(status['candidate_id'],'nn_r8c3')
                self.assertEqual(status['candidate'],pilot.ref(root/'nn_r8c3.bin'))
                self.assertEqual(status['float_checkpoint'],pilot.ref(root/'nn_r8c3.pt'))
                self.assertFalse(status['adopted'])
                self.assertEqual(status['integer_parameter_diff'],
                                 pilot.ref(root/'integer-parameter-diff.json'))
            unbound=copy.deepcopy(plan)
            unbound['bound_inputs']=[p for p in plan['bound_inputs'] if p['path']!=str(diff_tool)]
            with self.assertRaisesRegex(ValueError,'unbound'):
                pilot.validate_plan(unbound)
            self.assertFalse((root/'nn_r8c1.bin').exists())
            self.assertEqual(pilot.sha(baseline),plan['baseline_sha256'])

    def test_quality_floors_are_optional_strict_and_do_not_weaken_original_minima(self):
        self.assertEqual(pilot.quality_floors({}),
                         {'train_positions':4096,'validation_positions':256})
        self.assertEqual(pilot.quality_floors({'quality_floors':{'train_positions':1}}),
                         {'train_positions':4096,'validation_positions':256})
        for floors in (None, [], {'unknown':1}, {'train_games':0},
                       {'train_games':-1}, {'train_games':True}, {'train_games':1.5}):
            with self.subTest(floors=floors),self.assertRaisesRegex(ValueError,'quality_floors'):
                pilot.quality_floors({'quality_floors':floors})
        plan={'quality_floors':{'train_positions':200000,'validation_positions':20000,
                               'train_games':4000,'validation_games':400}}
        split={'train':{'positions':200000,'games':4000},
               'validation':{'positions':20000,'games':400}}
        pilot.validate_quality(plan,split)
        for arm,field in (('train','positions'),('validation','positions'),
                          ('train','games'),('validation','games')):
            bad=copy.deepcopy(split);bad[arm][field]-=1
            with self.subTest(arm=arm,field=field),self.assertRaisesRegex(ValueError,'insufficient'):
                pilot.validate_quality(plan,bad)

    def test_prepare_rejects_quality_floor_after_split_without_training_or_refill(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);raw=root/'raw.txt'
            raw.write_text(row(A,1,'game'))
            pilot.save(root/'generation-receipt.json',[pilot.ref(raw)])
            plan={'exclusions':[],'spent_manifests':[],
                  'generation':{'min_training_ply':24},
                  'training':{'validation_ratio':0.1,'seed':42},
                  'quality_floors':{'train_games':4000}}
            split={'train':{'positions':200000,'games':3999},
                   'validation':{'positions':20000,'games':400}}
            with patch.object(pilot,'load_exclusions',return_value=(set(), [])), \
                 patch.object(pilot,'split_files',return_value=split) as splitter, \
                 patch.object(pilot.subprocess,'Popen') as launch:
                with self.assertRaisesRegex(ValueError,'train_games 3999 < 4000'):
                    pilot.prepare_data(plan,root)
                splitter.assert_called_once()
                launch.assert_not_called()
            self.assertTrue((root/'eligible.txt').exists())
            self.assertFalse((root/'train.bin').exists())
            self.assertFalse((root/'nn_r8c1.bin').exists())

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

    def test_transient_or_malformed_cpu_observation_is_unknown(self):
        for failure in (OSError('temporary ps failure'),subprocess.TimeoutExpired('ps',10)):
            with self.subTest(error=failure),patch.object(pilot.subprocess,'run',side_effect=failure):
                self.assertIsNone(pilot.observe_host_cpu())
        for output in ('', 'nan', '-1', 'broken'):
            with self.subTest(output=output),patch.object(pilot.subprocess,'run',
                    return_value=SimpleNamespace(returncode=0,stdout=output)):
                self.assertIsNone(pilot.observe_host_cpu())

    def test_operator_signal_during_cpu_observation_is_not_swallowed(self):
        def terminate(*args, **kwargs):
            pilot.interrupted(signal.SIGTERM,None)
        with patch.object(pilot.subprocess,'run',side_effect=terminate):
            with self.assertRaisesRegex(pilot.PilotShutdown,'received signal'):
                pilot.observe_host_cpu()

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
        for name in ('split_gensfen_by_game.py', 'run_observed_process.py'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);plan,source=continuation_fixture(root)
                active=root/'active';active.mkdir()
                for old in source.glob('*.py'):
                    target=active/old.name;target.write_bytes(old.read_bytes())
                    plan['bound_inputs'].append(pilot.ref(target))
                altered=active/name
                altered.write_text('different helper implementation')
                rebind(plan,altered)
                with patch.object(pilot,'__file__',str(active/'run_halfkp_pilot.py')):
                    with self.assertRaisesRegex(ValueError,'active preparation helper differs'):
                        pilot.capacity_retained(plan)

    def test_capacity_continuation_requires_active_cleanup_helper_pin(self):
        with tempfile.TemporaryDirectory() as tmp:
            plan,_=continuation_fixture(Path(tmp))
            helper=Path(pilot.__file__).resolve().parent/'run_observed_process.py'
            plan['bound_inputs']=[item for item in plan['bound_inputs']
                                  if item['path'] != str(helper)]
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


@unittest.skipUnless(os.name == 'posix', 'POSIX process groups required')
class CapacityPauseProcessTest(unittest.TestCase):
    @staticmethod
    def launch_writer(path):
        code = ("import os,sys,time\n"
                "with open(sys.argv[1],'x') as f:\n"
                " for n in range(40):\n"
                "  f.write(str(n)+'\\n');f.flush();time.sleep(.02)\n")
        return subprocess.Popen([sys.executable,'-u','-c',code,str(path)],
                                start_new_session=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)

    def wait_started(self, proc, path):
        deadline=time.monotonic()+5
        while time.monotonic()<deadline:
            if path.exists() and path.stat().st_size:
                return
            self.assertIsNone(proc.poll())
            time.sleep(.01)
        self.fail('test writer did not start')

    def test_paused_group_stops_writing_then_continues_same_sequence_and_pid(self):
        with tempfile.TemporaryDirectory() as tmp:
            output=Path(tmp)/'sequence';proc=self.launch_writer(output)
            try:
                self.wait_started(proc,output)
                pid=proc.pid;events=[];waits=[];reserves=[]
                def wait_capacity():
                    deadline=time.monotonic()+3
                    while time.monotonic()<deadline:
                        state=subprocess.check_output(['ps','-p',str(pid),'-o','stat='],text=True)
                        if 'T' in state:
                            break
                        time.sleep(.01)
                    else:
                        self.fail('group never stopped')
                    before=output.read_bytes();time.sleep(.06)
                    self.assertEqual(output.read_bytes(),before)
                    self.assertIsNone(proc.poll());waits.append(True)
                for _ in range(2):
                    duration=pilot.pause_child_for_capacity(proc,wait_capacity,
                        lambda:reserves.append(True),lambda action,seconds:events.append(action))
                    self.assertGreaterEqual(duration,.12)
                    self.assertEqual(proc.pid,pid)
                self.assertEqual(proc.wait(timeout=5),0)
                self.assertEqual(output.read_text().splitlines(),[str(n) for n in range(40)])
                self.assertEqual(events,['paused','resumed','paused','resumed'])
                self.assertEqual(len(waits),4);self.assertEqual(len(reserves),2)
            finally:
                pilot.terminate_child(proc)

    def test_runner_wiring_pauses_after_capacity_drop_and_waits_through_unknown_cpu(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp).resolve();generator=root/'generator'
            generator.write_text('#!'+sys.executable+'\nimport sys,time\n'
                "time.sleep(.2)\nprint('eval r6 (HalfKp)',file=sys.stderr)\n"
                "out=sys.argv[sys.argv.index('--out')+1]\n"
                'open(out,"x").write('+repr(row(A,1,'0000000000000007-0000000000000000'))+')\n')
            generator.chmod(0o700)
            baseline=root/'baseline';baseline.write_text('frozen fixture evaluator')
            trainer=root/'trainer';trainer.write_text('unused fixture trainer')
            plan={'schema':'sekirei.own-halfkp-pilot-plan.v1',
                  'generator':str(generator),'packer':str(generator),'trainer':str(trainer),
                  'baseline':str(baseline),'baseline_sha256':pilot.sha(baseline),
                  'generation':{'seeds':[7],'games_per_shard':1,'depth':1,'nodes':1,
                      'random_plies':0,'min_training_ply':24,'max_ply':64},
                  'training':{},'max_wall_hours':1,'exclusions':[],'spent_manifests':[],
                  'bound_inputs':[pilot.ref(x) for x in (generator,baseline,trainer)]}
            pilot.save(root/'plan.json',plan)
            real_popen=subprocess.Popen;real_sleep=time.sleep;real_save=pilot.save
            real_allocate=pilot.allocate_reserve;processes=[];drops=[];unknown=[];allocations=[]
            def launch(argv,**kwargs):
                proc=real_popen(argv,**kwargs)
                if argv[0]==str(generator):
                    processes.append(proc)
                return proc
            def disk(_):
                if processes and not drops:
                    drops.append(True);return SimpleNamespace(free=1024*1048576)
                return SimpleNamespace(free=5000*1048576)
            def cpu():
                state=json.loads((root/'status.json').read_text())
                if state.get('capacity_paused') and not unknown:
                    unknown.append(True);return None
                return 0.0
            def save(path,document):
                if Path(path).name=='generation-receipt.json':
                    raise ValueError('fixture stops before preparation')
                real_save(path,document)
            def allocate(path):
                allocations.append(True);real_allocate(path,mib=1)
            with patch.object(pilot.subprocess,'Popen',side_effect=launch), \
                 patch.object(pilot.shutil,'disk_usage',side_effect=disk), \
                 patch.object(pilot,'observe_host_cpu',side_effect=cpu), \
                 patch.object(pilot.time,'sleep',side_effect=lambda n:real_sleep(min(n,.002))), \
                 patch.object(pilot,'allocate_reserve',side_effect=allocate), \
                 patch.object(pilot,'save',side_effect=save):
                with self.assertRaisesRegex(ValueError,'fixture stops before preparation'):
                    pilot.run(root/'plan.json')
            self.assertEqual(len(processes),1)
            self.assertEqual(processes[0].returncode,0)
            self.assertEqual(len(allocations),2)
            self.assertEqual(len(unknown),1)
            events=[json.loads(x.read_text()) for x in sorted(root.glob('capacity-event-*.json'))]
            self.assertEqual([x['action'] for x in events],['paused','resumed'])
            self.assertEqual({x['child_pid'] for x in events},{processes[0].pid})
            self.assertEqual((root/'raw-00.txt').read_text(),row(A,1,'0000000000000007-0000000000000000'))
            self.assertFalse((root/'STOP_GENERATION').exists())
            self.assertEqual(json.loads((root/'status.json').read_text())['capacity_pause_count'],1)

    def test_cancel_while_paused_terminates_child_without_relaunch(self):
        with tempfile.TemporaryDirectory() as tmp:
            output=Path(tmp)/'sequence';proc=self.launch_writer(output)
            try:
                self.wait_started(proc,output)
                def cancel():
                    raise InterruptedError('operator stop')
                with self.assertRaisesRegex(InterruptedError,'operator stop'):
                    pilot.pause_child_for_capacity(proc,cancel,lambda:None,lambda *args:None)
                self.assertIsNotNone(proc.poll())
                self.assertNotEqual(proc.returncode,0)
            finally:
                pilot.terminate_child(proc)

    def test_cont_denial_during_paused_cleanup_keeps_original_shutdown(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)/'sequence'
            proc = self.launch_writer(output)
            real_killpg = os.killpg
            primary = pilot.PilotShutdown('received signal 15')

            def deny_cont(pid, number):
                if number == signal.SIGCONT:
                    raise PermissionError(errno.EPERM, 'group CONT denied')
                return real_killpg(pid, number)

            def cancel():
                raise primary

            try:
                self.wait_started(proc, output)
                with patch.object(pilot.os, 'killpg', side_effect=deny_cont):
                    with self.assertRaises(pilot.PilotShutdown) as caught:
                        pilot.pause_child_for_capacity(proc, cancel, lambda:None, lambda *args:None)
                self.assertIs(caught.exception, primary)
                self.assertEqual(proc.returncode, -signal.SIGTERM)
                self.assertFalse(primary.cleanup_incomplete)
                self.assertEqual(primary.cleanup_errors[0]['action'], 'killpg:SIGCONT')
            finally:
                pilot.terminate_child(proc)

    def test_killed_paused_child_is_not_resumed_or_retried(self):
        with tempfile.TemporaryDirectory() as tmp:
            output=Path(tmp)/'sequence';proc=self.launch_writer(output);events=[]
            try:
                self.wait_started(proc,output)
                def kill():
                    os.killpg(proc.pid,signal.SIGKILL);proc.wait(timeout=3)
                with self.assertRaisesRegex(ValueError,'paused child exited'):
                    pilot.pause_child_for_capacity(proc,kill,lambda:None,
                                                   lambda action,seconds:events.append(action))
                self.assertEqual(events,['paused'])
                self.assertEqual(proc.returncode,-signal.SIGKILL)
            finally:
                pilot.terminate_child(proc)

    def test_pause_record_failure_cleans_up_stopped_child(self):
        with tempfile.TemporaryDirectory() as tmp:
            output=Path(tmp)/'sequence';proc=self.launch_writer(output)
            try:
                self.wait_started(proc,output)
                def fail(*args):
                    raise OSError(errno.ENOSPC,'record full')
                with self.assertRaises(OSError):
                    pilot.pause_child_for_capacity(proc,lambda:None,lambda:None,fail)
                self.assertIsNotNone(proc.poll())
            finally:
                pilot.terminate_child(proc)


if __name__=='__main__':
    unittest.main()
