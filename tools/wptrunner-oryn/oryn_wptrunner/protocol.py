"""Authenticated transport; execution algorithms remain upstream-owned."""
import asyncio
import threading
from urllib.parse import urljoin, urlsplit
from webdriver.client import Session, WebWindow, WebFrame
from webdriver.error import InvalidSessionIdException, WebDriverException
from wptrunner.executors.executorwebdriver import (
    WebDriverProtocol, WebDriverTestharnessExecutor, WebDriverRefTestExecutor,
    WebDriverCrashtestExecutor, WebDriverPrintRefTestExecutor, WebDriverSelectorProtocolPart, WebDriverAccessibilityProtocolPart,
    WebDriverTestDriverProtocolPart, WebDriverTestharnessProtocolPart,
)

from wptrunner.executors.protocol import BidiPermissionsProtocolPart, BidiEmulationProtocolPart


from .navigation_evidence import NavigationEvidenceMixin
from .completion_diagnostics import capture as capture_completion_diagnostics
from .completion_diagnostics import capture_result as capture_completion_result


from .transport import AuthenticatedTransport, TransportRetiredError
from .timing import span


def phase(protocol, name, test_id=None):
    return span(getattr(protocol, 'logger', None), name,
                identity=getattr(protocol, '_timing_identity', None),
                service=getattr(protocol, '_timing_service', None), test_id=test_id,
                webdriver_session_id=getattr(protocol, '_timing_session_id', None),
                custody_id=getattr(protocol, '_custody_session', None))


class OrynTestharnessProtocolPart(WebDriverTestharnessProtocolPart):
    def load_runner(self, url_protocol):
        with phase(self.parent, 'harness.load_runner'):
            return super().load_runner(url_protocol)

    def close_old_windows(self):
        with phase(self.parent, 'harness.close_old_windows'):
            return super().close_old_windows()


class CheckedSession(Session):
    def __init__(self, *args, **kwargs):
        self._lifecycle = threading.RLock()
        self._ending = False
        super().__init__(*args, **kwargs)

    def __del__(self):
        # Explicit failed teardown retains its identity for an explicit retry,
        # not an unlogged new DELETE launched later by Session.__del__.
        if getattr(self, '_ending', True):
            return
        transport = getattr(self, 'transport', None)
        if isinstance(transport, AuthenticatedTransport):
            self.end()
        elif transport is not None:
            # Construction can fail before Oryn installs authenticated transport.
            transport.close()

    def start(self):
        # Cover upstream publication of session_id/capabilities after send().
        # RLock also permits Session.start's same-thread failure -> end path.
        with self._lifecycle:
            if self._ending:
                raise TransportRetiredError("Native driver session is ending")
            return super().start()

    def send_session_command(self, method, uri, body=None, timeout=None):
        # Capture the identity once: teardown can clear it while an upstream
        # liveness probe is entering this method from the timed-out thread.
        identity = self.session_id
        if self._ending:
            raise TransportRetiredError("Native driver session is ending")
        if not isinstance(identity, str):
            raise TypeError("Session.session_id must be a str to send a session command")
        measured_command = {
            ('POST', 'url'): 'webdriver.navigation',
            ('POST', 'window/new'): 'webdriver.new_window',
            ('POST', 'window'): 'webdriver.switch_window',
            ('DELETE', 'window'): 'webdriver.close_window',
            ('GET', 'window/handles'): 'webdriver.window_handles',
            ('DELETE', 'actions'): 'webdriver.release_actions',
            ('POST', 'execute/sync'): 'webdriver.execute_sync',
            ('POST', 'execute/async'): 'webdriver.execute_async',
        }.get((method, uri))
        if method == 'POST' and uri.startswith('element/') and uri.endswith('/click'):
            measured_command = 'webdriver.element_click'
        if measured_command:
            protocol = getattr(self, '_oryn_timing_protocol', None)
            with phase(protocol, measured_command):
                return self.send_command(method, urljoin('session/' + identity + '/', uri), body, timeout)
        return self.send_command(method, urljoin('session/' + identity + '/', uri), body, timeout)

    def end(self):
        self._ending = True
        failure = None
        settlement_failure = None
        locked = False
        try:
            try:
                self.transport.retire()
            except BaseException as error:
                settlement_failure = error
            locked = self._lifecycle.acquire(timeout=self.transport.settlement_timeout)
            if not locked:
                raise TimeoutError('Native session start did not settle')
            identity = self.session_id or getattr(self, '_retiring_session_id', None)
            if identity is not None:
                self._retiring_session_id = identity
                def cleanup(transport):
                    # Do not swap self.transport: a late executor command must
                    # hit the retired transport, never this cleanup connection.
                    session = Session(transport.host, transport.port)
                    session.transport = transport
                    try:
                        session.send_command('DELETE', 'session/' + identity)
                    except InvalidSessionIdException:
                        # A last-window close is accepted only with the exact
                        # native completed-retirement receipt, as before.
                        receipt = session.send_command(
                            'GET', 'session/' + identity + '/oryn/diagnostics/retirement')
                        if receipt != {'schema_version': 1, 'session_id': identity, 'status': 'complete'}:
                            raise ValueError('native session retirement was not completed')
                self.transport.run_isolated(cleanup)
                self._retiring_session_id = None
            if settlement_failure is not None:
                raise settlement_failure
        except BaseException as error:
            failure = error
            if settlement_failure is not None and error is not settlement_failure:
                raise failure from settlement_failure
            raise
        finally:
            if locked:
                self.session_id = None
                self.capabilities = None
                self.bidi_session = None
                self.extension = None
                self._lifecycle.release()
            # If start has not settled, preserve its state for failed-owner
            # diagnosis/retry. _ending continues to deny every later start.
            try:
                self.transport.close()
            except BaseException as error:
                if failure is None:
                    raise
                failure.add_note('transport cleanup failed: ' + type(error).__name__)


class OrynSelectorProtocolPart(WebDriverSelectorProtocolPart):
    def elements_by_selector_array(self, selectors):
        # Pinned upstream sends a selector for each shadow host. The native
        # command traverses native roots without author prototype lookups.
        return self.webdriver.send_session_command("POST", "oryn/elements", {"selectors": selectors})


class OrynAccessibilityProtocolPart(WebDriverAccessibilityProtocolPart):
    def get_accessibility_properties_for_element(self, element):
        return self.webdriver.send_session_command("POST", "oryn/accessibility/element", {"element": element})

    def get_accessibility_properties_for_accessibility_node(self, id):
        return self.webdriver.send_session_command("POST", "oryn/accessibility/node", {"id": id})


class OrynBidiPermissionsProtocolPart(BidiPermissionsProtocolPart):
    def setup(self):
        self.webdriver = self.parent.webdriver

    async def set_permission(self, descriptor, state, origin, embedded_origin=None):
        body = {"descriptor": descriptor, "state": state, "origin": origin}
        if embedded_origin is not None:
            body["embeddedOrigin"] = embedded_origin
        return self.webdriver.send_session_command("POST", "oryn/permissions", body)


class OrynBidiEmulationProtocolPart(BidiEmulationProtocolPart):
    def setup(self):
        self.webdriver = self.parent.webdriver

    async def set_geolocation_override(self, coordinates, error, contexts):
        from webdriver.bidi.undefined import UNDEFINED
        body = {"contexts": contexts}
        if error is not UNDEFINED:
            body["error"] = error
        if coordinates is not UNDEFINED:
            body["coordinates"] = coordinates
        return self.webdriver.send_session_command("POST", "oryn/geolocation", body)

    async def set_locale_override(self, locale, contexts):
        raise NotImplementedError("Locale emulation is not implemented by this native product")

    async def set_screen_orientation_override(self, screen_orientation, contexts):
        raise NotImplementedError("Screen orientation emulation is not implemented by this native product")

    async def set_touch_override(self, max_touch_points, contexts):
        raise NotImplementedError("Touch emulation is not implemented by this native product")


class OrynTestDriverProtocolPart(WebDriverTestDriverProtocolPart):
    def run(self, url, script_resume, test_window=None):
        # Includes navigation, asynchronous test work, actions and result wait.
        # This is wall time, not a claim of pure JavaScript execution time.
        with phase(self.parent, 'harness.run', test_id=url):
            try:
                result = super().run(url, script_resume, test_window)
                capture_completion_result(self.parent, url, result)
                return result
            finally:
                capture_completion_diagnostics(self.parent, url)

    def get_next_message(self, url, script_resume, test_window):
        # The required async permission/location actions use native HTTP routes.
        # Drain their queued upstream handlers before blocking for the next
        # classic message; no WebSocket capability or event stream is claimed.
        self.parent.loop.run_until_complete(asyncio.sleep(0))
        message = super().get_next_message(url, script_resume, test_window)
        if (isinstance(message, (tuple, list)) and len(message) == 3
                and message[1] == "action" and isinstance(message[2], dict)
                and message[2].get("action") == "bidi.emulation.set_geolocation_override"):
            payload = dict(message[2])
            params = dict(payload.get("params") or {})
            contexts = params.get("contexts")
            if isinstance(contexts, list):
                # Classic native Window references are already authenticated
                # session handles. Preserve unknown values for upstream errors.
                params["contexts"] = [value.id if isinstance(value, (WebWindow, WebFrame)) else value
                                      for value in contexts]
            payload["params"] = params
            message = [message[0], message[1], payload]
        return message


class OrynProtocol(WebDriverProtocol):
    implements = [OrynTestharnessProtocolPart if part is WebDriverTestharnessProtocolPart else
                  OrynSelectorProtocolPart if part is WebDriverSelectorProtocolPart else
                  OrynAccessibilityProtocolPart if part is WebDriverAccessibilityProtocolPart else
                  OrynTestDriverProtocolPart if part is WebDriverTestDriverProtocolPart else part
                  for part in WebDriverProtocol.implements] + [OrynBidiPermissionsProtocolPart, OrynBidiEmulationProtocolPart]
    def __init__(self, executor, browser, capabilities, **kwargs):
        super().__init__(executor, browser, capabilities, **kwargs)
        self._token = browser.oryn_driver_token
        self._timing_identity = getattr(browser, 'oryn_timing_identity', None)
        self._timing_service = getattr(browser, 'oryn_timing_service', None)

    def is_alive(self):
        if not self.webdriver:
            return False
        try:
            receipt = self.webdriver.send_session_command(
                'GET', 'oryn/diagnostics/health', timeout=2)
            return (isinstance(receipt, dict)
                    and receipt.get('schema_version') == 1
                    and receipt.get('session_id') == self.webdriver.session_id
                    and isinstance(receipt.get('live_pages'), int)
                    and receipt['live_pages'] > 0)
        except (OSError, WebDriverException, TransportRetiredError):
            return False

    def connect(self):
        with span(getattr(self, 'logger', None), 'session.connect',
                  service=getattr(self, '_timing_service', None),
                  webdriver_session_id=getattr(self, '_timing_session_id', None),
                  identity=getattr(self, '_timing_identity', None)) as timing:
            try:
                return OrynProtocol._connect(self)
            finally:
                # begin() is deliberately kept at its original admission point.
                # The ending span can bind custody once connect created it.
                timing.custody_id = getattr(self, '_custody_session', None)
                timing.webdriver_session_id = getattr(self, '_timing_session_id', None)

    def _connect(self):
        url = urlsplit(self.url)
        if url.scheme != "http" or url.hostname not in ("127.0.0.1", "::1") or not url.port:
            raise ValueError("Native driver requires an explicit loopback endpoint")
        if url.username or url.password or url.query or url.fragment or url.path not in ("", "/"):
            raise ValueError("Unexpected native driver endpoint fields")
        self.webdriver = CheckedSession(url.hostname, url.port,
                                 capabilities={"alwaysMatch": self.capabilities},
                                 enable_bidi=self.enable_bidi)
        self.webdriver._oryn_timing_protocol = self
        self.webdriver.transport = AuthenticatedTransport(url.hostname, url.port, self._token)
        from .custody import begin, append
        self._custody_session = begin()
        try:
            with phase(self, 'session.create') as measured:
                try:
                    self.webdriver.start()
                finally:
                    self._timing_session_id = getattr(self.webdriver, 'session_id', None)
                    measured.webdriver_session_id = self._timing_session_id
            from .navigation_evidence import snapshot
            self._navigation_baseline = snapshot(self)
            self._navigation_boundary = "session-start"
            self.loop = asyncio.new_event_loop()
        except BaseException as error:
            append(self._custody_session, 'connect-failed', error)
            raise


    def teardown(self):
        with span(getattr(self, 'logger', None), 'session.teardown',
                  service=getattr(self, '_timing_service', None),
                  webdriver_session_id=getattr(self, '_timing_session_id', None),
                  identity=getattr(self, '_timing_identity', None),
                  custody_id=getattr(self, '_custody_session', None)):
            return OrynProtocol._teardown(self)

    def _teardown(self):
        from .custody import append
        failure = None
        try:
            # Pinned upstream catches session.end failures. Oryn must retain
            # and propagate the actual native cleanup failure to its supervisor.
            if self.webdriver is not None:
                self.webdriver.end()
        except BaseException as error:
            failure = error
            raise
        finally:
            self.webdriver = None
            try:
                loop = getattr(self, "loop", None)
                if loop is not None and not loop.is_closed():
                    pending = asyncio.all_tasks(loop)
                    for task in pending:
                        task.cancel()
                    if pending:
                        loop.run_until_complete(asyncio.gather(*pending, return_exceptions=True))
                    loop.close()
            except BaseException as error:
                if failure is None:
                    failure = error
                    raise
                failure.add_note('protocol loop cleanup failed: ' + type(error).__name__)
            finally:
                identity = getattr(self, '_custody_session', None)
                if identity is not None:
                    try:
                        append(identity, 'teardown', failure)
                        if isinstance(failure, Exception):
                            from .recovery import teardown_failed
                            teardown_failed()
                    except BaseException as error:
                        if failure is None:
                            raise
                        failure.add_note('protocol custody append failed: ' + type(error).__name__)
                    finally:
                        self._custody_session = None


class OrynTestharnessExecutor(NavigationEvidenceMixin, WebDriverTestharnessExecutor):
    protocol_cls = OrynProtocol

    def create_test_window(self, protocol):
        with phase(protocol, 'harness.create_test_window'):
            return super().create_test_window(protocol)

    def __init__(self, *args, oryn_placement=None, oryn_support_descriptor=None,
                 oryn_support_candidate=None, **kwargs):
        self._placement = oryn_placement
        self._support_descriptor = oryn_support_descriptor
        self._support_candidate = oryn_support_candidate
        super().__init__(*args, **kwargs)

    def convert_result(self, test, result, extra=None):
        converted = super().convert_result(test, result, extra=extra)
        if self._support_descriptor is not None:
            from .support_executor_capture import capture
            converted = capture(self, test, converted, self._support_descriptor, self._support_candidate)
        if self._placement:
            from .placement import capture
            capture(self, test, self._placement)
        return converted


class OrynRefTestExecutor(NavigationEvidenceMixin, WebDriverRefTestExecutor):
    protocol_cls = OrynProtocol


class OrynCrashtestExecutor(NavigationEvidenceMixin, WebDriverCrashtestExecutor):
    protocol_cls = OrynProtocol


class OrynPrintRefTestExecutor(NavigationEvidenceMixin, WebDriverPrintRefTestExecutor):
    protocol_cls = OrynProtocol
