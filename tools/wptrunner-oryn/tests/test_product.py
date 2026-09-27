"""Run in the pinned upstream dependency runtime after full generation completes."""
import unittest
from unittest.mock import patch

from oryn_wptrunner import WPT_REVISION, load_product
from oryn_wptrunner import product as integration
from wptrunner.executors.executorwebdriver import (
    WebDriverCrashtestExecutor, WebDriverPrintRefTestExecutor,
    WebDriverRefTestExecutor, WebDriverTestharnessExecutor,
)


class ProductContractTests(unittest.TestCase):
    def test_registration_uses_actual_upstream_executors(self):
        product = load_product()
        self.assertEqual(product.name, "oryn")
        expected = {
            "testharness": WebDriverTestharnessExecutor,
            "reftest": WebDriverRefTestExecutor,
            "crashtest": WebDriverCrashtestExecutor,
            "print-reftest": WebDriverPrintRefTestExecutor,
        }
        self.assertEqual(set(product.executor_classes), set(expected))
        for kind, upstream in expected.items():
            executor = product.executor_classes[kind]
            self.assertTrue(issubclass(executor, upstream))
            self.assertIs(executor.do_test, upstream.do_test)
        self.assertIs(product.get_browser_cls("testharness"), integration.OrynBrowser)
        self.assertEqual(WPT_REVISION, "719d5e38fdd0903a18ed9007aba816c98cc491e0")

    def test_capabilities_require_native_identity_and_certificate_validation(self):
        with patch.object(integration, "upstream_executor_kwargs", return_value={"timeout_multiplier": 3}):
            result = integration.executor_kwargs(None, "testharness", None, {})
            self.assertEqual(result["timeout_multiplier"], 3)
            self.assertEqual(result["capabilities"], {"browserName": "oryn", "acceptInsecureCerts": False})
            for capabilities in ({"browserName": "chrome"}, {"acceptInsecureCerts": True}):
                with self.assertRaises(ValueError):
                    integration.executor_kwargs(None, "testharness", None, {}, capabilities=capabilities)

    def test_capability_input_is_not_mutated(self):
        original = {"oryn:configuration": {"policy": "test"}}
        with patch.object(integration, "upstream_executor_kwargs", return_value={}):
            result = integration.executor_kwargs(None, "reftest", None, {}, capabilities=original)
        self.assertNotIn("browserName", original)
        self.assertIsNot(result["capabilities"], original)

    def test_driver_command_keeps_paths_as_arguments(self):
        browser = object.__new__(integration.OrynBrowser)
        browser.webdriver_binary = "/runtime path/driver"
        browser.binary = "/runtime path/oryn"
        browser.webdriver_args = ["--configuration", "/configuration path/test.json"]
        browser.host = "127.0.0.1"
        browser._port = 8123
        browser._token_path = "/private/token-file"
        browser._contract_path = "/private/run-contract.json"
        with patch.object(integration, "load_contract", return_value={"fixture_path": "/configuration path/test.json", "fixture_sha256": "a" * 64}):
            command = browser.make_command()
        import shutil
        self.addCleanup(shutil.rmtree, browser._native_timing.directory, True)
        self.addCleanup(lambda: browser._native_timing.finish(process_stopped=True))
        self.assertEqual(command, [
            "/runtime path/driver", "--host", "127.0.0.1", "--port", "8123",
            "--browser-binary", "/runtime path/oryn", "--auth-token-file", "/private/token-file",
            "--configuration", "/configuration path/test.json", "--configuration-sha256", "a" * 64,
            "--diagnostic-fd", str(browser._native_timing.fd),
        ])


class AuthenticatedTransportTests(unittest.TestCase):
    def test_header_injection_does_not_mutate_caller_or_url(self):
        from oryn_wptrunner.protocol import AuthenticatedTransport
        from webdriver.transport import HTTPWireProtocol
        token = "a" * 64
        transport = AuthenticatedTransport("127.0.0.1", 8123, token)
        headers = {"Content-Type": "application/json"}
        import io
        response = io.BytesIO(b'{"value": null}')
        response.status = 200
        response.getheaders = lambda: [("Content-Type", "application/json")]
        with patch.object(HTTPWireProtocol, "_request", return_value=response) as request:
            self.assertEqual(transport.send("POST", "/session", {}, headers).body, {"value": None})
            args = request.call_args.args
        self.assertEqual(args[1], "/session")
        self.assertEqual(args[3]["Authorization"], "Bearer " + token)
        self.assertNotIn("Authorization", headers)
        self.assertEqual(transport.url_prefix, "/")

    def test_rejects_credential_override_and_invalid_tokens(self):
        from oryn_wptrunner.protocol import AuthenticatedTransport
        transport = AuthenticatedTransport("127.0.0.1", 8123, "a" * 64)
        with self.assertRaises(ValueError):
            transport.send("GET", "/status", None, {"authorization": "other"})
        for token in ("", "A" * 64, "a" * 63, "a" * 65):
            with self.assertRaises(ValueError):
                AuthenticatedTransport("127.0.0.1", 8123, token)


class NativeSelectorContractTests(unittest.TestCase):
    def test_shadow_chain_retains_pinned_upstream_element_transport(self):
        from unittest.mock import Mock
        from oryn_wptrunner.protocol import OrynSelectorProtocolPart, OrynProtocol
        from wptrunner.executors.executorwebdriver import WebDriverSelectorProtocolPart
        part=object.__new__(OrynSelectorProtocolPart)
        part.webdriver=Mock()
        part.webdriver.send_session_command.return_value=["native-reference"]
        selectors=["#closed-shadow-host","button"]
        self.assertEqual(part.elements_by_selector_array(selectors),["native-reference"])
        part.webdriver.send_session_command.assert_called_once_with("POST","oryn/elements",{"selectors":selectors})
        self.assertIn(OrynSelectorProtocolPart,OrynProtocol.implements)
        self.assertNotIn(WebDriverSelectorProtocolPart,OrynProtocol.implements)

    def test_native_selector_failure_is_preserved(self):
        from unittest.mock import Mock
        from oryn_wptrunner.protocol import OrynSelectorProtocolPart
        from webdriver.error import StaleElementReferenceException
        part=object.__new__(OrynSelectorProtocolPart);part.webdriver=Mock()
        part.webdriver.send_session_command.side_effect=StaleElementReferenceException("retired frame")
        with self.assertRaises(StaleElementReferenceException):
            part.elements_by_selector_array(["#host","button"])


class NativeAccessibilityContractTests(unittest.TestCase):
    def test_both_pinned_accessibility_operations_keep_native_reference_transport(self):
        from unittest.mock import Mock
        from oryn_wptrunner.protocol import OrynAccessibilityProtocolPart, OrynProtocol
        from wptrunner.executors.executorwebdriver import WebDriverAccessibilityProtocolPart
        part=object.__new__(OrynAccessibilityProtocolPart);part.webdriver=Mock()
        element={"element-6066-11e4-a52e-4f735466cecf":"opaque"}
        part.get_accessibility_properties_for_element(element)
        part.webdriver.send_session_command.assert_called_with("POST","oryn/accessibility/element",{"element":element})
        part.get_accessibility_properties_for_accessibility_node("ax-opaque")
        part.webdriver.send_session_command.assert_called_with("POST","oryn/accessibility/node",{"id":"ax-opaque"})
        self.assertIn(OrynAccessibilityProtocolPart,OrynProtocol.implements)
        self.assertNotIn(WebDriverAccessibilityProtocolPart,OrynProtocol.implements)


class NativeAsyncOperationContractTests(unittest.TestCase):
    def test_queued_upstream_actions_complete_before_classic_message_wait(self):
        import asyncio
        from types import SimpleNamespace
        from oryn_wptrunner.protocol import OrynTestDriverProtocolPart
        from wptrunner.executors.executorwebdriver import WebDriverTestDriverProtocolPart
        loop = asyncio.new_event_loop()
        part = object.__new__(OrynTestDriverProtocolPart)
        part.parent = SimpleNamespace(loop=loop)
        events = []
        async def queued_action():
            events.append("native-result-delivered")
        task = loop.create_task(queued_action())
        def wait(*args):
            self.assertEqual(events, ["native-result-delivered"])
            self.assertTrue(task.done())
            return ["window", "complete", None]
        try:
            with patch.object(WebDriverTestDriverProtocolPart, "get_next_message", side_effect=wait):
                self.assertEqual(part.get_next_message("url", "resume", "window"), ["window", "complete", None])
        finally:
            loop.close()

    def test_only_geolocation_context_references_are_adapted_without_mutation(self):
        import asyncio
        from types import SimpleNamespace
        from webdriver.client import WebWindow, WebFrame
        from oryn_wptrunner.protocol import OrynTestDriverProtocolPart
        from wptrunner.executors.executorwebdriver import WebDriverTestDriverProtocolPart
        loop = asyncio.new_event_loop()
        part = object.__new__(OrynTestDriverProtocolPart)
        part.parent = SimpleNamespace(loop=loop)
        window, frame = WebWindow(None, "window-id"), WebFrame(None, "frame-id")
        unknown = {"untrusted": "reference"}
        params = {"contexts": [window, frame, unknown], "coordinates": None}
        payload = {"id": 42, "action": "bidi.emulation.set_geolocation_override", "params": params}
        message = ["window-id", "action", payload]
        try:
            with patch.object(WebDriverTestDriverProtocolPart, "get_next_message", return_value=message):
                result = part.get_next_message("url", "resume", "window-id")
            self.assertEqual(result[2]["params"]["contexts"], ["window-id", "frame-id", unknown])
            self.assertIs(params["contexts"][0], window)
            self.assertEqual(result[2]["id"], 42)
            payload["action"] = "bidi.permissions.set_permission"
            with patch.object(WebDriverTestDriverProtocolPart, "get_next_message", return_value=message):
                self.assertIs(part.get_next_message("url", "resume", "window-id"), message)
        finally:
            loop.close()

    def test_emulation_distinguishes_omitted_fields_from_explicit_null(self):
        import asyncio
        from unittest.mock import Mock
        from webdriver.bidi.undefined import UNDEFINED
        from oryn_wptrunner.protocol import OrynBidiEmulationProtocolPart
        part = object.__new__(OrynBidiEmulationProtocolPart)
        part.webdriver = Mock()
        asyncio.run(part.set_geolocation_override(None, UNDEFINED, ["window"]))
        part.webdriver.send_session_command.assert_called_with("POST", "oryn/geolocation", {"coordinates": None, "contexts": ["window"]})
        error = {"type": "positionUnavailable"}
        asyncio.run(part.set_geolocation_override(UNDEFINED, error, ["window"]))
        part.webdriver.send_session_command.assert_called_with("POST", "oryn/geolocation", {"error": error, "contexts": ["window"]})


class BoundArgumentsTimingTests(unittest.TestCase):
    def fixture(self):
        from types import SimpleNamespace
        contract = {'candidate_sha256': 'a' * 64, 'sha256': 'b' * 64,
                    'fixture_path': '/fixture/config.json',
                    'fixture': {'tls': {name: {'path': name + '.pem'}
                                      for name in ('ca', 'certificate', 'key')}}}
        args = {'binary': '/runtime', 'webdriver_binary': '/driver', 'oryn_run_contract': '/run',
                'ssl_type': 'pregenerated', 'ca_cert_path': '/fixture/ca.pem',
                'host_cert_path': '/fixture/certificate.pem', 'host_key_path': '/fixture/key.pem'}
        info = {'oryn_candidate_sha256': 'a' * 64, 'oryn_run_contract_sha256': 'b' * 64}
        return contract, args, info, SimpleNamespace(config={})

    def test_one_load_per_browser_kwargs_keeps_each_public_admission_fresh(self):
        contract, args, info, subsuite = self.fixture()
        with patch.object(integration, 'load_contract', return_value=contract) as loader:
            result = integration.browser_kwargs(None, 'testharness', info, None, subsuite, **args)
            self.assertEqual(loader.call_count, 1)
            self.assertEqual(result['binary'], args['binary'])
            integration.check_args(**args)
            self.assertEqual(loader.call_count, 2)
            loader.return_value = {**contract, 'sha256': 'c' * 64}
            with self.assertRaisesRegex(ValueError, 'changed after run_info'):
                integration.browser_kwargs(None, 'testharness', info, None, subsuite, **args)
            self.assertEqual(loader.call_count, 3)

    def test_shared_argument_checks_keep_tls_and_argument_rejections(self):
        contract, args, info, subsuite = self.fixture()
        for change in ({'ssl_type': 'none'}, {'ca_cert_path': '/other/ca.pem'},
                       {'webdriver_args': ['--unsafe']}, {'binary_args': ['--unsafe']}):
            with self.subTest(change=change), patch.object(integration, 'load_contract', return_value=contract):
                with self.assertRaises(ValueError):
                    integration.check_args(**{**args, **change})
                with self.assertRaises(ValueError):
                    integration.browser_kwargs(None, 'testharness', info, None, subsuite, **{**args, **change})
        subsuite.config['binary_args'] = ['--unsafe']
        with patch.object(integration, 'load_contract', return_value=contract), self.assertRaises(ValueError):
            integration.browser_kwargs(None, 'testharness', info, None, subsuite, **args)

    def test_browser_start_forwards_and_preserves_failure_without_previous_identity(self):
        from wptrunner.browsers.base import WebDriverBrowser
        browser = object.__new__(integration.OrynBrowser)
        browser._timing_identity = {'candidate_sha256': 'stale'}
        failure = RuntimeError('actual start failed')
        with patch.object(WebDriverBrowser, 'start', side_effect=failure) as start:
            with self.assertRaises(RuntimeError) as caught:
                browser.start({'group': 'value'}, preference=True)
            self.assertIs(caught.exception, failure)
            start.assert_called_once_with({'group': 'value'}, preference=True)
        self.assertIsNone(browser._timing_identity)
