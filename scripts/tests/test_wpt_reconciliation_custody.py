"""Reject after-use input replacement in the public Core reconciliation CLI."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(SCRIPTS))
spec=importlib.util.spec_from_file_location('core_reconcile_cli',SCRIPTS/'reconcile-full-wpt-qualification.py')
cli=importlib.util.module_from_spec(spec);spec.loader.exec_module(cli)


class ReconciliationCustodyTests(unittest.TestCase):
    def test_conflicting_late_digest_cannot_replace_initial_identity(self):
        inputs={'/original':'a'*64}
        with self.assertRaisesRegex(ValueError,'identity changed'):
            cli.merge_inputs(inputs,{'/original':'b'*64})
        self.assertEqual(inputs,{'/original':'a'*64})

    def test_mutation_during_reconcile_preserves_and_rechecks_original_hash(self):
        for changed in ('assessments.json','attribution.json','execution.json','preflight.json','protocol-custody.jsonl','assessment-destination'):
            with self.subTest(changed=changed),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);inventory=root/'inventory';inventory.mkdir();run=root/'run';run.mkdir()
                files={}
                for name in ('plan.json','coverage.jsonl','manifest.json','policy.json','reviews.json','assessments.json','attribution.json'):
                    base=inventory if name in ('plan.json','coverage.jsonl') else root
                    files[name]=base/name;files[name].write_text('{}\n')
                for name in ('report.json','execution.json','preflight.json','protocol-custody.jsonl'):
                    files[name]=run/name;files[name].write_text('{}\n')
                if changed == 'assessment-destination':
                    original=root/'original-assessments.json';files['assessments.json'].rename(original)
                    replacement=root/'replacement-assessments.json';replacement.write_text('{}\n')
                    files['assessments.json'].symlink_to(original)
                args=['reconcile-full-wpt-qualification.py','--allow-historical','--inventory',str(inventory),
                    '--manifest',str(files['manifest.json']),'--policy',str(files['policy.json']),
                    '--checkout',str(root),'--report',str(files['report.json']),
                    '--reviews',str(files['reviews.json']),'--assessments',str(files['assessments.json']),
                    '--attribution',str(files['attribution.json']),'--candidate-sha256','a'*64,'--output',str(root/'out')]
                def mutate(*args,**kwargs):
                    if changed != 'assessment-destination':
                        files[changed].write_text('{"changed_after_use":true}\n')
                    return [],dict(complete=False,inputs={})
                def verify(*args,**kwargs):
                    if changed == 'assessment-destination':
                        files['assessments.json'].unlink();files['assessments.json'].symlink_to(replacement)
                    return {'returncode':0}
                with patch.object(sys,'argv',args),patch.object(cli,'validate_inventory',return_value={}),\
                     patch.object(cli,'bind_inventory'),patch.object(cli,'verify_execution',side_effect=verify),\
                     patch.object(cli.wpt_source_matrix,'CORPORA',[]),patch.object(cli.wpt_source_matrix,'validate',return_value=[]),\
                     patch.object(cli,'reconcile',side_effect=mutate),self.assertRaisesRegex(ValueError,'changed'):
                    cli.main()
                self.assertFalse((root/'out').exists())
