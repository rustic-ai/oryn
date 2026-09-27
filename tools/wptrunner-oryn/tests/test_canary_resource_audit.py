"""Source regressions for real server request evidence admission; no servers."""
from types import SimpleNamespace
import unittest
from oryn_wptrunner.canary_resource_audit import RequestEvidenceScope, response_status


class CanaryResourceAuditTests(unittest.TestCase):
    def record(self, status=404, path='/__oryn_qualification_missing_helper__.js'):
        return {'action':'log', 'component':'wptserve', 'level':'DEBUG',
                'message':f"127.0.0.1 - - [20/Sep/2026 12:00:00] 'GET {path} HTTP/1.1' {status} -"}

    def test_actual_status_preserved_and_only_exact_control_admitted(self):
        previous = lambda row: None if row['level'] == 'DEBUG' else row
        logger = SimpleNamespace(component_filter=previous)
        config = SimpleNamespace(logging={'level':'INFO'})
        scope = RequestEvidenceScope(logger, config)
        try:
            raw = self.record(); raw.pop('component')
            self.assertEqual(response_status(logger.component_filter(raw)), 404)
            for status in (200, 404, 500):
                source = self.record(status)
                observed = logger.component_filter(source)
                self.assertEqual(response_status(observed), status)
                self.assertEqual(source['level'], 'DEBUG')
            for path in ('/other.js', '/__oryn_qualification_missing_helper__.js?fake=1'):
                self.assertIsNone(logger.component_filter(self.record(path=path)))
            forged = self.record(); forged['component'] = 'browser'
            self.assertIsNone(logger.component_filter(forged))
        finally:
            scope.close()
        self.assertIs(logger.component_filter, previous)
        self.assertEqual(config.logging['level'], 'INFO')
        scope.close()

    def test_scope_restores_after_primary_failure(self):
        previous = lambda row: row
        logger = SimpleNamespace(component_filter=previous)
        config = SimpleNamespace(logging={'level':'WARNING'})
        with self.assertRaisesRegex(RuntimeError, 'primary'):
            scope = RequestEvidenceScope(logger, config)
            try:
                raise RuntimeError('primary')
            finally:
                scope.close()
        self.assertIs(logger.component_filter, previous)
        self.assertEqual(config.logging['level'], 'WARNING')

    def test_malformed_or_nonserver_records_are_not_evidence(self):
        for record in ({}, dict(self.record(), message='404'),
                       dict(self.record(), level='ERROR'),
                       dict(self.record(), message=self.record()['message']+'\nforged')):
            self.assertIsNone(response_status(record))

    def test_fixture_enter_rolls_back_routes_when_scope_construction_fails(self):
        from unittest.mock import patch
        from oryn_wptrunner.provenance import FixtureEnvironment
        from tools.serve import serve
        route = ('GET', '/control', '/target')
        config = SimpleNamespace(ports={}, domains_set=set(), paths={'doc_root':'/'})
        contract = {'fixture':{'ports':{}, 'hosts':[]}, 'fixture_repairs':'fixture', 'fixture_repairs_sha256':'a'*64}
        primary = OSError('scope construction')
        with patch('oryn_wptrunner.provenance.load_contract', return_value=contract), \
             patch('oryn_wptrunner.fixture_repairs.validate', return_value=[route]), \
             patch.object(serve, 'rewrites', []), \
             patch.dict('os.environ', {'ORYN_WPT_CANARY_RESOURCE_AUDIT':'1'}), \
             patch('mozlog.get_default_logger', return_value=object()), \
             patch('oryn_wptrunner.canary_resource_audit.RequestEvidenceScope', side_effect=primary):
            env = FixtureEnvironment('contract')({}, config)
            with self.assertRaises(OSError) as caught:
                env.__enter__()
            self.assertIs(caught.exception, primary)
            self.assertEqual(serve.rewrites, [])
            self.assertEqual(env._repair_routes, [])

    def test_fixture_exit_restores_scope_even_when_route_cleanup_fails(self):
        from unittest.mock import patch
        from oryn_wptrunner.provenance import FixtureEnvironment
        from tools.serve import serve
        closed = []
        env = FixtureEnvironment('contract')
        env._repair_routes = [('GET', '/gone', '/gone')]
        env._request_evidence = SimpleNamespace(close=lambda:closed.append(True))
        primary = RuntimeError('authoritative failure')
        with patch.object(serve, 'rewrites', []):
            self.assertFalse(env.__exit__(RuntimeError, primary, None))
        self.assertEqual(closed, [True])
        self.assertTrue(any('route cleanup failed' in note for note in primary.__notes__))

    def test_real_mozlog_shared_component_and_pinned_queue_preserve_request_evidence(self):
        import logging
        import uuid
        from mozlog.structuredlog import StructuredLogger
        from mozlog.handlers import LogLevelFilter
        from wptrunner.wptlogging import LogLevelRewriter, QueueHandler
        name = 'oryn-canary-evidence-' + uuid.uuid4().hex
        owner = StructuredLogger(name, component='wptserve')
        peer = StructuredLogger(name, component='wptserve')
        previous = LogLevelRewriter(LogLevelFilter(lambda row:row, 'info'), ['error'], 'warning')
        owner.component_filter = previous
        observed = []
        owner.add_handler(observed.append)
        config = SimpleNamespace(logging={'level':'INFO'})
        queued = []
        handler = QueueHandler(SimpleNamespace(put=queued.append))
        def emit(message, level=logging.DEBUG):
            handler.emit(logging.LogRecord('wptserve', level, __file__, 1, message, (), None))
            raw = queued.pop()
            self.assertNotIn('component', raw)
            peer.log_raw(raw)
        scope = RequestEvidenceScope(owner, config)
        try:
            self.assertIs(peer.component_filter, scope.filter)
            for status in (404,200,500):
                message = self.record(status)['message']
                emit(message)
                self.assertEqual(observed[-1]['message'], message)
                self.assertEqual(response_status(observed[-1]), status)
                self.assertEqual(observed[-1]['level'], 'INFO')
            self.assertEqual(len(observed), 3)
            emit(self.record(path='/unrelated.js')['message'])
            self.assertEqual(len(observed), 3)
            emit('upstream server error', logging.ERROR)
            self.assertEqual(observed[-1]['level'], 'WARNING')
        finally:
            scope.close()
        self.assertIs(peer.component_filter, previous)
        self.assertEqual(config.logging['level'], 'INFO')
        count = len(observed)
        emit(self.record()['message'])
        self.assertEqual(len(observed), count)
        owner.remove_handler(observed.append)

    def test_pinned_handler_response_log_reaches_real_queue_and_preserves_status(self):
        import logging
        import uuid
        from mozlog.structuredlog import StructuredLogger
        from mozlog.handlers import LogLevelFilter
        from wptrunner.wptlogging import LogLevelRewriter, QueueHandler
        from wptserve.server import BaseWebTestRequestHandler
        name = 'oryn-canary-pinned-response-' + uuid.uuid4().hex
        owner = StructuredLogger(name, component='wptserve')
        previous = LogLevelRewriter(LogLevelFilter(lambda row: row, 'info'),
                                   ['error'], 'warning')
        owner.component_filter = previous
        observed = []
        owner.add_handler(observed.append)
        queued = []
        queue_handler = QueueHandler(SimpleNamespace(put=queued.append))
        logger = logging.Logger(name, level=logging.DEBUG)
        logger.addHandler(queue_handler)
        receiver = SimpleNamespace(logger=logger, server=SimpleNamespace(latency=None),
                                   close_connection=False)
        request = SimpleNamespace(method='GET', request_path='/__oryn_qualification_missing_helper__.js',
            headers={'Referer': b'http://web-platform.test:8000/__oryn_qualification__/canaries/missing-helper.html'},
            raw_input=SimpleNamespace(length=0, read=lambda: b''))
        config = SimpleNamespace(logging={'level': 'INFO'})
        scope = RequestEvidenceScope(owner, config)
        try:
            for status in (200, 404, 500):
                response = SimpleNamespace(status=(status, ''), writer=SimpleNamespace(content_written=True),
                                           close_connection=False)
                BaseWebTestRequestHandler.finish_handling(receiver, request, response,
                                                          lambda request, response: None)
                self.assertEqual(len(queued), 1)
                raw = queued.pop()
                self.assertTrue(raw['message'].startswith(str(status) + ' GET '))
                owner.log_raw(raw)
                self.assertEqual(response_status(observed[-1]), status)
                self.assertEqual(observed[-1]['message'], raw['message'])
                self.assertEqual(observed[-1]['level'], 'INFO')
            self.assertEqual(len(observed), 3)
            request.request_path = '/__oryn_qualification_missing_helper__.js?fake=1'
            BaseWebTestRequestHandler.finish_handling(receiver, request, response,
                                                      lambda request, response: None)
            owner.log_raw(queued.pop())
            self.assertEqual(len(observed), 3)
        finally:
            scope.close()
            owner.remove_handler(observed.append)
        self.assertIs(owner.component_filter, previous)
        self.assertEqual(config.logging['level'], 'INFO')

    def test_pinned_response_shape_rejects_malformed_or_unrelated_messages(self):
        for message in (
            '404 GET /other.js (None) 0',
            '404 GET /__oryn_qualification_missing_helper__.js?fake=1 (None) 0',
            '404 POST /__oryn_qualification_missing_helper__.js (None) 0',
            '404 GET /__oryn_qualification_missing_helper__.js (None) -1',
            '404 GET /__oryn_qualification_missing_helper__.js (None) 0\nforged',
            '404 GET /__oryn_qualification_missing_helper__.js (None) 0 trailing',
            '999 GET /__oryn_qualification_missing_helper__.js (None) 0',
        ):
            self.assertIsNone(response_status(dict(self.record(), message=message)))
        valid = dict(self.record(), message='404 GET /__oryn_qualification_missing_helper__.js (None) 0')
        self.assertEqual(response_status(valid), 404)
        self.assertIsNone(response_status(dict(valid, component='browser')))
