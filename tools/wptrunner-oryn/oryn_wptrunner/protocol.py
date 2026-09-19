"""Authenticated transport; execution algorithms remain upstream-owned."""
import asyncio
from urllib.parse import urlsplit
from webdriver.client import Session, WebWindow, WebFrame
from webdriver.error import InvalidSessionIdException
from webdriver.transport import HTTPWireProtocol
from wptrunner.executors.executorwebdriver import (
    WebDriverProtocol, WebDriverTestharnessExecutor, WebDriverRefTestExecutor,
    WebDriverCrashtestExecutor, WebDriverPrintRefTestExecutor, WebDriverSelectorProtocolPart, WebDriverAccessibilityProtocolPart,
    WebDriverTestDriverProtocolPart,
)

from wptrunner.executors.protocol import BidiPermissionsProtocolPart, BidiEmulationProtocolPart


from .navigation_evidence import NavigationEvidenceMixin


class AuthenticatedTransport(HTTPWireProtocol):
    def __init__(self, host, port, token):
        super().__init__(host, port)
        if len(token) != 64 or any(char not in "0123456789abcdef" for char in token):
            raise ValueError("Invalid native driver credential")
        self._token = token

    def _request(self, method, uri, payload, headers=None, timeout=None):
        headers = dict(headers or {})
        if any(name.lower() == "authorization" for name in headers):
            raise ValueError("Caller must not override driver authorization")
        headers["Authorization"] = "Bearer " + self._token
        return super()._request(method, uri, payload, headers, timeout)


class CheckedSession(Session):
    def end(self):
        identity = self.session_id
        try:
            if identity is not None:
                try:
                    self.send_command('DELETE', 'session/' + identity)
                except InvalidSessionIdException:
                    # Last-window close may already have retired this session.
                    # Only an exact native completed-retirement receipt suffices.
                    receipt = self.send_command('GET', 'session/' + identity + '/oryn/diagnostics/retirement')
                    if receipt != {'schema_version': 1, 'session_id': identity, 'status': 'complete'}:
                        raise ValueError('native session retirement was not completed')
        finally:
            self.session_id = None
            self.capabilities = None
            self.bidi_session = None
            self.extension = None
            self.transport.close()


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
    implements = [OrynSelectorProtocolPart if part is WebDriverSelectorProtocolPart else
                  OrynAccessibilityProtocolPart if part is WebDriverAccessibilityProtocolPart else
                  OrynTestDriverProtocolPart if part is WebDriverTestDriverProtocolPart else part
                  for part in WebDriverProtocol.implements] + [OrynBidiPermissionsProtocolPart, OrynBidiEmulationProtocolPart]
    def __init__(self, executor, browser, capabilities, **kwargs):
        super().__init__(executor, browser, capabilities, **kwargs)
        self._token = browser.oryn_driver_token

    def connect(self):
        url = urlsplit(self.url)
        if url.scheme != "http" or url.hostname not in ("127.0.0.1", "::1") or not url.port:
            raise ValueError("Native driver requires an explicit loopback endpoint")
        if url.username or url.password or url.query or url.fragment or url.path not in ("", "/"):
            raise ValueError("Unexpected native driver endpoint fields")
        self.webdriver = CheckedSession(url.hostname, url.port,
                                 capabilities={"alwaysMatch": self.capabilities},
                                 enable_bidi=self.enable_bidi)
        self.webdriver.transport = AuthenticatedTransport(url.hostname, url.port, self._token)
        from .custody import begin, append
        self._custody_session = begin()
        try:
            self.webdriver.start()
            from .navigation_evidence import snapshot
            self._navigation_baseline = snapshot(self)
            self._navigation_boundary = "session-start"
            self.loop = asyncio.new_event_loop()
        except BaseException as error:
            append(self._custody_session, 'connect-failed', error)
            raise


    def teardown(self):
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
                failure = error
                raise
            finally:
                identity = getattr(self, '_custody_session', None)
                if identity is not None:
                    append(identity, 'teardown', failure)
                    self._custody_session = None


class OrynTestharnessExecutor(NavigationEvidenceMixin, WebDriverTestharnessExecutor):
    protocol_cls = OrynProtocol

    def __init__(self, *args, oryn_placement=None, **kwargs):
        self._placement = oryn_placement
        super().__init__(*args, **kwargs)

    def convert_result(self, test, result, extra=None):
        converted = super().convert_result(test, result, extra=extra)
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
