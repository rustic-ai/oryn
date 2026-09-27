"""Parent navigation history between result boundaries; raw statuses stay intact."""

from .timing import span


def snapshot(protocol):
    try:
        value = protocol.webdriver.send_session_command("GET", "oryn/diagnostics/navigation")
        if not isinstance(value, dict) or value.get("schema_version") != 1:
            raise ValueError("native navigation snapshot schema mismatch")
        return value
    except Exception as error:
        return {"schema_version": 1, "collection_error": str(error)}


def attach(executor, test, converted):
    protocol = executor.protocol
    with span(getattr(executor, "logger", None), "test.navigation_snapshot",
              service=getattr(getattr(executor, "browser", None), "oryn_timing_service", None),
              identity=getattr(getattr(executor, "browser", None), "oryn_timing_identity", None),
              webdriver_session_id=getattr(protocol, "_timing_session_id", None),
              test_id=test.url, custody_id=getattr(protocol, "_custody_session", None)):
        after = snapshot(protocol)
    evidence = {"schema_version": 1, "test": test.url,
                "interval_start": getattr(protocol, "_navigation_boundary", None),
                "before": getattr(protocol, "_navigation_baseline", None), "after": after}
    protocol._navigation_baseline = after
    protocol._navigation_boundary = test.url
    result = converted[0]
    result.extra = dict(result.extra or {})
    result.extra["oryn_navigation"] = evidence
    return converted


class NavigationEvidenceMixin:
    def run_test(self, test):
        with span(getattr(self, "logger", None), "test.total",
                  service=getattr(getattr(self, "browser", None), "oryn_timing_service", None),
                  identity=getattr(getattr(self, "browser", None), "oryn_timing_identity", None),
                  webdriver_session_id=getattr(self.protocol, "_timing_session_id", None),
                  test_id=test.url,
                  custody_id=getattr(self.protocol, "_custody_session", None)):
            return super().run_test(test)

    def convert_result(self, test, *args, **kwargs):
        with span(getattr(self, "logger", None), "test.result_conversion",
                  service=getattr(getattr(self, "browser", None), "oryn_timing_service", None),
                  identity=getattr(getattr(self, "browser", None), "oryn_timing_identity", None),
                  webdriver_session_id=getattr(self.protocol, "_timing_session_id", None),
                  test_id=test.url,
                  custody_id=getattr(self.protocol, "_custody_session", None)):
            return attach(self, test, super().convert_result(test, *args, **kwargs))
