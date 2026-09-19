"""Parent navigation history between result boundaries; raw statuses stay intact."""


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
    def convert_result(self, test, *args, **kwargs):
        return attach(self, test, super().convert_result(test, *args, **kwargs))
