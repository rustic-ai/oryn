"""Both production callers must select attested signed-worker, not CLI identity."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import ExitStack
from unittest.mock import patch

import completion_qualification
import core_profile_contract
import wpt_canary_qualification
import wpt_execution_evidence

sys.path.insert(0, str(Path(__file__).resolve().parent/'tests'))
import test_wpt_shared_placement as placement_fixture


def cli_module():
    path=Path(__file__).resolve().parent/'qualify-wpt-shared-placement.py'
    spec=importlib.util.spec_from_file_location('placement_identity_cli',path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


class PlacementWorkerIdentityTests(unittest.TestCase):
    def evidence(self, root):
        record, report, context, worker = placement_fixture.PlacementTests().evidence()
        for name,value in [('placement.json',record),('wptreport.json',report),('placement-context.json',context)]:
            (root/name).write_text(json.dumps(value))
        (root/'mozlog.jsonl').write_text('')
        (root/'workflow-lock.json').write_text(json.dumps({'inputs':{}}))
        return context, worker

    def run_cli(self, wrong_worker=False):
        module=cli_module()
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,worker=self.evidence(root)
            cli='9'*64
            self.assertNotEqual(cli,worker)
            artifacts={'runtime':{'sha256':cli},'worker':{'sha256':cli if wrong_worker else worker}}
            bound={'candidate_sha256':context['candidate_sha256'], 'sha256':context['run_contract_sha256'],
                   'candidate':{'artifacts':artifacts}}
            execution=dict(collection='placement',returncode=0,launch_error=None,
                candidate_sha256=context['candidate_sha256'],workflow_lock_sha256=module.sha256(root/'workflow-lock.json'),
                artifacts={name:module.sha256(root/name) for name in ('placement-context.json','placement.json','wptreport.json','mozlog.jsonl')})
            argv=['qualify-placement','--generation-gate',str(root/'gate'), '--run-contract',str(root/'contract'),
                  '--prepared',str(root),'--execution',str(root),'--output',str(root/'result.json')]
            with patch.object(module,'generation_gate'),patch.object(module,'load_contract',return_value=bound), \
                 patch.object(module,'verify_execution',return_value=execution),patch.object(sys,'argv',argv):
                if wrong_worker:
                    with self.assertRaisesRegex(ValueError,'candidate child process'):module.main()
                    self.assertFalse((root/'result.json').exists())
                else:
                    module.main()
                    result=json.loads((root/'result.json').read_text())
                    self.assertTrue(result['qualification_accepted'])
                    self.assertEqual(set(result['witnesses']),{'fixed','growable','wasm'})

    def test_cli_accepts_distinct_attested_signed_worker(self):
        self.assertEqual(unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__]).countTestCases(),4)
        self.run_cli()

    def test_cli_rejects_parent_digest_as_worker(self):self.run_cli(wrong_worker=True)

    def run_combined(self, wrong_worker=False):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,worker=self.evidence(root);cli='9'*64
            self.assertNotEqual(cli,worker)
            audit=root/'audit.json';audit.write_text('{}')
            profile=dict(run_contract=str(root/'contract'),prepared=str(root),
                canaries=dict(directory=str(root),audit=str(audit)),placement=dict(directory=str(root)))
            invocation=dict(profile_id='standard-block',envelope_sha256=context['run_contract_sha256'])
            candidate=dict(sha256=context['candidate_sha256'],record=dict(artifacts={
                'runtime':dict(sha256=cli),'worker':dict(sha256=cli if wrong_worker else worker)}))
            envelope=dict(envelope=dict(prepared=str(root)))
            with ExitStack() as stack:
                stack.enter_context(patch.object(core_profile_contract,'resolve_envelope',return_value=envelope))
                stack.enter_context(patch.object(core_profile_contract,'identity',return_value={}))
                stack.enter_context(patch.object(core_profile_contract,'check_identity'))
                stack.enter_context(patch.object(wpt_execution_evidence,'verify',return_value=dict(returncode=0,profile_context={})))
                stack.enter_context(patch.object(wpt_canary_qualification,'qualify',return_value=dict(controls_rejected=True)))
                # The actual native placement verifier remains unmocked in both callers.
                if wrong_worker:
                    with self.assertRaisesRegex(ValueError,'candidate child process'):
                        completion_qualification.profile_controls(profile,invocation,candidate)
                else:
                    result=completion_qualification.profile_controls(profile,invocation,candidate)
                    self.assertTrue(result['placement']['qualification_accepted'])
                    self.assertEqual(set(result['placement']['witnesses']),{'fixed','growable','wasm'})

    def test_combined_accepts_distinct_attested_signed_worker(self):self.run_combined()

    def test_combined_rejects_parent_digest_as_worker(self):self.run_combined(wrong_worker=True)


if __name__=='__main__':unittest.main()
