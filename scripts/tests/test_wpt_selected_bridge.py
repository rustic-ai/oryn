import unittest
import tempfile
import hashlib
import json
from pathlib import Path
from wpt_selected_bridge import execution_url, reconcile, prepare

class SelectedBridgeTests(unittest.TestCase):
    def test_exact_worker_variant_urls(self):
        self.assertEqual(execution_url('x.any.js','serviceworker','?a=1'),'/x.any.serviceworker.html?a=1')
        self.assertEqual(execution_url('x.any.js','sharedworker',''),'/x.any.sharedworker.html')
        self.assertEqual(execution_url('x.window.js','window',''),'/x.window.html')
        with self.assertRaises(ValueError): execution_url('x.html','dedicatedworker','')

    def test_missing_required_rows_remain_missing(self):
        mapping={'executions':[{'id':'support|window|','url':'/support.html'}, {'id':'a|window|','url':'/a.html'}]}
        result=reconcile(mapping,{'results':[{'test':'/a.html','status':'FAIL','subtests':[{'status':'FAIL'}]}]})
        self.assertFalse(result['complete'])
        self.assertEqual(result['missing_ids'],['support|window|'])
        self.assertEqual(result['results'][0]['status'],'FAIL')
        with self.assertRaises(ValueError):reconcile(mapping,{'results':[{'test':'/a.html'},{'test':'/a.html'}]})

    def fixture(self, root, path='a.html', kind='testharness', manifest_path=None):
        data=b'<!doctype html>fixture'
        file=root/path
        file.parent.mkdir(parents=True,exist_ok=True)
        file.write_bytes(data)
        source={'path':path,'sha256':hashlib.sha256(data).hexdigest(),'globals':['window'],'variants':['']}
        inventory={'tests':[source],'required_execution_total':1,'upstream_revision':'pinned'}
        key=manifest_path or path
        tree={}
        cursor=tree
        parts=key.split('/')
        for part in parts[:-1]:cursor=cursor.setdefault(part,{})
        blob=hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()
        cursor[parts[-1]]=[blob,[None,{}]]
        manifest={'version':9,'items':{kind:tree}}
        inv=root/'inventory.json';man=root/'manifest.json';ledger=root/'ledger.json'
        inv.write_text(json.dumps(inventory));man.write_text(json.dumps(manifest))
        ledger.write_text(json.dumps({'assertions':[{'id':path+'|window|'}]}))
        return inv,man,ledger,inventory,manifest

    def test_source_alias_and_hash_authority(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            inv,man,ledger,inventory,manifest=self.fixture(root,'WebIDL/a.html',manifest_path='webidl/a.html')
            rows=prepare(inv,man,root,ledger)['executions']
            self.assertEqual(rows[0]['url'],'/webidl/a.html');self.assertTrue(rows[0]['runnable'])
            (root/'WebIDL/a.html').write_bytes(b'changed')
            with self.assertRaises(ValueError):prepare(inv,man,root,ledger)
            (root/'WebIDL/a.html').write_bytes(b'<!doctype html>fixture')
            manifest['items']['testharness']['webidl']['a.html'][0]='wrong'
            man.write_text(json.dumps(manifest))
            row=prepare(inv,man,root,ledger)['executions'][0]
            self.assertFalse(row['runnable']);self.assertEqual(row['mapping_error'],'pinned_manifest_source_missing_or_changed')

    def test_required_support_and_crash_retained(self):
        for kind in ('support','crashtest'):
            with tempfile.TemporaryDirectory() as directory:
                root=Path(directory);inv,man,ledger,_,_=self.fixture(root,kind=kind)
                mapping=prepare(inv,man,root,ledger)
                self.assertEqual(mapping['execution_total'],1);self.assertEqual(mapping['runnable_total'],0)
                self.assertEqual(reconcile(mapping,{'results':[]})['missing_ids'],['a.html|window|'])

    def test_duplicates_population_and_missing_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);inv,man,ledger,inventory,manifest=self.fixture(root)
            ledger.write_text(json.dumps({'assertions':[]}))
            with self.assertRaises(ValueError):prepare(inv,man,root,ledger)
            inventory['tests']*=2;inventory['required_execution_total']=2
            inv.write_text(json.dumps(inventory))
            with self.assertRaises(ValueError):prepare(inv,man,root)
            inventory['tests']=inventory['tests'][:1];inventory['required_execution_total']=1
            inv.write_text(json.dumps(inventory));manifest['items']={};man.write_text(json.dumps(manifest))
            self.assertFalse(prepare(inv,man,root)['executions'][0]['runnable'])
        for rows in ([{'id':'a','url':'/a'},{'id':'b','url':'/a'}],[{'id':'a','url':'/a'},{'id':'a','url':'/b'}]):
            with self.assertRaises(ValueError):reconcile({'executions':rows},{'results':[]})

    def test_paths_and_symlink_escape_rejected(self):
        for path in ('/a.html','../a.html','a/../b.html','a//b.html','a\\b.html','a.html?x'):
            with self.assertRaises(ValueError):execution_url(path,'window','')
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            root=Path(directory);inv,man,_,_,_=self.fixture(root)
            (root/'a.html').unlink();(Path(outside)/'a.html').write_text('outside')
            (root/'a.html').symlink_to(Path(outside)/'a.html')
            with self.assertRaises(ValueError):prepare(inv,man,root)

    def test_complete_is_not_passing(self):
        mapping={'executions':[{'id':'a','url':'/a'}]}
        result=reconcile(mapping,{'results':[{'test':'/a','status':'OK','subtests':[{'status':'FAIL'}]}]})
        self.assertTrue(result['url_presence_complete']);self.assertFalse(result['selected_results_passed'])
        self.assertEqual(result['qualification'],'not_evaluated')
        with self.assertRaises(ValueError):reconcile(mapping,{'results':[{'test':'/extra'}]})

    def test_empty_assertions_and_nonrunnable_cannot_pass(self):
        mapping={'executions':[{'id':'a','url':'/a','runnable':True},{'id':'support','url':'/support','runnable':False}]}
        result=reconcile(mapping,{'results':[{'test':'/a','status':'OK','subtests':[]}]})
        self.assertFalse(result['selected_results_passed']);self.assertEqual(result['no_assertions_ids'],['a'])
        self.assertEqual(result['missing_ids'],['support'])
        self.assertEqual(result['required_dispositions'][0]['id'],'support')
        with self.assertRaises(ValueError):reconcile(mapping,{'results':[{'test':'/support','status':'OK','subtests':[{'status':'PASS'}]}]})

    def test_report_custody_preserved_and_validated(self):
        mapping={'upstream_revision':'pinned','inventory_sha256':'inventory','manifest_sha256':'manifest','executions':[{'id':'a','url':'/a','runnable':True}]}
        info={'oryn_wpt_revision':'pinned','oryn_executor':'upstream-wptrunner','oryn_candidate_sha256':'candidate','oryn_run_contract_sha256':'contract','run_id':'run'}
        report={'run_info':info,'results':[{'test':'/a','status':'OK','subtests':[{'status':'PASS'}]}]}
        result=reconcile(mapping,report,'rawhash')
        self.assertEqual(result['run_info'],info);self.assertEqual(result['report_sha256'],'rawhash')
        self.assertEqual(result['mapping_inventory_sha256'],'inventory');self.assertTrue(result['selected_results_passed'])
        for field,value in (('oryn_wpt_revision','other'),('oryn_executor','historical')):
            bad=dict(report,run_info=dict(info,**{field:value}))
            with self.assertRaises(ValueError):reconcile(mapping,bad)

if __name__=='__main__':unittest.main()
