"""Real loopback HTTP races, bounded teardown and unchanged upstream timeouts."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from webdriver.client import Session
from webdriver.error import UnknownErrorException
from wptrunner.executors.executorwebdriver import WebDriverRun

from oryn_wptrunner import custody
from oryn_wptrunner.protocol import AuthenticatedTransport, CheckedSession, OrynProtocol
from oryn_wptrunner.transport import _InterruptibleConnection


TOKEN = 'a' * 64


@contextmanager
def driver(*, body=False, delete_error=False, trickle=False):
    entered = threading.Event()
    release = threading.Event()
    requests = []

    class Handler(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *args):
            pass

        def do_GET(self):
            self.handle_command()

        def do_DELETE(self):
            self.handle_command()

        def handle_command(self):
            requests.append((self.command, self.path, self.client_address[1],
                             self.headers.get('Authorization')))
            if self.headers.get('Authorization') != 'Bearer ' + TOKEN:
                self.send_error(401)
                return
            status = 200
            payload = b'{"value": null}'
            if self.path == '/session/owned' and delete_error:
                status = 500
                payload = json.dumps({'value': {'error': 'unknown error',
                                               'message': 'native retirement failed',
                                               'stacktrace': ''}}).encode()
            held = self.path.endswith('/actions')
            if held and not body:
                entered.set()
                release.wait(3)
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(payload)))
            if held and body:
                self.send_header('Connection', 'close')
                self.close_connection = True
            self.end_headers()
            try:
                if held and body:
                    self.wfile.write(payload[:1])
                    self.wfile.flush()
                    entered.set()
                    release.wait(3)
                    self.wfile.write(payload[1:])
                elif self.path == '/session/owned' and trickle:
                    # Faster than the socket inactivity timeout, but longer
                    # than the total cleanup deadline selected by the test.
                    for byte in payload:
                        self.wfile.write(bytes([byte]))
                        self.wfile.flush()
                        if release.wait(0.02):
                            break
                else:
                    self.wfile.write(payload)
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = False
    serve = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    serve.start()
    transport = AuthenticatedTransport('127.0.0.1', server.server_port, TOKEN)
    transport.settlement_timeout = 0.5
    try:
        yield SimpleNamespace(transport=transport, entered=entered, release=release,
                              requests=requests, port=server.server_port)
    finally:
        release.set()
        try:
            transport.retire()
        finally:
            server.shutdown()
            serve.join(2)
            server.server_close()
        if serve.is_alive():
            raise AssertionError('owned fixture server did not settle')


def client(fixture):
    session = CheckedSession('127.0.0.1', fixture.port)
    session.transport = fixture.transport
    session.session_id = 'owned'
    return session


@contextmanager
def command_thread(operation):
    outcome = []

    def run():
        try:
            outcome.append((True, operation()))
        except BaseException as error:
            outcome.append((False, error))

    thread = threading.Thread(target=run)
    thread.start()
    try:
        yield thread, outcome
    finally:
        thread.join(2)
        if thread.is_alive():
            raise AssertionError('owned command thread did not settle')


class TransportSettlementTests(unittest.TestCase):
    def test_concurrent_command_uses_private_wire_and_ordinary_wire_is_reused(self):
        with driver() as fixture:
            transport = fixture.transport
            transport.send('GET', '/status')
            transport.send('GET', '/status')
            self.assertEqual(fixture.requests[0][2], fixture.requests[1][2])
            with command_thread(lambda: transport.send('DELETE', '/session/owned/actions')) as (_, result):
                self.assertTrue(fixture.entered.wait(2))
                transport.send('GET', '/status')
                self.assertNotEqual(fixture.requests[2][2], fixture.requests[3][2])
                fixture.release.set()
            self.assertTrue(result[0][0])
            self.assertTrue(all(row[3] == 'Bearer ' + TOKEN for row in fixture.requests))

    def test_timeout_during_actions_cleanup_keeps_timeout_and_checked_teardown_succeeds(self):
        with driver(body=True) as fixture:
            session = client(fixture)
            executor_threads = []

            def execute(protocol, url, timeout):
                executor_threads.append(threading.current_thread())
                # Exercise the actual upstream finally path that was in flight
                # when R12 returned EXTERNAL-TIMEOUT.
                try:
                    return 'completed callback'
                finally:
                    session.actions.release()

            def alive():
                try:
                    fixture.transport.send('GET', '/status')
                    return True
                except OSError:
                    return False

            protocol = SimpleNamespace(base=SimpleNamespace(set_timeout=lambda value: None),
                                       is_alive=alive)
            logger = SimpleNamespace(error=lambda value: None, info=lambda value: None)
            runner = WebDriverRun(logger, execute, protocol, 'http://example.test/', 0.25, 0)
            result = runner.run()
            self.assertEqual(result[0], False)
            self.assertEqual(result[1][0], 'EXTERNAL-TIMEOUT')
            self.assertTrue(fixture.entered.is_set())
            try:
                session.end()
            finally:
                fixture.release.set()
                for thread in executor_threads:
                    thread.join(2)
                    self.assertFalse(thread.is_alive())
            self.assertEqual(result[1][0], 'EXTERNAL-TIMEOUT')
            self.assertEqual(sum(row[:2] == ('DELETE', '/session/owned') for row in fixture.requests), 1)
            action = next(row for row in fixture.requests if row[1].endswith('/actions'))
            deleted = next(row for row in fixture.requests if row[1] == '/session/owned')
            self.assertNotEqual(action[2], deleted[2])
            with self.assertRaisesRegex(OSError, 'ending'):
                session.start()
            with self.assertRaisesRegex(OSError, 'retired'):
                session.send_command('POST', 'session', {})
            self.assertIsNone(session.session_id)

    def test_native_delete_error_remains_failed_protocol_custody_after_local_settlement(self):
        with driver(delete_error=True) as fixture, tempfile.TemporaryDirectory() as directory:
            session = client(fixture)
            path = Path(directory) / 'custody.jsonl'
            path.touch(mode=0o600)
            with patch.dict(os.environ, {'ORYN_WPT_PROTOCOL_CUSTODY': str(path),
                                         'ORYN_WPT_CANDIDATE_SHA256': 'b' * 64}):
                identity = custody.begin()
                protocol = SimpleNamespace(webdriver=session, _custody_session=identity)
                with self.assertRaisesRegex(UnknownErrorException, 'native retirement failed'):
                    OrynProtocol.teardown(protocol)
            records = [json.loads(row) for row in path.read_text().splitlines()]
            self.assertEqual([row['phase'] for row in records], ['connect', 'teardown'])
            self.assertFalse(records[-1]['success'])
            self.assertEqual(records[-1]['error_type'], 'UnknownErrorException')
            self.assertEqual(session._retiring_session_id, 'owned')
            before = list(fixture.requests)
            session.__del__()
            self.assertEqual(fixture.requests, before)

    def test_unsettled_exchange_is_failure_even_when_isolated_native_delete_succeeds(self):
        with driver() as fixture:
            session = client(fixture)
            entered = threading.Event()
            release = threading.Event()

            def blocked_decode(*args, **kwargs):
                entered.set()
                release.wait(2)

            fixture.transport.settlement_timeout = 0.02
            with patch.object(fixture.transport._ordinary, 'send', side_effect=blocked_decode):
                with command_thread(lambda: fixture.transport.send('GET', '/status')):
                    self.assertTrue(entered.wait(1))
                    try:
                        with self.assertRaisesRegex(TimeoutError, 'exchanges did not settle'):
                            session.end()
                    finally:
                        release.set()
            self.assertEqual(sum(row[:2] == ('DELETE', '/session/owned') for row in fixture.requests), 1)

    def test_trickle_response_cannot_extend_isolated_cleanup_wall_deadline(self):
        with driver(trickle=True) as fixture:
            session = client(fixture)
            fixture.transport.teardown_timeout = 0.08
            with self.assertRaisesRegex(TimeoutError, 'wall deadline'):
                session.end()
            self.assertIsNone(fixture.transport._cleanup_owner)
            self.assertIsNone(session.session_id)

    def test_end_waits_for_admitted_start_publication_before_selecting_delete_identity(self):
        with driver() as fixture:
            session = client(fixture)
            session.session_id = None
            publication = threading.Event()
            release = threading.Event()

            def start(upstream_self):
                # Deliberately pause after the wire exchange but before the
                # upstream Session.start state publication that _active misses.
                upstream_self.transport.send('GET', '/status')
                publication.set()
                self.assertTrue(release.wait(2))
                upstream_self.session_id = 'owned'
                upstream_self.capabilities = {'test': True}

            with patch.object(Session, 'start', start):
                with command_thread(session.start) as (_, started):
                    self.assertTrue(publication.wait(1))
                    with command_thread(session.end) as (_, ended):
                        # retire is observable before waiting for publication.
                        with fixture.transport._condition:
                            self.assertTrue(fixture.transport._condition.wait_for(
                                lambda: fixture.transport._retired, timeout=1))
                        release.set()
                self.assertTrue(started[0][0])
                self.assertTrue(ended[0][0])
            self.assertIsNone(session.session_id)
            self.assertEqual(sum(row[:2] == ('DELETE', '/session/owned') for row in fixture.requests), 1)

    def test_start_failure_can_reenter_end_without_deadlock_or_new_session(self):
        with driver() as fixture:
            session = client(fixture)
            session.session_id = None
            primary = OSError('new session failed')
            with patch.object(session, 'send_command', side_effect=primary):
                with self.assertRaises(OSError) as caught:
                    session.start()
            self.assertIs(caught.exception, primary)
            self.assertIsNone(session.session_id)
            self.assertEqual(fixture.requests, [])

    def test_socket_published_after_retirement_is_interrupted_before_checked_delete(self):
        with driver() as fixture:
            session = client(fixture)
            entering_connect = threading.Event()
            release_connect = threading.Event()
            connect = _InterruptibleConnection.connect

            def delayed_connect(connection):
                entering_connect.set()
                self.assertTrue(release_connect.wait(2))
                connect(connection)

            with patch.object(_InterruptibleConnection, 'connect', delayed_connect):
                with command_thread(session.actions.release) as (_, action):
                    self.assertTrue(entering_connect.wait(1))
                    try:
                        with command_thread(session.end) as (_, ended):
                            with fixture.transport._condition:
                                self.assertTrue(fixture.transport._condition.wait_for(
                                    lambda: fixture.transport._retired, timeout=1))
                            release_connect.set()
                    finally:
                        release_connect.set()
                        fixture.release.set()
            self.assertFalse(action[0][0])
            self.assertTrue(ended[0][0])
            self.assertEqual(sum(row[:2] == ('DELETE', '/session/owned') for row in fixture.requests), 1)

    def test_local_and_native_cleanup_failures_both_remain_observable(self):
        with driver(delete_error=True) as fixture:
            session = client(fixture)
            entered = threading.Event()
            release = threading.Event()

            def blocked_decode(*args, **kwargs):
                entered.set()
                release.wait(2)

            fixture.transport.settlement_timeout = 0.02
            with patch.object(fixture.transport._ordinary, 'send', side_effect=blocked_decode):
                with command_thread(lambda: fixture.transport.send('GET', '/status')):
                    self.assertTrue(entered.wait(1))
                    try:
                        with self.assertRaises(UnknownErrorException) as caught:
                            session.end()
                        self.assertIsInstance(caught.exception.__cause__, TimeoutError)
                    finally:
                        release.set()

    def test_unsettled_cleanup_thread_is_retained_and_cannot_be_replaced(self):
        transport = AuthenticatedTransport('127.0.0.1', 4444, TOKEN)
        transport.teardown_timeout = 0.02
        transport.settlement_timeout = 0.02
        release = threading.Event()
        try:
            with self.assertRaisesRegex(TimeoutError, 'wall deadline'):
                transport.run_isolated(lambda wire: release.wait(2))
            owner, wire = transport._cleanup_owner
            self.assertTrue(owner.is_alive())
            with self.assertRaisesRegex(RuntimeError, 'unsettled owner'):
                transport.run_isolated(lambda unused: None)
        finally:
            release.set()
            if transport._cleanup_owner is not None:
                owner, wire = transport._cleanup_owner
                owner.join(2)
                self.assertFalse(owner.is_alive())
                wire.retire()
            transport.retire()

    def test_start_publication_timeout_preserves_state_for_explicit_cleanup_retry(self):
        with driver() as fixture:
            session = client(fixture)
            session.session_id = None
            publication = threading.Event()
            release = threading.Event()

            def start(upstream_self):
                publication.set()
                self.assertTrue(release.wait(2))
                upstream_self.session_id = 'owned'

            fixture.transport.settlement_timeout = 0.02
            with patch.object(Session, 'start', start):
                with command_thread(session.start):
                    self.assertTrue(publication.wait(1))
                    try:
                        with self.assertRaisesRegex(TimeoutError, 'start did not settle'):
                            session.end()
                    finally:
                        release.set()
            self.assertEqual(session.session_id, 'owned')
            self.assertEqual(fixture.requests, [])
            session.end()
            self.assertIsNone(session.session_id)
            self.assertEqual(sum(row[:2] == ('DELETE', '/session/owned') for row in fixture.requests), 1)

    def test_second_end_cannot_bypass_retained_cleanup_owner_after_session_id_clear(self):
        with driver() as fixture:
            session = client(fixture)
            release = threading.Event()
            command = Session.send_command
            fixture.transport.teardown_timeout = 0.02
            fixture.transport.settlement_timeout = 0.02

            def blocked_cleanup(cleanup_session, *args, **kwargs):
                release.wait(2)
                return command(cleanup_session, *args, **kwargs)

            with patch.object(Session, 'send_command', blocked_cleanup):
                try:
                    with self.assertRaisesRegex(TimeoutError, 'wall deadline'):
                        session.end()
                    self.assertIsNone(session.session_id)
                    self.assertEqual(session._retiring_session_id, 'owned')
                    owner, isolated = fixture.transport._cleanup_owner
                    self.assertTrue(owner.is_alive())
                    with self.assertRaisesRegex(RuntimeError, 'unsettled owner'):
                        session.end()
                    with self.assertRaisesRegex(RuntimeError, 'unsettled native retirement owner'):
                        fixture.transport.close()
                    self.assertEqual(fixture.requests, [])
                finally:
                    release.set()
                    if fixture.transport._cleanup_owner is not None:
                        owner, isolated = fixture.transport._cleanup_owner
                        owner.join(2)
                        self.assertFalse(owner.is_alive())
            session.end()
            self.assertIsNone(fixture.transport._cleanup_owner)
            self.assertIsNone(session._retiring_session_id)
            self.assertEqual(sum(row[:2] == ('DELETE', '/session/owned') for row in fixture.requests), 1)

    def test_joined_cleanup_owner_is_retained_when_its_wire_close_failed(self):
        transport = AuthenticatedTransport('127.0.0.1', 4444, TOKEN)
        isolated = transport.isolated()
        failure = RuntimeError('isolated close failed')
        with patch.object(transport, 'isolated', return_value=isolated), \
                patch.object(isolated._ordinary, 'close', side_effect=failure):
            with self.assertRaises(RuntimeError) as caught:
                transport.run_isolated(lambda wire: None)
            self.assertIs(caught.exception, failure)
            owner, retained = transport._cleanup_owner
            self.assertFalse(owner.is_alive())
            self.assertIs(retained, isolated)
            with self.assertRaisesRegex(RuntimeError, 'isolated close failed'):
                transport.retire()
            with self.assertRaisesRegex(RuntimeError, 'unsettled native retirement owner'):
                transport.close()
        transport.retire()
        self.assertIsNone(transport._cleanup_owner)
        transport.close()

    def test_retire_cannot_clear_cleanup_owner_before_thread_start_publishes(self):
        transport = AuthenticatedTransport('127.0.0.1', 4444, TOKEN)
        transport.settlement_timeout = 0.5
        entering_start = threading.Event()
        release_start = threading.Event()
        retiring = threading.Event()
        start = threading.Thread.start

        def delayed_start(worker):
            if worker.name == 'oryn-native-retirement':
                entering_start.set()
                self.assertTrue(release_start.wait(2))
            return start(worker)

        def retire():
            try:
                return transport.retire()
            finally:
                retiring.set()

        with patch.object(threading.Thread, 'start', delayed_start):
            with command_thread(lambda: transport.run_isolated(lambda wire: None)) as (_, operation):
                self.assertTrue(entering_start.wait(1))
                try:
                    owner = transport._cleanup_owner
                    self.assertIsNone(owner[0].ident)
                    with command_thread(retire) as (_, retired):
                        with transport._condition:
                            self.assertTrue(transport._condition.wait_for(
                                lambda: transport._retired, timeout=1))
                        self.assertFalse(retiring.wait(0.03))
                        self.assertIs(transport._cleanup_owner, owner)
                        release_start.set()
                finally:
                    release_start.set()
            self.assertTrue(operation[0][0])
            self.assertTrue(retired[0][0])
        self.assertIsNone(transport._cleanup_owner)
        transport.close()


class SessionTimingTests(unittest.TestCase):
    def identity(self):
        from oryn_wptrunner.timing import IDENTITY_FIELDS
        return {key: 'validated-' + key for key in IDENTITY_FIELDS}

    def test_connect_span_forwards_return_and_binds_new_custody_identity(self):
        records = []
        identity = self.identity()
        protocol = SimpleNamespace(logger=SimpleNamespace(info=records.append),
                                   _timing_identity=identity)
        returned = object()

        def connect(parent):
            parent._custody_session = 'b' * 32
            return returned

        with patch.object(OrynProtocol, '_connect', connect):
            self.assertIs(OrynProtocol.connect(protocol), returned)
        rows = [json.loads(value.removeprefix('oryn-timing ')) for value in records]
        self.assertEqual([row['event'] for row in rows], ['begin', 'end'])
        self.assertTrue(all(row['phase'] == 'session.connect' and row['identity'] == identity for row in rows))
        self.assertIsNone(rows[0]['custody_id'])
        self.assertEqual(rows[-1]['custody_id'], 'b' * 32)
        self.assertEqual(rows[-1]['outcome'], 'returned')
        self.assertGreaterEqual(rows[-1]['elapsed_ns'], 0)

    def test_teardown_span_preserves_exact_exception_and_prior_custody_identity(self):
        records = []
        identity = self.identity()
        protocol = SimpleNamespace(logger=SimpleNamespace(info=records.append),
                                   _timing_identity=identity, _custody_session='c' * 32)
        failure = OSError('private native failure detail')

        def teardown(parent):
            parent._custody_session = None
            raise failure

        with patch.object(OrynProtocol, '_teardown', teardown):
            with self.assertRaises(OSError) as caught:
                OrynProtocol.teardown(protocol)
        self.assertIs(caught.exception, failure)
        rows = [json.loads(value.removeprefix('oryn-timing ')) for value in records]
        self.assertEqual([row['event'] for row in rows], ['begin', 'end'])
        self.assertTrue(all(row['phase'] == 'session.teardown' and row['identity'] == identity and
                            row['custody_id'] == 'c' * 32 for row in rows))
        self.assertEqual(rows[-1]['exception_type'], 'OSError')
        self.assertNotIn('private native failure detail', ''.join(records))
