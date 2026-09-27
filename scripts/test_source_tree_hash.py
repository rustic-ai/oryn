"""Compatibility with the pre-optimization source hash pipeline; temporary Git only."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
LEGACY = r'''
set -euo pipefail
git ls-files --cached --others --exclude-standard \
  | LC_ALL=C sort \
  | while IFS= read -r path; do
      case "$path" in
        benchmarks/evidence/*|docs/oryn2/IMPLEMENTATION_STATUS.md|docs/oryn2/CAPABILITY_MILESTONES.md) continue ;;
      esac
      [[ -f "$path" ]] || continue
      printf '%s\0' "$path"
      shasum -a 256 "$path" | awk '{print $1}'
    done \
  | shasum -a 256 \
  | awk '{print $1}'
'''


class SourceTreeHashTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / 'repo'
        self.repo.mkdir()
        self.env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_')}
        self.env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
                        LC_ALL='C')
        self.git('init', '-q')
        self.write('scripts/source-tree-hash.sh',
                   (ROOT / 'scripts/source-tree-hash.sh').read_bytes())

    def git(self, *args, input=None):
        return subprocess.run(['git', *args], cwd=self.repo, env=self.env,
                              input=input, capture_output=True, check=True).stdout

    def write(self, name, body):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
        return path

    def run_hash(self, legacy=False, env=None):
        command = ['bash', '-c', LEGACY] if legacy else ['bash', 'scripts/source-tree-hash.sh']
        return subprocess.run(command, cwd=self.repo, env=env or self.env,
                              capture_output=True, timeout=30)

    def assert_compatible(self):
        old, new = self.run_hash(True), self.run_hash()
        self.assertEqual(new.returncode, old.returncode, (old.stderr, new.stderr))
        self.assertEqual(new.stdout, old.stdout)
        self.assertEqual(new.returncode, 0, new.stderr)
        self.assertRegex(new.stdout, rb'^[0-9a-f]{64}\n$')
        return new.stdout

    def test_tracked_modified_deleted_untracked_ignored_and_exact_exclusions(self):
        self.write('tracked', b'original'); self.write('deleted', b'gone')
        self.git('add', 'tracked', 'deleted')
        self.write('tracked', b'working tree bytes'); (self.repo / 'deleted').unlink()
        self.write('untracked', b'\x00\xff\r\n')
        self.write('.gitignore', b'ignored\n'); self.write('ignored', b'ignore')
        for name in ['benchmarks/evidence/run/raw', 'docs/oryn2/IMPLEMENTATION_STATUS.md',
                     'docs/oryn2/CAPABILITY_MILESTONES.md']:
            self.write(name, b'excluded')
        before = self.assert_compatible()
        self.write('benchmarks/evidence/run/raw', b'changed but excluded')
        self.write('ignored', b'changed ignored'); self.assertEqual(before, self.assert_compatible())
        self.write('docs/oryn2/IMPLEMENTATION_STATUS.md.extra', b'included')
        self.assertNotEqual(before, self.assert_compatible())

    def test_byte_sort_and_git_display_quoting_configuration(self):
        for name in ['Z', 'a space', 'a\tb', 'line\nbreak', 'back\\slash', 'é', 'a']:
            self.write(name, name.encode())
        self.assert_compatible()
        self.git('config', 'core.quotePath', 'false')
        self.assert_compatible()

    def test_symlink_target_bytes_broken_links_and_nonregular_entries(self):
        target = Path(self.temp.name) / 'outside'; target.write_bytes(b'first')
        (self.repo / 'link').symlink_to(target)
        (self.repo / 'broken').symlink_to(self.repo / 'absent')
        (self.repo / 'dirlink').symlink_to(self.repo / 'scripts', target_is_directory=True)
        os.mkfifo(self.repo / 'pipe')
        before = self.assert_compatible(); target.write_bytes(b'second')
        self.assertNotEqual(before, self.assert_compatible())

    def test_unmerged_index_duplicate_rows_are_not_deduplicated(self):
        self.write('conflict', b'working')
        blob = self.git('hash-object', '-w', '--stdin', input=b'base').strip()
        self.git('update-index', '--index-info', input=b'100644 ' + blob + b' 1\tconflict\n'
                 + b'100644 ' + blob + b' 2\tconflict\n' + b'100644 ' + blob + b' 3\tconflict\n')
        rows = self.git('ls-files', '--cached', '--others', '--exclude-standard').splitlines()
        self.assertGreater(rows.count(b'conflict'), 1)
        self.assert_compatible()

    def test_large_binary_chunking_and_filename_identity(self):
        self.write('large', bytes(range(256)) * 9000)
        before = self.assert_compatible()
        (self.repo / 'large').rename(self.repo / 'renamed')
        self.assertNotEqual(before, self.assert_compatible())

    def test_normal_inventory_does_not_launch_per_file_hash_or_sort_tools(self):
        self.write('ordinary', b'bytes')
        expected = self.assert_compatible()
        tools = Path(self.temp.name) / 'tools'; tools.mkdir()
        for name in ['shasum', 'awk', 'sort']:
            path = tools / name; path.write_text('#!/bin/sh\nexit 99\n'); path.chmod(0o755)
        env = dict(self.env, PATH=str(tools) + os.pathsep + self.env['PATH'])
        actual = self.run_hash(env=env)
        self.assertEqual(actual.returncode, 0, actual.stderr)
        self.assertEqual(actual.stdout, expected)

    def test_dash_stdin_filename_preserves_legacy_inventory_consumption(self):
        self.write('-', b'not the bytes shasum reads')
        self.write('z', b'last')
        self.assert_compatible()

    def test_empty_inventory_hashes_empty_record_stream(self):
        (self.repo / ".git/info/exclude").write_text("scripts/\n")
        self.assertEqual(self.assert_compatible(),
                         b"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n")

    def test_git_failure_is_nonzero_and_never_claims_success(self):
        shutil.rmtree(self.repo / '.git')
        old, new = self.run_hash(True), self.run_hash()
        self.assertNotEqual(old.returncode, 0)
        self.assertNotEqual(new.returncode, 0)
        self.assertEqual(new.stdout, b'')


if __name__ == '__main__':
    unittest.main()
