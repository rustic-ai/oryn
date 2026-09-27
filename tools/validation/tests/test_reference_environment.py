"""Reference custody regressions use only temporary fake apps; never Chrome."""
import copy
import json
import os
from pathlib import Path
import plistlib
import stat
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import reference_environment as subject
from oryn_validation.inputs import InputError, snapshot_input

SIGNATURE={'identifier':'test.reference','team':'TESTTEAM','cdhash':'a'*40,'authorities':['Test Authority']}
SYSTEM={'macos_build':'TEST','os_shared_cache_names':['/usr/lib/libSystem.B.dylib'],'internal_loads':[]}


class ReferenceEnvironmentTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve();self.store=self.root/'store';self.store.mkdir()
        self.app=self.root/'Reference.app'
        binary=self.app/'Contents/MacOS/Reference';binary.parent.mkdir(parents=True)
        binary.write_bytes(b'fake executable');binary.chmod(0o755)
        (self.app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleExecutable':'Reference'}))
        framework=self.app/'Contents/Frameworks/Test.framework';(framework/'Versions/A/Resources').mkdir(parents=True)
        (framework/'Versions/A/Test').write_bytes(b'framework bytes')
        (framework/'Versions/A/Resources/asset').write_bytes(b'original')
        (framework/'Versions/Current').symlink_to('A')
        (framework/'Test').symlink_to('Versions/Current/Test')
        (framework/'Resources').symlink_to('Versions/Current/Resources')
        self.driver=self.root/'chromedriver';self.driver.write_bytes(b'driver');self.driver.chmod(0o755)
        self.sign=patch.object(subject,'_signature',return_value=SIGNATURE);self.signature=self.sign.start();self.addCleanup(self.sign.stop)
        self.system=patch.object(subject,'_native_libraries',return_value=SYSTEM);self.libraries=self.system.start();self.addCleanup(self.system.stop)

    def stage(self):
        return subject.stage_reference_environment(self.app,self.driver,self.store,'reference/test')

    def target(self,identity,relative):return self.store/identity['app']['path']/relative

    def test_full_copy_preserves_internal_links_bytes_modes_and_signatures(self):
        identity=self.stage()
        self.assertEqual(len(identity['app']['files']),4)
        self.assertEqual(len(identity['app']['symlinks']),3)
        self.assertEqual(subject._scan(self.app,full=True)[0],subject._scan(self.store/identity['app']['path'],full=True)[0])
        self.assertEqual(json.loads((self.store/'reference/test/identity.json').read_text()),identity)
        self.assertEqual(subject.verify_reference_environment(identity,self.store)['inputs_sha256'],
                         subject.manifest_digest(subject.physical_inputs(identity)))
        self.assertGreaterEqual(self.signature.call_count,8)
        with self.assertRaises(InputError):snapshot_input(self.store,identity['app']['path'],kind='directory')

    def test_escape_absolute_dangling_and_cyclic_links_rejected_before_copy(self):
        link=self.app/'Contents/extra'
        for target in ('../../outside','/tmp','missing','extra'):
            with self.subTest(target=target):
                link.symlink_to(target)
                with self.assertRaises(ValueError):self.stage()
                self.assertFalse((self.store/'reference/test').exists())
                link.unlink()

    def test_special_file_rejected_before_copy(self):
        os.mkfifo(self.app/'Contents/fifo')
        with self.assertRaisesRegex(ValueError,'special file'):self.stage()
        self.assertFalse((self.store/'reference/test').exists())

    def test_admission_rejects_missing_extra_and_retargeted_members(self):
        identity=self.stage();root=self.store/identity['app']['path']
        added=root/'Contents/unlisted';added.write_bytes(b'extra')
        with self.assertRaisesRegex(ValueError,'mode|population'):subject.verify_reference_environment(identity,self.store)
        added.unlink()
        link=root/'Contents/Frameworks/Test.framework/Versions/Current';link.unlink();link.symlink_to('A/.')
        with self.assertRaisesRegex(ValueError,'link membership'):subject.verify_reference_environment(identity,self.store)

    def test_checkpoint_rejects_same_size_restored_mtime_bytes(self):
        identity=self.stage();file=self.target(identity,'Contents/Frameworks/Test.framework/Versions/A/Resources/asset')
        lease=subject.ReferenceLease(identity,self.store);lease.__enter__();old=file.stat()
        file.write_bytes(b'modified');os.utime(file,ns=(old.st_atime_ns,old.st_mtime_ns))
        with self.assertRaises(InputError):lease.checkpoint()
        with self.assertRaises((ValueError,InputError)):lease.finalize()
        self.assertIsNone(lease.finalization)

    def test_checkpoint_rejects_directory_and_link_restore(self):
        for relative in ('Contents/Frameworks/Test.framework/Versions/A/Resources',
                         'Contents/Frameworks/Test.framework/Versions/Current'):
            with self.subTest(relative=relative):
                identity=self.stage();lease=subject.ReferenceLease(identity,self.store);lease.__enter__()
                path=self.target(identity,relative);other=path.with_name(path.name+'-temporary')
                path.rename(other);other.rename(path)
                with self.assertRaises((ValueError,InputError)):lease.checkpoint()
                # A fresh directory is used for the second independent attempt.
                import shutil;shutil.rmtree(self.store/'reference')

    def test_context_exit_rechecks_after_explicit_successful_finalize(self):
        identity=self.stage();lease=subject.ReferenceLease(identity,self.store)
        with self.assertRaises((ValueError,InputError)):
            with lease:
                lease.finalize()
                self.target(identity,'Contents/Frameworks/Test.framework/Versions/A/Resources/asset').write_bytes(b'modified')
        self.assertIsNone(lease.finalization)

    def test_final_signature_drift_rejects_even_when_bytes_match(self):
        identity=self.stage();lease=subject.ReferenceLease(identity,self.store);lease.__enter__()
        self.signature.return_value=dict(SIGNATURE,cdhash='b'*40)
        with self.assertRaisesRegex(ValueError,'signature.*changed'):lease.finalize()
        self.assertIsNotNone(lease.final_error)

    def test_final_system_drift_rejects_even_when_bytes_match(self):
        identity=self.stage();lease=subject.ReferenceLease(identity,self.store);lease.__enter__()
        self.libraries.return_value=dict(SYSTEM,macos_build='CHANGED')
        with self.assertRaisesRegex(ValueError,'system/load graph'):lease.finalize()

    def test_primary_error_preserved_and_final_checks_still_run(self):
        identity=self.stage();before=self.signature.call_count
        lease=subject.ReferenceLease(identity,self.store)
        with self.assertRaisesRegex(RuntimeError,'primary'):
            with lease:
                self.signature.return_value=dict(SIGNATURE,cdhash='b'*40)
                raise RuntimeError('primary')
        self.assertGreater(self.signature.call_count,before+2)
        self.assertIsNotNone(lease.final_error);self.assertIsNone(lease.finalization)

    def test_identity_cannot_omit_files_or_select_browser_outside_app(self):
        identity=self.stage()
        changed=copy.deepcopy(identity);changed['browser']=changed['driver']
        with self.assertRaisesRegex(ValueError,'outside declared app'):subject.physical_inputs(changed)
        changed=copy.deepcopy(identity);changed['app']['files'].pop()
        with self.assertRaisesRegex(ValueError,'file modes'):subject.physical_inputs(changed)

    def test_cannot_select_signed_executable_helper_as_main_browser(self):
        helper=self.app/'Contents/MacOS/Helper';helper.write_bytes(b'helper');helper.chmod(0o755)
        identity=self.stage()
        identity['browser']=next(row for row in identity['app']['files'] if row['path'].endswith('/Helper'))
        with self.assertRaisesRegex(ValueError,'CFBundleExecutable'):
            subject.verify_reference_environment(identity,self.store)

    def test_signature_mismatch_rejects_copied_app(self):
        def signature(path,*,app):
            return dict(SIGNATURE,cdhash=('b' if Path(path).is_relative_to(self.store) else 'a')*40)
        self.signature.side_effect=signature
        with self.assertRaisesRegex(ValueError,'signature.*changed'):self.stage()
        self.assertFalse((self.store/'reference/test/identity.json').exists())

    def archive(self):
        import zipfile
        from oryn_validation import reference_archive
        path=self.root/'publisher.zip'
        observed,_=subject._scan(self.app,full=True)
        with zipfile.ZipFile(path,'w') as archive:
            def add(name,mode,data=b''):
                row=zipfile.ZipInfo(reference_archive.APP_PREFIX+name);row.create_system=3;row.external_attr=mode<<16
                archive.writestr(row,data)
            add('',stat.S_IFDIR|observed['mode'])
            for row in observed['directories']:add(row['path']+'/',stat.S_IFDIR|row['mode'])
            for row in observed['files']:add(row['path'],stat.S_IFREG|row['mode'],(self.app/row['path']).read_bytes())
            for row in observed['symlinks']:add(row['path'],stat.S_IFLNK|0o777,row['target'].encode())
        return path

    def archive_policy(self,path):
        import hashlib,zipfile
        from contextlib import ExitStack
        from oryn_validation import reference_archive
        stack=ExitStack()
        stack.enter_context(patch.object(reference_archive,'PACKAGE_SHA256',hashlib.sha256(path.read_bytes()).hexdigest()))
        stack.enter_context(patch.object(reference_archive,'PACKAGE_SIZE',path.stat().st_size))
        with zipfile.ZipFile(path) as archive:count=len(archive.infolist())
        stack.enter_context(patch.object(reference_archive,'PACKAGE_MEMBERS',count))
        return stack

    def test_exact_publisher_archive_preserves_unsigned_observation(self):
        archive=self.archive()
        def signature(path,*,app,publisher_archive=False):
            if app:
                self.assertTrue(publisher_archive)
                return dict(SIGNATURE,verified=False,failure='code has no resources but signature indicates they must be present')
            return dict(SIGNATURE,verified=True)
        self.signature.side_effect=signature
        with self.archive_policy(archive):
            identity=subject.stage_reference_environment(self.app,self.driver,self.store,'reference/test',source_archive=archive)
            self.assertFalse(identity['signatures']['app']['verified'])
            self.assertTrue(identity['signatures']['driver']['verified'])
            self.assertEqual(identity['authentication']['method'],'publisher-archive')
            self.assertIn(identity['authentication']['archive'],subject.physical_inputs(identity))
            subject.verify_reference_environment(identity,self.store)
            self.target(identity,'Contents/MacOS/Reference').write_bytes(b'changed executable')
            with self.assertRaises((ValueError,InputError)):subject.verify_reference_environment(identity,self.store)

    def test_archive_digest_or_app_mismatch_rejected_before_staging(self):
        archive=self.archive()
        from oryn_validation import reference_archive
        with self.assertRaisesRegex(ValueError,'pinned publisher'):
            reference_archive.verify_archive(archive)
        with self.archive_policy(archive):
            (self.app/'Contents/MacOS/Reference').write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError,'differs from pinned publisher archive'):
                subject.stage_reference_environment(self.app,self.driver,self.store,'reference/test',source_archive=archive)
        self.assertFalse((self.store/'reference/test').exists())

    def test_archive_rejects_case_alias_and_escape_even_with_test_pin(self):
        import zipfile
        from oryn_validation import reference_archive
        for name in ('../outside','chrome-mac-arm64/google chrome for testing.app/'):
            archive=self.archive()
            with zipfile.ZipFile(archive,'a') as opened:
                row=zipfile.ZipInfo(name);row.create_system=3
                row.external_attr=(stat.S_IFDIR|0o755 if name.endswith('/') else stat.S_IFREG|0o644)<<16
                opened.writestr(row,b'')
            with self.archive_policy(archive):
                with self.assertRaises(ValueError):reference_archive.verify_archive(archive)


class SignatureAndLoadTests(unittest.TestCase):
    def test_codesign_requires_success_and_exact_fields(self):
        import subprocess
        from types import SimpleNamespace
        raw='Identifier=test\nTeamIdentifier=TEAM\nCDHash='+('a'*40)+'\nAuthority=Test\n'
        with patch.object(subject.subprocess,'run',return_value=SimpleNamespace(stderr=raw,returncode=0)) as run:
            result=subject._signature(Path('/fake.app'),app=True)
            self.assertEqual(result['authorities'],['Test'])
            self.assertIn('--deep',run.call_args_list[0].args[0])
            self.assertIn('--strict',run.call_args_list[0].args[0])
        with patch.object(subject.subprocess,'run',side_effect=subprocess.CalledProcessError(1,['codesign'])):
            with self.assertRaises(subprocess.CalledProcessError):subject._signature(Path('/fake'),app=False)
        with patch.object(subject.subprocess,'run',return_value=SimpleNamespace(stderr=raw+'CDHash='+('b'*40)+'\n',returncode=0)):
            with self.assertRaisesRegex(ValueError,'ambiguous'):subject._signature(Path('/fake'),app=False)

    def test_signature_failure_is_only_observable_for_explicit_publisher_archive_app(self):
        from types import SimpleNamespace
        missing='code has no resources but signature indicates they must be present'
        raw='Identifier=test\nTeamIdentifier=TEAM\nCDHash='+('a'*40)+'\nAuthority=Test\n'
        failed=SimpleNamespace(returncode=1,stderr='/fake.app: '+missing+'\n')
        details=SimpleNamespace(returncode=0,stderr=raw)
        with patch.object(subject.subprocess,'run',side_effect=[failed,details]):
            self.assertFalse(subject._signature(Path('/fake.app'),app=True,publisher_archive=True)['verified'])
        for options in ({'app':True},{'app':False,'publisher_archive':True}):
            with patch.object(subject.subprocess,'run',return_value=failed):
                with self.assertRaisesRegex(ValueError,'signature verification failed'):
                    subject._signature(Path('/fake.app'),**options)

    def test_native_load_graph_rejects_external_dependencies(self):
        from oryn_validation import environments
        with tempfile.TemporaryDirectory() as temporary:
            app=Path(temporary).resolve()/'Browser.app';(app/'Contents/MacOS').mkdir(parents=True)
            binary=app/'Contents/MacOS/Browser';binary.write_bytes(b'native')
            external=app.parent/'external.dylib';external.write_bytes(b'outside')
            with patch.object(environments,'_macho',return_value=True),patch.object(environments,'_otool',return_value='0xfeedfacf 16777228 0 0x00 2 10 1234 0x0\ncommands'), \
                 patch.object(environments,'_dylib_loads',return_value=[str(external)]):
                with self.assertRaisesRegex(ValueError,'outside declared app'):
                    subject._native_libraries(app,external,binary,[{'path':'Contents/MacOS/Browser'}])

    def test_inherited_browser_rpath_keeps_its_original_loader_directory(self):
        from oryn_validation import environments
        with tempfile.TemporaryDirectory() as temporary:
            app=Path(temporary).resolve()/'Browser.app'
            rows=[]
            for name in ('Contents/MacOS/Browser','Contents/Plugins/plugin.dylib','Contents/Frameworks/dependency.dylib'):
                path=app/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(b'native');rows.append({'path':name})
            binary=app/'Contents/MacOS/Browser';driver=app.parent/'driver';driver.write_bytes(b'driver')
            main='0xfeedfacf 16777228 0 0x00 2 10 1234 0x0\n cmd LC_RPATH\n cmdsize 48\n path @loader_path/../Frameworks (offset 12)\n'
            with patch.object(environments,'_macho',side_effect=lambda path:path!=driver), \
                 patch.object(environments,'_otool',side_effect=lambda path,flag:main if path==binary else '0xfeedfacf 16777228 0 0x00 6 10 1234 0x0\n'+str(path)), \
                 patch.object(environments,'_dylib_loads',side_effect=lambda raw:['@rpath/dependency.dylib'] if 'plugin.dylib' in raw else []), \
                 patch.object(subject.subprocess,'check_output',return_value='TEST\n'):
                graph=subject._native_libraries(app,driver,binary,rows)
            self.assertEqual(graph['internal_loads'],[{'from':'Contents/Plugins/plugin.dylib',
                'to':'Contents/Frameworks/dependency.dylib','load_name':'@rpath/dependency.dylib'}])


if __name__=='__main__':unittest.main()
