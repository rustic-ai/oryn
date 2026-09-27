"""Own complete upstream HTTP exchanges across executor timeout boundaries."""
from http.client import HTTPConnection
import socket
import threading
import time

from webdriver.transport import HTTPWireProtocol


class TransportRetiredError(OSError):
    """Local closed ownership, also understood by upstream's liveness probe."""


class _InterruptibleConnection(HTTPConnection):
    def connect(self):
        super().connect()
        # getresponse() may clear HTTPConnection.sock for Connection: close
        # while the response still owns a makefile reader on this same socket.
        self.interrupt_socket = self.sock


class _AuthenticatedWire(HTTPWireProtocol):
    def __init__(self, host, port, token, socket_timeout=None):
        super().__init__(host, port)
        self._token = token
        self._socket_timeout = socket_timeout

    @property
    def connection(self):
        if self._conn is None:
            # Unlike upstream's per-request timeout, this also covers connect
            # and response-body decoding on the isolated teardown connection.
            options = {} if self._socket_timeout is None else {"timeout": self._socket_timeout}
            self._conn = _InterruptibleConnection(self.host, self.port, **options)
        return self._conn

    def _request(self, method, uri, payload, headers=None, timeout=None):
        headers = dict(headers or {})
        if any(name.lower() == "authorization" for name in headers):
            raise ValueError("Caller must not override driver authorization")
        headers["Authorization"] = "Bearer " + self._token
        return super()._request(method, uri, payload, headers, timeout)


class AuthenticatedTransport:
    """Keep one ordinary wire; concurrent exchanges never share HTTP state.

    Retirement denies subsequent sends and waits for all admitted sends,
    including upstream response decoding and private-wire cleanup. It does not
    imply that the native WebDriver session has been deleted.
    """
    settlement_timeout = 5.0
    teardown_timeout = 30.0

    def __init__(self, host, port, token):
        if len(token) != 64 or any(char not in "0123456789abcdef" for char in token):
            raise ValueError("Invalid native driver credential")
        self.host = host
        self.port = port
        self.url_prefix = "/"
        self._token = token
        self._condition = threading.Condition()
        self._retired = False
        self._active = set()
        self._cleanup_error = None
        self._cleanup_owner = None
        self._cleanup_starting = False
        self._ordinary = _AuthenticatedWire(host, port, token)

    def isolated(self):
        """Create a fresh authenticated wire for checked native retirement."""
        transport = AuthenticatedTransport(self.host, self.port, self._token)
        transport._ordinary._socket_timeout = self.teardown_timeout
        transport.settlement_timeout = self.settlement_timeout
        return transport

    def run_isolated(self, operation):
        """Run checked cleanup with a wall deadline and a retained thread owner."""
        isolated = self.isolated()
        outcome = []
        completed = threading.Event()

        def run():
            try:
                outcome.append((True, operation(isolated)))
            except BaseException as error:
                outcome.append((False, error))
            finally:
                completed.set()

        worker = threading.Thread(target=run, name="oryn-native-retirement")
        owner = (worker, isolated)
        with self._condition:
            if self._cleanup_owner is not None:
                raise RuntimeError("Native retirement already has an unsettled owner")
            self._cleanup_owner = owner
            self._cleanup_starting = True
        failure = None
        isolated_settled = False
        deadline = time.monotonic() + self.teardown_timeout
        try:
            try:
                worker.start()
            finally:
                with self._condition:
                    self._cleanup_starting = False
                    self._condition.notify_all()
            if not completed.wait(max(0, deadline - time.monotonic())):
                failure = TimeoutError("Native session retirement exceeded wall deadline")
            try:
                isolated.retire()
                isolated_settled = True
            except BaseException as error:
                if failure is None:
                    failure = error
                else:
                    failure.add_note("retirement transport settlement failed: " +
                                     type(error).__name__)
            worker.join(self.settlement_timeout)
            if worker.is_alive():
                if failure is None:
                    failure = TimeoutError("Native session retirement owner did not settle")
                else:
                    failure.add_note("native session retirement owner did not settle")
            if failure is not None:
                if outcome and not outcome[0][0]:
                    raise failure from outcome[0][1]
                raise failure
            success, value = outcome[0]
            if not success:
                raise value
            return value
        finally:
            # Never silently detach or forget an owner that did not join. Its
            # failure reaches protocol custody and the outer process supervisor.
            if not worker.is_alive() and isolated_settled:
                with self._condition:
                    if self._cleanup_owner is owner:
                        self._cleanup_owner = None

    def send(self, *args, **kwargs):
        with self._condition:
            if self._retired:
                raise TransportRetiredError("Native driver transport is retired")
            wire = (self._ordinary if self._ordinary not in self._active else
                    _AuthenticatedWire(self.host, self.port, self._token))
            self._active.add(wire)
        failure = None
        try:
            # HTTPWireProtocol.send owns Response.from_http as well as _request.
            return wire.send(*args, **kwargs)
        except BaseException as error:
            failure = error
            raise
        finally:
            with self._condition:
                try:
                    if wire is not self._ordinary or self._retired:
                        wire.close()
                except BaseException as error:
                    self._cleanup_error = type(error).__name__
                    if failure is None:
                        raise
                    failure.add_note("driver wire cleanup failed: " + type(error).__name__)
                finally:
                    self._active.remove(wire)
                    self._condition.notify_all()

    def retire(self):
        deadline = time.monotonic() + self.settlement_timeout
        with self._condition:
            self._retired = True
            self._condition.notify_all()
            while self._active:
                for wire in self._active:
                    connection = wire._conn
                    sock = getattr(connection, "interrupt_socket", None)
                    if sock is not None:
                        try:
                            # close() alone need not interrupt a response's
                            # socket.makefile reader in another thread.
                            sock.shutdown(socket.SHUT_RDWR)
                        except OSError:
                            # The send's owner may already have closed it. Only
                            # removal from _active establishes local settlement.
                            pass
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("Native driver exchanges did not settle")
                # A connect already admitted before retirement can publish its
                # socket later; observe it again without admitting new sends.
                self._condition.wait(min(remaining, 0.05))
            self._ordinary.close()
            if self._cleanup_error is not None:
                raise RuntimeError("Native driver wire cleanup failed: " +
                                   self._cleanup_error)
            while self._cleanup_starting:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("Native retirement owner launch did not settle")
                self._condition.wait(remaining)
            owner = self._cleanup_owner
        if owner is not None:
            worker, isolated = owner
            # A previous deadline failure retains this exact owner. A later
            # end/close cannot bypass it merely because session_id was cleared.
            isolated.retire()
            if worker.ident is not None:
                worker.join(self.settlement_timeout)
            if worker.is_alive():
                raise TimeoutError("Native session retirement owner did not settle")
            with self._condition:
                if self._cleanup_owner is owner:
                    self._cleanup_owner = None

    def close(self):
        with self._condition:
            if self._active:
                raise RuntimeError("Cannot close active native driver exchanges")
            if self._cleanup_owner is not None:
                raise RuntimeError("Cannot close with an unsettled native retirement owner")
            self._ordinary.close()
