"""Use upstream execution, reference readiness, fuzzy metadata and result types."""
import os
import secrets
import tempfile
import time

from wptrunner.browsers.base import WebDriverBrowser, OutputHandler, get_timeout_multiplier, require_arg
from wptrunner.executors import executor_kwargs as upstream_executor_kwargs
from .protocol import (
    OrynCrashtestExecutor, OrynPrintRefTestExecutor,
    OrynRefTestExecutor, OrynTestharnessExecutor,
)
from wptrunner.products import Product
from .provenance import load_run_contract as load_contract, run_info_extras, FixtureEnvironment
from .timing import span, contract_identity
from .native_timing import Collector


def check_args(**kwargs):
    require_arg(kwargs, "binary")
    require_arg(kwargs, "webdriver_binary")
    contract = load_contract(kwargs.get("oryn_run_contract"), kwargs["binary"], kwargs["webdriver_binary"])
    _check_bound_args(contract, kwargs)


def _check_bound_args(contract, kwargs):
    from pathlib import Path
    if kwargs.get("ssl_type") != "pregenerated":
        raise ValueError("Official fixtures require candidate-bound pregenerated certificates")
    for argument, name in (("ca_cert_path", "ca"), ("host_cert_path", "certificate"), ("host_key_path", "key")):
        expected = (Path(contract["fixture_path"]).parent / contract["fixture"]["tls"][name]["path"]).resolve()
        if not kwargs.get(argument) or Path(kwargs[argument]).resolve() != expected:
            raise ValueError("Upstream fixture TLS differs from the native candidate: " + argument)
    if kwargs.get("webdriver_args"):
        raise ValueError("Driver arguments must come from the bound fixture contract")
    if kwargs.get("binary_args"):
        raise ValueError("Native runtime configuration belongs in the driver contract, not unchecked browser arguments")


def browser_kwargs(logger, test_type, run_info_data, config, subsuite, **kwargs):
    require_arg(kwargs, "binary")
    require_arg(kwargs, "webdriver_binary")
    contract = load_contract(kwargs.get("oryn_run_contract"), kwargs["binary"], kwargs["webdriver_binary"])
    _check_bound_args(contract, kwargs)
    if (run_info_data.get("oryn_candidate_sha256") != contract["candidate_sha256"]
            or run_info_data.get("oryn_run_contract_sha256") != contract["sha256"]):
        raise ValueError("Run contract changed after run_info was established")
    if subsuite.config.get("binary_args"):
        raise ValueError("Subsuite browser arguments require native driver configuration support")
    return {
        "binary": kwargs["binary"],
        "webdriver_binary": kwargs["webdriver_binary"],
        "webdriver_args": [],
        "oryn_run_contract": kwargs.get("oryn_run_contract"),
    }


def executor_kwargs(logger, test_type, test_environment, run_info_data, **kwargs):
    result = upstream_executor_kwargs(test_type, test_environment, run_info_data, **kwargs)
    capabilities = dict(kwargs.get("capabilities") or {})
    if capabilities.get("browserName", "oryn") != "oryn":
        raise ValueError("Oryn qualification requires the native Oryn browser")
    if capabilities.get("acceptInsecureCerts", False) is not False:
        raise ValueError("Configure trust for the WPT CA; certificate bypass is not qualification")
    capabilities["browserName"] = "oryn"
    capabilities["acceptInsecureCerts"] = False
    result["capabilities"] = capabilities
    bound = load_contract(kwargs.get('oryn_run_contract'), kwargs.get('binary'), kwargs.get('webdriver_binary'))
    descriptor = bound.get('parent_support_descriptor')
    if descriptor is not None:
        if test_type != 'testharness':
            raise ValueError('support evidence requires original testharness conversion')
        result['oryn_support_descriptor'] = descriptor
        result['oryn_support_candidate'] = bound['candidate_sha256']
    from .placement import configuration
    placement = configuration()
    if placement:
        if test_type != 'testharness':
            raise ValueError('placement collection requires testharness')
        result['oryn_placement'] = placement
        result['cleanup_after_test'] = False
    return result


class OrynBrowser(WebDriverBrowser):
    """Command-server lifecycle belongs to upstream's process supervisor."""

    # Both upstream initialization timers include contract validation and setup.
    init_timeout: float = 120

    def __init__(self, *args, oryn_run_contract=None, **kwargs):
        self._contract_path = oryn_run_contract
        super().__init__(*args, **kwargs)
        self._driver_token = secrets.token_hex(32)
        descriptor, self._token_path = tempfile.mkstemp(prefix="oryn-driver-", suffix=".token")
        with os.fdopen(descriptor, "w") as token_file:
            token_file.write(self._driver_token)

    def start(self, group_metadata, **kwargs):
        self._recovery_settled = False
        self._process_settlement = None
        self._timing_identity = None
        self._spawn_span = self._ready_span = None
        with span(getattr(self, "logger", None), "browser.start") as measured:
            try:
                return super().start(group_metadata, **kwargs)
            finally:
                import sys
                error = sys.exc_info()
                collector = getattr(self, "_native_timing", None)
                if collector is not None:
                    try:
                        collector.private()
                    except OSError as cleanup_error:
                        if error[1] is None:
                            raise
                        error[1].add_note('native diagnostic descriptor reset failed: ' + type(cleanup_error).__name__)
                    measured.service = collector.service
                for name in ("_ready_span", "_spawn_span"):
                    active = getattr(self, name, None)
                    if active is not None:
                        active.__exit__(*error)
                        setattr(self, name, None)
                measured.identity = getattr(self, "_timing_identity", None)
                if collector is not None and not WebDriverBrowser.is_alive(self):
                    collector.finish(process_stopped=True)

    def create_output_handler(self, command):
        browser = self
        collector = self._native_timing

        class ServiceOutputHandler(OutputHandler):
            def after_process_start(self, pid):
                # Keep the exact upstream hook and reset descriptor inheritance
                # even when its logger/state transition raises.
                collector.private()
                collector.service["pid"] = pid
                active = browser._spawn_span
                if active is not None:
                    active.__exit__(None, None, None)
                    browser._spawn_span = None
                with span(browser.logger, "driver.identity_observation",
                          identity=browser._timing_identity, service=collector.service):
                    collector.published(pid)
                browser._ready_span = span(browser.logger, "driver.tcp_ready_wait",
                    identity=browser._timing_identity, service=collector.service)
                browser._ready_span.__enter__()
                result = super().after_process_start(pid)
                from .recovery import ProcessSettlement
                browser._process_settlement = ProcessSettlement(pid)
                return result

        with span(self.logger, "driver.spawn_slot_wait",
                  identity=self._timing_identity, service=collector.service):
            collector.inherit_for_spawn()
        # Use the existing upstream deadline; descriptor admission consumes it too.
        deadline = getattr(self, "init_deadline", None)
        if deadline is not None and time.time() >= deadline:
            raise TimeoutError("Oryn initialization deadline expired before driver spawn")
        self._spawn_span = span(self.logger, "driver.construct_and_spawn",
                                identity=self._timing_identity, service=collector.service)
        self._spawn_span.__enter__()
        return ServiceOutputHandler(self.logger, command)

    def stop(self, force=False):
        # Upstream owns every kill/wait and the actual return value.
        observer = getattr(self, '_process_settlement', None)
        failure = None
        try:
            if observer is not None:
                observer.observe()
        except BaseException as error:
            failure = error
        try:
            result = super().stop(force=force)
        except BaseException as error:
            if failure is None:
                failure = error
            else:
                failure.add_note('upstream process stop failed: ' + type(error).__name__)
        finally:
            try:
                if observer is not None:
                    observer.finish()
                    self._recovery_settled = not WebDriverBrowser.is_alive(self)
            except BaseException as error:
                if failure is None:
                    failure = error
                else:
                    failure.add_note('process settlement failed: ' + type(error).__name__)
            collector = getattr(self, "_native_timing", None)
            if collector is not None:
                collector.finish(process_stopped=not WebDriverBrowser.is_alive(self))
        if failure is not None:
            raise failure
        return result

    def make_command(self):
        with span(getattr(self, "logger", None), "browser.make_command") as measured:
            contract = load_contract(self._contract_path, self.binary, self.webdriver_binary)
            if 'typed_run' in contract:
                binding = contract.get('worker_binding')
                if (not isinstance(binding, dict) or binding.get('variable') != 'ORYN_PAGE_WORKER'
                        or not isinstance(binding.get('path'), str)
                        or self.env.get('ORYN_PAGE_WORKER') != binding.get('path')):
                    raise ValueError('driver environment differs from typed sealed worker')
            self._timing_identity = contract_identity(contract)
            measured.bind_contract(contract)
            previous = getattr(self, "_native_timing", None)
            if previous is not None and not previous.closed:
                # Never replace a collector still held by a live old service.
                raise RuntimeError("previous native diagnostic service is not settled")
            with span(getattr(self, "logger", None), "browser.diagnostic_setup",
                      identity=self._timing_identity):
                self._native_timing = Collector(getattr(self, "logger", None), self._timing_identity)
            measured.service = self._native_timing.service
            return [self.webdriver_binary, "--host", self.host, "--port", str(self.port),
                    "--browser-binary", self.binary, "--auth-token-file", self._token_path,
                    "--configuration", contract["fixture_path"],
                    "--configuration-sha256", contract["fixture_sha256"],
                    "--admitted-candidate-manifest", contract["candidate_manifest_path"],
                    "--admitted-candidate-manifest-raw-sha256", contract["candidate_manifest_raw_sha256"],
                    "--admitted-candidate-sha256", contract["candidate_sha256"],
                    "--admitted-runtime-sha256", contract["admitted_runtime_sha256"],
                    "--diagnostic-fd", str(self._native_timing.fd)]

    def executor_browser(self):
        browser_class, kwargs = super().executor_browser()
        return browser_class, {**kwargs, "oryn_driver_token": self._driver_token,
                               "oryn_timing_identity": getattr(self, "_timing_identity", None),
                               "oryn_timing_service": dict(self._native_timing.service)}

    def cleanup(self):
        try:
            os.unlink(self._token_path)
        except FileNotFoundError:
            pass
        super().cleanup()


def env_extras(**kwargs):
    contract = load_contract(kwargs.get("oryn_run_contract"), kwargs.get("binary"), kwargs.get("webdriver_binary"))
    return [FixtureEnvironment(contract["path"])]


def add_arguments(parser):
    parser.add_argument("--oryn-run-contract", help="Hash-bound native candidate and fixture run contract")


def product():
    return Product(
        name="oryn",
        browser_classes={None: OrynBrowser},
        check_args=check_args,
        get_browser_kwargs=browser_kwargs,
        get_executor_kwargs=executor_kwargs,
        env_options={"server_host": "127.0.0.1", "supports_debugger": False},
        get_env_extras=env_extras,
        run_info_extras=run_info_extras,
        add_arguments=add_arguments,
        get_timeout_multiplier=get_timeout_multiplier,
        executor_classes={
            "testharness": OrynTestharnessExecutor,
            "reftest": OrynRefTestExecutor,
            "crashtest": OrynCrashtestExecutor,
            "print-reftest": OrynPrintRefTestExecutor,
        },
    )
