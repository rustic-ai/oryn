"""Use upstream execution, reference readiness, fuzzy metadata and result types."""
import os
import secrets
import tempfile

from wptrunner.browsers.base import WebDriverBrowser, get_timeout_multiplier, require_arg
from wptrunner.executors import executor_kwargs as upstream_executor_kwargs
from .protocol import (
    OrynCrashtestExecutor, OrynPrintRefTestExecutor,
    OrynRefTestExecutor, OrynTestharnessExecutor,
)
from wptrunner.products import Product
from .provenance import load_contract, run_info_extras, FixtureEnvironment


def check_args(**kwargs):
    require_arg(kwargs, "binary")
    require_arg(kwargs, "webdriver_binary")
    contract = load_contract(kwargs.get("oryn_run_contract"), kwargs["binary"], kwargs["webdriver_binary"])
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
    check_args(**kwargs)
    contract = load_contract(kwargs.get("oryn_run_contract"), kwargs["binary"], kwargs["webdriver_binary"])
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

    def __init__(self, *args, oryn_run_contract=None, **kwargs):
        self._contract_path = oryn_run_contract
        super().__init__(*args, **kwargs)
        self._driver_token = secrets.token_hex(32)
        descriptor, self._token_path = tempfile.mkstemp(prefix="oryn-driver-", suffix=".token")
        with os.fdopen(descriptor, "w") as token_file:
            token_file.write(self._driver_token)

    def make_command(self):
        contract = load_contract(self._contract_path, self.binary, self.webdriver_binary)
        return [self.webdriver_binary, "--host", self.host, "--port", str(self.port),
                "--browser-binary", self.binary, "--auth-token-file", self._token_path,
                "--configuration", contract["fixture_path"],
                "--configuration-sha256", contract["fixture_sha256"]]

    def executor_browser(self):
        browser_class, kwargs = super().executor_browser()
        return browser_class, {**kwargs, "oryn_driver_token": self._driver_token}

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
