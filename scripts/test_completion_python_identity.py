"""Candidate-bound command identity; no producer or interpreter is executed."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import completion_execution as execution

class PythonIdentityTests(unittest.TestCase):
    def fixture(self, root):
        real=root/'base-python';real.write_bytes(b'pinned interpreter fixture')
        alias=root/'venv/bin/python';alias.parent.mkdir(parents=True);alias.symlink_to(real)
        contract=root/'controls.json';contract.write_text(json.dumps({'upstream_python':{'path':str(alias),'sha256':execution.digest(real)}}))
        candidate_path=root/'candidate.json';candidate_path.write_text('{}')
        candidate=dict(path=candidate_path,sha256=execution.digest(candidate_path),record={'source_tree_sha256':'a'*64},
            artifacts={'native_controls_contract':contract,'runtime':root/'runtime','worker':root/'worker',
                       'framework_build':root/'framework','workflow_executor':root/'executor'})
        return candidate,alias,real,contract
    def test_all_python_producers_keep_declared_alias_across_verifier_interpreters(self):
        with tempfile.TemporaryDirectory() as name:
            candidate,alias,real,_=self.fixture(Path(name).resolve())
            for kind in ('fixture-topology','native-media','native-prerequisites','frameworks','reliability','architecture'):
                with self.subTest(kind=kind),patch('sys.executable',str(alias)):
                    original=execution.command(kind,candidate,'/gate','/output')
                with patch('sys.executable',str(real)):
                    reconstructed=execution.command(kind,candidate,'/gate','/output')
                self.assertEqual(original,reconstructed);self.assertEqual(original[0],str(alias))
                self.assertNotEqual(original[0],str(alias.resolve()))
    def test_relative_missing_nonfile_and_changed_interpreter_are_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            candidate,alias,real,contract=self.fixture(Path(name).resolve())
            original=json.loads(contract.read_text())
            for path in ('relative/python',str(real.parent/'missing'),str(real.parent)):
                value=copy.deepcopy(original);value['upstream_python']['path']=path;contract.write_text(json.dumps(value))
                with self.subTest(path=path),self.assertRaises(ValueError):execution.candidate_python(candidate)
            contract.write_text(json.dumps(original));real.write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError,'changed'):execution.candidate_python(candidate)
    def receipt(self,root,candidate,argv):
        out=root/'component';out.mkdir();gate=root/'gate.json';gate.write_text('{}')
        before=dict(kind='native-prerequisites',candidate_manifest_sha256=candidate['sha256'],
            source_tree_sha256=candidate['record']['source_tree_sha256'],candidate=execution.reference(candidate['path']),
            generation_gate=execution.reference(gate),producer_source=execution.reference(execution.ROOT/'scripts/completion_execution.py'),argv=argv)
        pre=out/'preflight.json';pre.write_text(json.dumps(before))
        receipt=dict(schema_version=1,kind='native-prerequisites',candidate_manifest_sha256=candidate['sha256'],
            preflight=execution.reference(pre),execution=dict(argv=argv,returncode=0,timed_out=False,error=None,cleanup_complete=True),
            producer_completed=True,errors=[],artifacts=[execution.reference(pre)])
        path=out/'receipt.json';path.write_text(json.dumps(receipt));return path
    def test_real_receipt_join_accepts_different_verifier_interpreter(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();candidate,alias,real,_=self.fixture(root)
            argv=execution.command('native-prerequisites',candidate,root/'gate.json',root/'component')
            receipt=self.receipt(root,candidate,argv)
            with patch('sys.executable',str(real)):
                self.assertTrue(execution.verify_receipt(receipt,'native-prerequisites',candidate)['producer_completed'])
    def test_real_receipt_join_rejects_resolved_base_substitution(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();candidate,alias,real,_=self.fixture(root)
            argv=execution.command('native-prerequisites',candidate,root/'gate.json',root/'component');argv[0]=str(real)
            receipt=self.receipt(root,candidate,argv)
            with self.assertRaisesRegex(ValueError,'substituted'):execution.verify_receipt(receipt,'native-prerequisites',candidate)

if __name__=='__main__':unittest.main()
