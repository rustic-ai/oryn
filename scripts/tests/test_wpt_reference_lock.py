import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import unittest


class ReferenceFocusLeaseTests(unittest.TestCase):
    def test_serialization_eof_and_crash_release(self):
        script = Path(__file__).resolve().parents[1] / 'wpt-reference-lock.py'
        children = []
        with tempfile.TemporaryDirectory() as directory:
            def start():
                child = subprocess.Popen([sys.executable, str(script)],
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                    env={**os.environ, 'TMPDIR': directory})
                children.append(child)
                return child
            def ready(child):
                self.assertTrue(select.select([child.stdout], [], [], 5)[0])
                self.assertEqual(child.stdout.readline(), b'ready\n')
            try:
                first = start()
                ready(first)
                second = start()
                self.assertFalse(select.select([second.stdout], [], [], .1)[0])
                first.stdin.close()
                self.assertEqual(first.wait(timeout=5), 0)
                ready(second)
                third = start()
                self.assertFalse(select.select([third.stdout], [], [], .1)[0])
                second.kill()
                second.wait(timeout=5)
                ready(third)
                third.stdin.close()
                self.assertEqual(third.wait(timeout=5), 0)
            finally:
                for child in children:
                    if child.poll() is None:
                        child.kill()
                    child.wait(timeout=5)
                    child.stdout.close()
                    if not child.stdin.closed:
                        child.stdin.close()
