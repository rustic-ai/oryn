"""Frozen security admission and no-mutation regressions; no real build or signing."""
import copy
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import completion_execution
import completion_provenance
import completion_qualification
import security_prebuilt as prebuilt


class SecurityPrebuiltTests(unittest.TestCase):
    def preparation(self, directory):
        root = Path(directory).resolve()
        lease = SimpleNamespace(path=root/'gate.json', sha256='a'*64, check=Mock())
        profile = root/'profile'; profile.write_text('{}')
        candidate = dict(path=root/'candidate.json', sha256='b'*64,
                         record=dict(source_tree_sha256='c'*64,
                                     artifacts={'engine_profile': {'path':str(profile), 'sha256':prebuilt.digest(profile)}}),
                         artifacts={'runtime':root/'target/release/oryn',
                                    'worker':root/'target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'})
        worker = candidate['artifacts']['worker']; worker.parent.mkdir(parents=True)
        worker.write_text('frozen signed worker')
        resources = worker.parents[1]/'Resources';resources.mkdir()
        (resources/'v8-profile-manifest.json').write_text('{}')
        candidate['artifacts']['worker_manifest'] = resources/'worker-manifest.json'
        candidate['artifacts']['worker_manifest'].write_text('{}')
        (worker.parents[1]/'Info.plist').write_text('frozen Info.plist')
        value = dict(schema_version=1, source_tree_sha256='c'*64,
                     engine_profile=candidate['record']['artifacts']['engine_profile'],
                     generation_gate=dict(path=str(lease.path),sha256=lease.sha256),
                     producer_sources={name:prebuilt.digest(prebuilt.ROOT/'scripts'/name) for name in
                                       ('prepare-security-prebuilt.py','security_prebuilt.py')}, builds={},
                     bundle_files=prebuilt.bundle_inventory(worker))
        for name, command in prebuilt.BUILD_COMMANDS.items():
            executable = root/name
            executable.write_text('#!/bin/sh\nexit 0\n'); executable.chmod(0o755)
            log = root/(name+'.log'); log.write_text('successful current-source build\n')
            value['builds'][name] = dict(argv=command, returncode=0,
                executable=dict(path=str(executable),sha256=prebuilt.digest(executable)),
                log=dict(path=str(log),sha256=prebuilt.digest(log)))
        manifest = root/'prepared.json'; manifest.write_text(json.dumps(value))
        candidate['artifacts']['security_prebuilt'] = manifest
        candidate['record']['artifacts'].update(prebuilt.preparation_artifacts(value))
        candidate['record']['artifacts']['security_prebuilt'] = dict(path=str(manifest),sha256=prebuilt.digest(manifest))
        return lease, candidate, value

    def test_preparation_rejects_source_profile_gate_command_and_byte_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            lease, candidate, value = self.preparation(directory)
            self.assertEqual(set(prebuilt.validate_preparation(value,candidate,lease)), set(prebuilt.BUILD_COMMANDS))
            mutations = [lambda v:v.update(source_tree_sha256='0'*64),
                         lambda v:v['engine_profile'].update(sha256='0'*64),
                         lambda v:v['generation_gate'].update(sha256='0'*64),
                         lambda v:v['producer_sources'].update({'security_prebuilt.py':'0'*64}),
                         lambda v:v['builds'].pop('debug_worker'),
                         lambda v:v['builds']['debug_worker'].update(argv=['true']),
                         lambda v:v['builds']['debug_worker'].update(returncode=True)]
            for mutate in mutations:
                changed = copy.deepcopy(value); mutate(changed)
                with self.assertRaises(ValueError): prebuilt.validate_preparation(changed,candidate,lease)
            Path(value['builds']['debug_worker']['executable']['path']).write_text('stale substituted bytes')
            with self.assertRaisesRegex(ValueError,'missing or changed'):
                prebuilt.validate_preparation(value,candidate,lease)

    def test_binding_rejects_candidate_source_and_build_target_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            lease, candidate, _ = self.preparation(directory)
            root = Path(directory).resolve()
            (root/'scripts').mkdir()
            for name in ('prepare-security-prebuilt.py','security_prebuilt.py'):
                shutil.copy2(prebuilt.ROOT/'scripts'/name,root/'scripts'/name)
            candidate['artifacts']['runtime'] = root/'target/release/oryn'
            candidate['artifacts']['worker'] = root/'target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
            resources = candidate['artifacts']['worker'].parents[1]/'Resources'
            resources.mkdir(parents=True,exist_ok=True)
            (resources/'v8-profile-manifest.json').write_text('{}')
            candidate['artifacts']['worker_manifest'] = resources/'worker-manifest.json'
            environment = {prebuilt.CANDIDATE_ENV:str(candidate['path']),prebuilt.DIGEST_ENV:candidate['sha256'],
                           'CARGO_TARGET_DIR':str(root.parent/(root.name+'-checks'))}
            with patch.dict(os.environ,environment,clear=True), patch.object(prebuilt,'ROOT',root), \
                 patch.object(prebuilt,'inherited_generation_lease',return_value=lease), \
                 patch.object(prebuilt,'load_candidate',return_value=candidate), \
                 patch.object(prebuilt,'source_hash',return_value='c'*64):
                prebuilt.bind()
                with patch.dict(os.environ,{prebuilt.DIGEST_ENV:'0'*64}):
                    with self.assertRaisesRegex(ValueError,'candidate digest'): prebuilt.bind()
                with patch.object(prebuilt,'source_hash',return_value='0'*64):
                    with self.assertRaisesRegex(ValueError,'source differs'): prebuilt.bind()
                with patch.dict(os.environ,{'CARGO_TARGET_DIR':str(prebuilt.ROOT/'target')}):
                    with self.assertRaisesRegex(ValueError,'overlaps frozen'): prebuilt.bind()
                with patch.object(prebuilt,'load_candidate',side_effect=ValueError('profile digest mismatch')):
                    with self.assertRaisesRegex(ValueError,'profile digest'): prebuilt.bind()
                (resources/'v8-profile-manifest.json').write_text('substituted profile')
                with self.assertRaisesRegex(ValueError,'bundle differs'): prebuilt.bind()

    def test_preparation_requires_every_transitive_record_in_candidate(self):
        with tempfile.TemporaryDirectory() as directory:
            lease,candidate,value=self.preparation(directory)
            for name in prebuilt.preparation_artifacts(value):
                changed=copy.deepcopy(candidate)
                changed['record']['artifacts'].pop(name)
                with self.assertRaisesRegex(ValueError,'custody'):
                    prebuilt.validate_preparation(value,changed,lease)

    def test_final_candidate_consumption_rejects_changed_or_deleted_prepared_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            _,candidate,value=self.preparation(directory)
            root=Path(directory); snapshot=root/'source.tar';snapshot.write_bytes(b'fixture source snapshot')
            contract=completion_provenance.ROOT/'test-harness/workflows/five-family-contract-v1.json'
            records=candidate['record']['artifacts']
            records['worker']=dict(path=str(candidate['artifacts']['worker']),
                                   sha256=prebuilt.digest(candidate['artifacts']['worker']))
            records['worker_manifest']=dict(path=str(candidate['artifacts']['worker_manifest']),
                                            sha256=prebuilt.digest(candidate['artifacts']['worker_manifest']))
            records['source_snapshot']=dict(path=str(snapshot),sha256=prebuilt.digest(snapshot))
            records['workflow_contract']=dict(path=str(contract),sha256=prebuilt.digest(contract))
            candidate_path=root/'final-candidate.json'
            candidate_path.write_text(json.dumps(dict(schema_version=1,source_tree_sha256='c'*64,artifacts=records)))
            with patch.object(completion_provenance,'REQUIRED_ARTIFACTS',frozenset()), \
                 patch.object(completion_provenance,'validate_source_snapshot'), \
                 patch.object(completion_provenance,'validate_engine'):
                completion_provenance.load_candidate(candidate_path)
                for record in prebuilt.preparation_artifacts(value).values():
                    path=Path(record['path']);before=path.read_bytes()
                    for remove in (False,True):
                        if remove:path.unlink()
                        else:path.write_bytes(before+b'changed after producer success')
                        with self.assertRaisesRegex(ValueError,'missing or changed'):
                            completion_provenance.load_candidate(candidate_path)
                        path.write_bytes(before)
                extra=candidate['artifacts']['worker'].parents[1]/'late-added-resource'
                extra.write_text('added after original security receipt consumption')
                with self.assertRaisesRegex(ValueError,'file set or bytes changed'):
                    completion_provenance.load_candidate(candidate_path)

    def test_final_security_consumption_rechecks_bundle_membership_and_signature(self):
        with tempfile.TemporaryDirectory() as directory:
            _,candidate,_=self.preparation(directory)
            with patch.object(prebuilt,'verify_signature') as verify:
                prebuilt.verify_consumed_security(candidate)
                verify.assert_called_once_with(candidate)
            with patch.object(prebuilt,'verify_signature',side_effect=ValueError('invalid live signature')):
                with self.assertRaisesRegex(ValueError,'invalid live signature'):
                    prebuilt.verify_consumed_security(candidate)
            sidecar=candidate['artifacts']['worker'].parents[1]/'unexpected-resource'
            sidecar.write_text('not in the signed candidate')
            with self.assertRaisesRegex(ValueError,'file set or bytes'):
                prebuilt.verify_consumed_security(candidate)

    def test_final_evidence_inventory_ignores_external_extension_stage(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve();raw=root/'raw';raw.mkdir()
            external=root/'private-extension';external.mkdir()
            modules=root/'installed-node-modules';modules.mkdir()
            (external/'node_modules').symlink_to(modules,target_is_directory=True)
            def record(path):return dict(path=str(path),sha256=prebuilt.digest(path))
            def document(name,value):
                path=root/name;path.write_text(json.dumps(value));return path
            stub=document('stub.json',{})
            before_candidate=document('before-candidate.json',{'artifacts':{}})
            preflight=document('before-preflight.json',{name:record(stub) for name in ('candidate','generation_gate','producer_source')})
            before_receipt=document('before-receipt.json',{'artifacts':[],'preflight':record(preflight)})
            diagnosis=document('diagnosis.json',{name:record(path) for name,path in {
                'original_failure':stub,'regression_before':stub,'regression_before_receipt':before_receipt,
                'regression_before_candidate':before_candidate,'regression_after':stub,'regression_receipt':stub}.items()})
            inventory=root/'inventory';inventory.mkdir()
            (inventory/'plan.json').write_text('{}');(inventory/'coverage.jsonl').write_text('')
            (raw/'extension-tests.log').write_text('checks retained without build dependencies')
            plan=dict(workflows=str(raw),profiles=str(raw),receipts={},
                      core=dict(inventory=str(inventory),reports=[]),booking_resolution=str(diagnosis))
            result=completion_qualification.input_inventory(plan)
            self.assertTrue(any(row['path'].endswith('extension-tests.log') for row in result))
            self.assertFalse(any('private-extension' in row['path'] for row in result))
            # The original in-evidence staging layout must remain rejected.
            (raw/'node_modules').symlink_to(modules,target_is_directory=True)
            with self.assertRaisesRegex(ValueError,'mutable symlink'):
                completion_qualification.input_inventory(plan)

    def test_relative_software_cli_keeps_gates_absolute_and_stage_outside_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve();tools=root/'tools';tools.mkdir()
            scripts=root/'scripts';scripts.mkdir()
            for name in ('oryn-python','intentgym','crates/oryn-scanner','extension-w/node_modules','extension-w/wasm'):
                (root/name).mkdir(parents=True)
            wasm=root/'extension-w/wasm/original.wasm';wasm.write_bytes(b'frozen-wasm')
            calls=root/'calls.jsonl'
            dispatcher='#!'+sys.executable+'\n'+r'''
import json, os, pathlib, subprocess, sys
root=pathlib.Path(os.environ['FIXTURE_ROOT']);args=sys.argv[1:]
with (root/'calls.jsonl').open('a') as stream:stream.write(json.dumps([pathlib.Path(sys.argv[0]).name,*args])+'\n')
if pathlib.Path(sys.argv[0]).name=='python3':
    if args[0]=='-':
        raise SystemExit(subprocess.run([sys.executable,*args]).returncode)
    if pathlib.Path(args[0]).name=='generation_gate.py':
        if pathlib.Path(args[0]).resolve()!=root/'scripts/generation_gate.py':raise SystemExit(91)
    elif pathlib.Path(args[0]).name!='security_prebuilt.py':raise SystemExit(92)
raise SystemExit(0)
'''
            for name in ('python3','cargo','poetry','node'):
                path=tools/name;path.write_text(dispatcher);path.chmod(0o755)
            for name in ('check-scanner-sync.sh','build-wasm.sh'):
                path=scripts/name;path.write_text('#!/bin/sh\nexit 0\n');path.chmod(0o755)
            source=prebuilt.ROOT/'scripts/validate-app-compatibility-software.sh'
            script=scripts/source.name;shutil.copy2(source,script);script.chmod(0o755)
            artifacts=root/'evidence';artifacts.mkdir()
            env=dict(os.environ,PATH=str(tools)+os.pathsep+os.environ['PATH'],FIXTURE_ROOT=str(root),
                     ORYN_NODE_BIN=str(tools/'node'),ORYN_APP_COMPATIBILITY_ARTIFACT_DIR=str(artifacts),
                     ORYN_SECURITY_PREBUILT_CANDIDATE='/fixture/candidate',
                     ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256='a'*64)
            result=subprocess.run(['bash','./scripts/'+source.name],cwd=root,env=env,
                                  capture_output=True,text=True,timeout=15)
            self.assertEqual(result.returncode,0,result.stdout+'\n'+result.stderr)
            self.assertEqual(wasm.read_bytes(),b'frozen-wasm')
            self.assertFalse(any(path.is_symlink() for path in artifacts.rglob('*')))
            stages=[Path(row[3]) for row in map(json.loads,calls.read_text().splitlines())
                    if row[:2]==['python3','-']]
            self.assertEqual(len(stages),1)
            self.assertFalse(artifacts in stages[0].parents)
            self.assertFalse(stages[0].exists())

    def test_live_signature_rejects_invalid_signature_certificate_and_entitlements(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cert = b'current-leaf-certificate'
            identity = hashlib.sha1(cert).hexdigest().upper()
            manifest = root/'manifest.json'; manifest.write_text(json.dumps({'signing_cert_sha1':identity}))
            candidate = {'artifacts':{'worker':root/'OrynPageWorker.app/Contents/MacOS/oryn-page-worker',
                                    'worker_manifest':manifest}}
            state = {'details':'Identifier=ai.rustic.oryn.page-worker\nTeamIdentifier=5HVA9VFF8K\n'
                                'Authority=Apple Development: Test\n'
                                'Authority=Apple Worldwide Developer Relations Certification Authority\n'
                                'flags=0x10000(runtime)',
                     'entitlements':{'com.apple.security.app-sandbox':True,'com.apple.security.cs.allow-jit':True},
                     'cert':cert}
            def codesign(command, **kwargs):
                self.assertEqual(command[0],'codesign')
                self.assertNotIn('--sign',command)
                self.assertNotIn('--force',command)
                for arg in command:
                    if arg.startswith('--extract-certificates='):
                        Path(arg.split('=',1)[1]+'0').write_bytes(state['cert'])
                return SimpleNamespace(stdout=plistlib.dumps(state['entitlements']),stderr=state['details'])
            with patch.dict(os.environ,{},clear=True):
                prebuilt.verify_signature(candidate,run=codesign)
                failing = Mock(side_effect=subprocess.CalledProcessError(1,['codesign','--verify']))
                with self.assertRaises(subprocess.CalledProcessError):prebuilt.verify_signature(candidate,run=failing)
                state['cert'] = b'wrong-leaf'
                with self.assertRaisesRegex(ValueError,'certificate'):prebuilt.verify_signature(candidate,run=codesign)
                state['cert'] = cert
                state['entitlements']['com.apple.security.network.client'] = True
                with self.assertRaisesRegex(ValueError,'entitlements'):prebuilt.verify_signature(candidate,run=codesign)
                state['entitlements'].pop('com.apple.security.network.client')
                state['details'] = state['details'].replace('TeamIdentifier=5HVA9VFF8K','TeamIdentifier=wrong')
                with self.assertRaisesRegex(ValueError,'identity/chain/runtime'):prebuilt.verify_signature(candidate,run=codesign)
                with patch.dict(os.environ,{'ORYN_CODESIGN_IDENTITY':'wrong'}):
                    with self.assertRaisesRegex(ValueError,'configured signing'):prebuilt.verify_signature(candidate,run=codesign)

    def test_frozen_example_executes_bound_binary_without_cargo_and_rechecks_after(self):
        candidate = {'artifacts':{'worker':Path('/frozen/worker'), 'page_benchmark':Path('/frozen/page')}}
        with patch.object(prebuilt,'bind',return_value=(candidate,{})) as binding, \
             patch.object(prebuilt.subprocess,'run',return_value=SimpleNamespace(returncode=0)) as run, \
             patch.object(sys,'argv',['security_prebuilt.py','run','sandbox_page_benchmark','--','5','10000']):
            self.assertEqual(prebuilt.main(),0)
            self.assertEqual(run.call_args.args[0],['/frozen/page','5','10000'])
            self.assertEqual(binding.call_count,2)
            self.assertEqual(run.call_args.kwargs['env']['ORYN_PAGE_WORKER'],'/frozen/worker')
        with patch.object(prebuilt,'bind',side_effect=[(candidate,{}),ValueError('frozen bytes changed')]), \
             patch.object(prebuilt.subprocess,'run',return_value=SimpleNamespace(returncode=0)), \
             patch.object(sys,'argv',['security_prebuilt.py','run','sandbox_page_benchmark']):
            with self.assertRaisesRegex(ValueError,'frozen bytes changed'):prebuilt.main()

    def test_runner_selects_prebuilt_route_with_exact_candidate_digest(self):
        root = completion_execution.ROOT
        candidate = dict(path=Path('/frozen/candidate.json'),sha256='f'*64,
                         artifacts={'runtime':root/'target/release/oryn',
                                    'worker':root/'target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'})
        for artifacts in (candidate['artifacts'],
                          {'runtime':Path('/external/copied-release/oryn'),
                           'worker':Path('/external/copied-release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker')}):
            candidate['artifacts'] = artifacts
            command = completion_execution.command('security',candidate,'/gate','/output')
            self.assertEqual(command,[str(root/'scripts/run-security-suite.sh'),'/output/raw',
                                      '--prebuilt-candidate','/frozen/candidate.json','--candidate-sha256','f'*64])

    def test_validated_worker_shell_path_never_builds_signs_or_rewrites_frozen_inputs(self):
        # The shell control-flow integration uses fake tools. The real helper's
        # negative signature/provenance cases above remain independently tested.
        if not shutil.which('bash') or not shutil.which('shasum'):
            self.skipTest('shell fixture requires bash and shasum')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); tools = root/'tools'; tools.mkdir()
            scripts = root/'scripts'; scripts.mkdir()
            frozen = root/'frozen'; frozen.mkdir()
            app = frozen/'OrynPageWorker.app'
            (app/'Contents/MacOS').mkdir(parents=True)
            (app/'Contents/Resources').mkdir()
            cert = b'fixture-certificate'; identity = hashlib.sha1(cert).hexdigest().upper()
            paths = {'runtime':frozen/'oryn','engine_worker':frozen/'engine-worker',
                     'worker':app/'Contents/MacOS/oryn-page-worker',
                     'worker_manifest':app/'Contents/Resources/worker-manifest.json',
                     'engine_profile':frozen/'profile.json'}
            config = {'paths':{k:str(v) for k,v in paths.items()},'identity':identity,
                      'cert':cert.decode(),'calls':str(root/'calls.jsonl')}
            (root/'config.json').write_text(json.dumps(config))
            dispatcher = '#!' + sys.executable + '\n' + r'''
import json, os, pathlib, plistlib, sys, time
cfg=json.loads(pathlib.Path(os.environ['FIXTURE_CONFIG']).read_text())
name=pathlib.Path(sys.argv[0]).name; args=sys.argv[1:]
with open(cfg['calls'],'a') as f: f.write(json.dumps([name,*args])+'\n')
if name=='python3':
    if args[0]=='-c':
        pathlib.Path(args[-1]).write_text('12345')
        time.sleep(30)
    elif pathlib.Path(args[0]).name=='native_protocol_versions.py': print('1')
    elif pathlib.Path(args[0]).name=='security_prebuilt.py':
        if args[1]=='path': print(cfg['paths'][args[2]])
        else: print('{}')
    sys.exit(0)
if name in ('cargo','cp'): sys.exit(95)
if name=='uname': print('Darwin'); sys.exit(0)
if name=='rustc': print('host: aarch64-apple-darwin'); sys.exit(0)
if name=='security': print(cfg['identity']); sys.exit(0)
if name=='jq':
    if '-r' in args:
        key=args[args.index('-r')+1]
        print(cfg['paths']['engine_profile'] if key.endswith('_path') else 'a'*64)
        sys.exit(0)
    # Consume the producer pipe before exiting, just as jq does. Otherwise
    # plutil can fail with BrokenPipeError under the shell's pipefail mode.
    value=sys.stdin.read()
    if any('com.apple.security.app-sandbox' in arg for arg in args):
        entitlements=json.loads(value)
        expected={'com.apple.security.app-sandbox':True,'com.apple.security.cs.allow-jit':True}
        sys.exit(0 if set(entitlements)==set(expected) and all(entitlements[key] is True for key in expected) else 1)
    sys.exit(0)
if name=='plutil':
    if '-convert' in args:
        print(json.dumps(plistlib.loads(pathlib.Path(args[-1]).read_bytes())))
    sys.exit(0)
if name=='PlistBuddy':
    if args[1] in ('Print :com.apple.security.app-sandbox','Print :com.apple.security.cs.allow-jit'):
        print('true'); sys.exit(0)
    sys.exit(1)
if name=='codesign':
    if '--force' in args or '--sign' in args: sys.exit(96)
    for arg in args:
        if arg.startswith('--extract-certificates='):
            pathlib.Path(arg.split('=',1)[1]+'0').write_bytes(cfg['cert'].encode())
    if '-dvvv' in args:
        print('Identifier=ai.rustic.oryn.page-worker\nAuthority=Apple Development: Fixture\nAuthority=Apple Worldwide Developer Relations Certification Authority\nTeamIdentifier=5HVA9VFF8K\nflags=0x10000(runtime)',file=sys.stderr)
    elif '--entitlements' in args:
        sys.stdout.buffer.write(plistlib.dumps({'com.apple.security.app-sandbox':True,'com.apple.security.cs.allow-jit':True}))
    sys.exit(0)
if name=='oryn':
    if args==['webdriver','--help'] and cfg.get('webdriver_exit'):
        print('fixture: unrecognized subcommand webdriver',file=sys.stderr)
        sys.exit(cfg['webdriver_exit'])
    print('Brokered' if '--evaluate' in args else '{}'); sys.exit(0)
if name in ('engine-worker','oryn-page-worker'):
    if args: print('{}')
    else:
        sys.stdin.read(); print('{"status":"pong"}\n{"status":"exiting"}')
    sys.exit(0)
sys.exit(97)
'''
            dispatcher=dispatcher.replace("os.environ['FIXTURE_CONFIG']",repr(str(root/'config.json')))
            for name in ('python3','cargo','cp','uname','rustc','security','jq','plutil','PlistBuddy','codesign'):
                path=tools/name; path.write_text(dispatcher); path.chmod(0o755)
            for name in ('runtime','engine_worker','worker'):
                paths[name].write_text(dispatcher);paths[name].chmod(0o755)
            paths['worker_manifest'].write_text('{}')
            paths['engine_profile'].write_text('{}')
            (app/'Contents/Info.plist').write_text('immutable signed plist')
            source=(prebuilt.ROOT/'scripts/build-sandboxed-worker.sh').read_text()
            source=source.replace('/usr/libexec/PlistBuddy',str(tools/'PlistBuddy'))
            script=scripts/'build-sandboxed-worker.sh';script.write_text(source)
            before={str(p):prebuilt.digest(p) for p in frozen.rglob('*') if p.is_file()}
            environment=dict(os.environ,PATH=str(tools)+os.pathsep+os.environ['PATH'],
                FIXTURE_CONFIG=str(root/'config.json'),ORYN_SECURITY_PREBUILT_CANDIDATE='/fixture/candidate',
                ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256='a'*64,ORYN_CODESIGN_IDENTITY=identity,
                CARGO_TARGET_DIR=str(root/'must-remain-absent'))
            result=subprocess.run(['bash',str(script)],env=environment,cwd=root,capture_output=True,text=True,timeout=15)
            self.assertEqual(result.returncode,0,result.stdout+'\n'+result.stderr)
            self.assertEqual(before,{str(p):prebuilt.digest(p) for p in frozen.rglob('*') if p.is_file()})
            self.assertFalse((root/'must-remain-absent').exists())
            calls=[json.loads(line) for line in (root/'calls.jsonl').read_text().splitlines()]
            self.assertFalse(any(call[0] in ('cargo','cp') for call in calls))
            self.assertFalse(any(call[0]=='codesign' and ('--force' in call or '--sign' in call) for call in calls))
            self.assertTrue(any(call[:2]==['codesign','--verify'] for call in calls))
            self.assertIn(['oryn','webdriver','--help'], calls)

            # A validated frozen CLI can still lack the compiled WebDriver
            # role. Preserve its original exit/error and reject it before any
            # signature action or worker launch, without rebuilding the inputs.
            config['webdriver_exit'] = 64
            (root/'config.json').write_text(json.dumps(config))
            (root/'calls.jsonl').write_text('')
            rejected=subprocess.run(['bash',str(script)],env=environment,cwd=root,
                                    capture_output=True,text=True,timeout=15)
            self.assertEqual(rejected.returncode,64,rejected.stdout+'\n'+rejected.stderr)
            self.assertIn('fixture: unrecognized subcommand webdriver',rejected.stderr)
            self.assertEqual(before,{str(p):prebuilt.digest(p) for p in frozen.rglob('*') if p.is_file()})
            self.assertFalse((root/'must-remain-absent').exists())
            rejected_calls=[json.loads(line) for line in (root/'calls.jsonl').read_text().splitlines()]
            self.assertIn(['oryn','webdriver','--help'],rejected_calls)
            self.assertFalse(any(call[0] in ('cargo','cp','codesign','engine-worker','oryn-page-worker')
                                 for call in rejected_calls))



class SelectedSecurityBundleTests(unittest.TestCase):
    def test_selected_staged_worker_rejects_stale_source_and_profile(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            worker = root/'copied-release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
            worker.parent.mkdir(parents=True); worker.write_bytes(b'signed worker')
            resources = worker.parents[1]/'Resources'; resources.mkdir()
            profile = root/'profile.json'
            profile.write_text(json.dumps({'worker_sha256': 'a'*64, 'worker_size_bytes': 19}))
            (resources/'v8-profile-manifest.json').write_bytes(profile.read_bytes())
            manifest = resources/'worker-manifest.json'
            value = {'source_tree_sha256':'b'*64, 'v8_profile_sha256':prebuilt.digest(profile),
                     'worker_build_sha256':'a'*64, 'worker_build_bytes':19}
            manifest.write_text(json.dumps(value))
            with patch.object(prebuilt, 'verify_signature') as verify:
                self.assertEqual(prebuilt.selected_worker_bundle(worker, profile, 'b'*64),
                                 (worker, manifest))
                verify.assert_called_once()
                for field, replacement in [('source_tree_sha256','c'*64),
                                           ('v8_profile_sha256','c'*64),
                                           ('worker_build_sha256','c'*64),
                                           ('worker_build_bytes',20)]:
                    changed = dict(value); changed[field] = replacement
                    manifest.write_text(json.dumps(changed))
                    with self.assertRaisesRegex(ValueError,'identity mismatch'):
                        prebuilt.selected_worker_bundle(worker, profile, 'b'*64)
                manifest.write_text(json.dumps(value))
                (resources/'v8-profile-manifest.json').write_text('{}')
                with self.assertRaisesRegex(ValueError,'identity mismatch'):
                    prebuilt.selected_worker_bundle(worker, profile, 'b'*64)

    def test_frozen_candidate_accepts_external_staging_but_rejects_bundle_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            lease, candidate, value = SecurityPrebuiltTests().preparation(directory)
            original_worker = candidate['artifacts']['worker']
            original_app = original_worker.parents[2]
            staged = Path(directory).resolve()/'copied-release'
            staged.mkdir(); shutil.move(str(original_app), str(staged/'OrynPageWorker.app'))
            worker = staged/'OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
            candidate['artifacts'].update(runtime=staged/'oryn',worker=worker,
                worker_manifest=worker.parents[1]/'Resources/worker-manifest.json')
            value['bundle_files'] = prebuilt.bundle_inventory(worker)
            candidate['record']['artifacts'].update(prebuilt.preparation_artifacts(value))
            candidate['artifacts']['security_prebuilt'].write_text(json.dumps(value))
            environment = {prebuilt.CANDIDATE_ENV:str(candidate['path']),
                prebuilt.DIGEST_ENV:candidate['sha256'],
                'CARGO_TARGET_DIR':str(Path(directory).parent/(Path(directory).name+'-checks'))}
            with patch.dict(os.environ,environment,clear=True), \
                 patch.object(prebuilt,'inherited_generation_lease',return_value=lease), \
                 patch.object(prebuilt,'load_candidate',return_value=candidate), \
                 patch.object(prebuilt,'source_hash',return_value='c'*64):
                actual, _ = prebuilt.bind()
                self.assertEqual(actual['artifacts']['worker'], worker)
                (worker.parents[1]/'Resources/late-resource').write_text('unbound')
                with self.assertRaisesRegex(ValueError,'file set or bytes changed'):
                    prebuilt.bind()

if __name__ == '__main__':
    unittest.main()
