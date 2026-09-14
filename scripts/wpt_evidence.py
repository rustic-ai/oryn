"""Conservative WPT diagnostic annotations and static script dependencies."""
import re
import hashlib
from html.parser import HTMLParser
from urllib.parse import urljoin, urlsplit


def resource_key(url, query_sha256=None):
    parsed = urlsplit(url)
    if query_sha256 is None and parsed.query:
        query_sha256 = hashlib.sha256(parsed.query.encode()).hexdigest()
    return parsed.path + ('?sha256=' + query_sha256 if query_sha256 else '')


def resource_integrity_errors(row):
    """Observation completeness is not a declaration that prerequisites succeeded."""
    resources = row.get('resources')
    if not isinstance(resources, list) or not resources:
        return ['resource_observations_missing']
    errors = []
    for index, resource in enumerate(resources):
        if not isinstance(resource, dict):
            return ['resource_observation_malformed']
        if (type(resource.get('sequence')) is not int or resource.get('sequence') != index or type(resource.get('page_id')) is not int
                or type(resource.get('worker_pid')) is not int or resource['worker_pid'] < 1
                or not isinstance(resource.get('url'), str)
                or resource.get('terminal') not in ('complete', 'failed', 'cancelled', 'cancelled_or_abandoned')
                or type(resource.get('elapsed_ms')) not in (int, float)):
            errors.append('resource_identity_or_terminal_unverified')
        if resource.get('terminal') == 'complete' and (
                type(resource.get('status')) is not int
                or not 100 <= resource['status'] <= 599
                or not re.fullmatch('[0-9a-f]{64}', resource.get('response_sha256') or '')):
            errors.append('resource_response_unverified')
    return sorted(set(errors))


def prerequisite_review_errors(row, review):
    errors = []
    required = {resource_key(value) for value in row.get('dependencies', [])}
    verified = {resource_key(value) for value in review.get('verified_dependencies', [])}
    if not required <= verified:
        errors.append('static_prerequisite_audit_incomplete')
    audited = review.get('verified_resources', {})
    allowances = review.get('expected_resource_errors', {})
    for resource in row.get('resources', []):
        key = str(resource['sequence'])
        expected = {field: resource.get(field) for field in
                    ('url', 'query_sha256', 'status', 'mime', 'response_sha256', 'terminal')}
        if audited.get(key) != expected:
            errors.append('observed_prerequisite_audit_incomplete')
        if (resource.get('terminal') != 'complete' or not 200 <= (resource.get('status') or 0) < 400
                or resource.get('destination') in ('Script', 'Worker')
                and not any(mime in (resource.get('mime') or '').lower()
                            for mime in ('javascript', 'ecmascript'))):
            allowance = allowances.get(key, {})
            if allowance.get('observation') != expected or not allowance.get('reason'):
                errors.append('unexpected_resource_error')
    observed = {resource_key(resource['url'], resource.get('query_sha256'))
                for resource in row.get('resources', [])}
    unused = review.get('unobserved_dependencies', {})
    if any(not unused.get(key) for key in required - observed):
        errors.append('unobserved_static_prerequisite_unexplained')
    operations = {item.get('operation') for item in (row.get('result', {}).get('testdriver') or [])}
    if not operations <= set(review.get('verified_automation_operations', [])):
        errors.append('automation_operation_audit_incomplete')
    if 'expected_native_automation_calls' in review:
        result = row.get('result') or {}
        native = result.get('automation_trace') or {}
        calls = sum(event.get('outcome') == 'completed' for event in result.get('testdriver') or [])
        if (native.get('complete') is not True or native.get('scope') != 'page_isolate'
                or calls != review['expected_native_automation_calls']):
            errors.append('reviewed_native_automation_missing')
    sockets = (row.get('resource_audit') or {}).get('websockets') or []
    if sockets != review.get('verified_websockets', []):
        errors.append('websocket_prerequisite_audit_incomplete')
    for index, socket in enumerate(sockets):
        if socket.get('terminal') != 'closed' or socket.get('handshake_status') != 101:
            allowance = review.get('expected_websocket_errors', {}).get(str(index), {})
            if allowance.get('observation') != socket or not allowance.get('reason'):
                errors.append('unexpected_websocket_terminal')
    return sorted(set(errors))


def completion_errors(result):
    """Validate types strictly: bool is not a numeric WPT status."""
    if not isinstance(result, dict):
        return ['completion_not_object']
    errors = []
    if type(result.get('complete')) is not bool:
        errors.append('invalid_completion_flag')
    if type(result.get('harness')) is not int or result['harness'] not in range(4):
        errors.append('invalid_harness_status')
    tests = result.get('tests')
    if not isinstance(tests, list):
        return errors + ['invalid_subtests']
    names = set()
    for test in tests:
        if (not isinstance(test, dict) or not isinstance(test.get('name'), str) or
                type(test.get('status')) is not int or test['status'] not in range(4)):
            errors.append('invalid_subtest')
            break
        if test['name'] in names:
            errors.append('duplicate_subtest_name')
        names.add(test['name'])
    return errors


def current_harness_errors(row):
    """Current qualification checks; historical diagnostics remain readable."""
    errors = []
    audit = row.get('resource_audit') or {}
    if audit.get('gaps') or audit.get('overflow'):
        errors.append('transport_audit_incomplete')
    for socket in audit.get('websockets') or []:
        if (type(socket.get('page_id')) is not int or type(socket.get('socket_id')) is not int
                or socket.get('terminal') not in ('closed', 'close_requested', 'failed', 'cancelled_or_abandoned')
                or any(type(socket.get(key)) is not int or socket[key] < 0 for key in ('sent_bytes', 'received_bytes'))):
            errors.append('websocket_evidence_unverified')
    fixture = row.get('fixture') or {}
    if (fixture.get('server') != 'direct_wptserve' or not fixture.get('id')
            or fixture.get('shutdown_complete') is not True or fixture.get('error')
            or row.get('fixture_error')):
        errors.append('fixture_integrity_unverified')
    generations = fixture.get('generations') or []
    reference = row.get('chromium_reference')
    if (reference or {}).get('automation_bridge_errors'):
        errors.append('reference_automation_bridge_unverified')
    if ((row.get('result') or {}).get('automation_overflow') or
            (reference or {}).get('automation_overflow') or
            ((reference or {}).get('completion') or {}).get('automation_overflow')):
        errors.append('automation_trace_overflow')
    expected = ['oryn', 'chromium'] if reference is not None else ['oryn']
    if ([item.get('backend') for item in generations] != expected
            or len({item.get('id') for item in generations}) != len(generations)
            or any(not item.get('id') for item in generations)):
        errors.append('fixture_isolation_unverified')
    if reference is not None:
        if reference.get('initial_focus_verified') is not True:
            errors.append('reference_initial_focus_unverified')
        if reference.get('entry_focus_verified') is not True:
            errors.append('reference_entry_focus_unverified')
        if reference.get('entry_focus_phase') != 'report_ready':
            errors.append('reference_focus_phase_unverified')
        if reference.get('focus_isolation') != 'host_serialized':
            errors.append('reference_focus_isolation_unverified')
        if (reference.get('error') or reference.get('valid') is not True
                or reference.get('validation_errors')
                or (reference.get('completion') or {}).get('complete') is not True
                or (reference.get('completion') or {}).get('harness') != 0):
            errors.append('reference_completion_unverified')
        if reference.get('same_subtests') is not True:
            errors.append('reference_subtest_identity_unverified')
    reference_result = dict((reference or {}).get('completion') or {})
    if reference is not None and 'automation_events' in reference:
        reference_result['testdriver'] = reference['automation_events']
    for result in [row.get('result') or {}, reference_result]:
        native = result.get('automation_trace')
        if native is not None:
            if (not isinstance(native, dict) or native.get('schema_version') != 1
                    or native.get('scope') != 'page_isolate' or native.get('complete') is not True
                    or native.get('overflow') is not False
                    or native.get('events') != result.get('testdriver')):
                errors.append('native_automation_trace_incomplete')
        operations = {}
        events = result.get('testdriver') or []
        if not isinstance(events, list):
            errors.append('automation_trace_unverified')
            continue
        for sequence, event in enumerate(events):
            if not isinstance(event, dict):
                errors.append('automation_trace_unverified')
                continue
            identifier = event.get('id')
            if any(event.get(field) is not None and type(event[field]) is not int
                   for field in ('context_id','document_generation')):
                errors.append('automation_trace_unverified')
                continue
            key = (event.get('backend'), event.get('context_id'), event.get('document_generation'), identifier)
            if native is not None and (event.get('sequence') != sequence
                    or type(event.get('context_id')) is not int or event['context_id'] < 0
                    or type(event.get('document_generation')) is not int or event['document_generation'] < 0
                    or event.get('backend') != 'oryn'):
                errors.append('native_automation_identity_unverified')
            if (type(identifier) is not int or identifier < 1
                    or not isinstance(event.get('operation'), str)
                    or event.get('backend') not in ('oryn','chromium')):
                errors.append('automation_trace_unverified')
                continue
            if event.get('outcome') == 'started' and key not in operations:
                operations[key] = (event['operation'], 'started')
            elif (operations.get(key) == (event['operation'], 'started')
                  and event.get('outcome') in ('completed', 'unsupported', 'failed')):
                operations[key] = (event['operation'], event['outcome'])
            else:
                errors.append('automation_trace_unverified')
        if any(state != 'completed' for _, state in operations.values()):
            errors.append('automation_execution_unqualified')
    return sorted(set(errors))


class ScriptReferences(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.base = None
        self.scripts = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'base' and self.base is None and attrs.get('href') is not None:
            self.base = attrs['href']
        if tag == 'script' and attrs.get('src'):
            self.scripts.append(attrs['src'])


def template_failure(status, body):
    return status == 500 and b"config_replacement" in body and b"Traceback" in body


def confirmed_dependency_failure(check):
    return check.get('template_failure') is True


def diagnostics(row):
    result = row.get("result", {})
    text = "\n".join(row.get("diagnostics", [])) + '\n' + str(row.get('execution', {}).get('error') or '')
    messages = str(result.get("harnessMessage", "")) + "\n" + "\n".join(
        str(test.get("message", "")) for test in result.get("tests", []))
    tags = set()
    if row.get("failure_domain") == "timeout":
        tags.add("outer_timeout")
    if result.get("harness") == 2:
        tags.add("harness_timeout")
    if 'generated platform operation wall time' in text:
        tags.add("generated_operation_wall_limit")
    elif 'WorkerLimitExceeded' in text and 'wall time' in text:
        tags.add("worker_wall_limit")
    if "WorkerCrashed" in text or "worker crashed" in text.lower():
        tags.add("reported_worker_crash")
    for gap in row.get('resource_audit', {}).get('gaps', []):
        tags.add('resource_audit_gap:' + gap)
    if "not implemented by testdriver-vendor.js" in messages:
        tags.add("testdriver_operation_not_implemented")
    if any(event.get('outcome') == 'unsupported' for event in result.get('testdriver') or []):
        tags.add('testdriver_operation_not_implemented')
    if row.get('fixture_error') or row.get('fixture', {}).get('error'):
        tags.add('fixture_integrity_failure')
    return sorted(tags)


def script_dependencies(checkout, source):
    """Discover literal script references only; never claim dynamic completeness."""
    pending = ["/" + source.lstrip("/")]
    seen = set()
    dependencies = set()
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        path = (checkout / urlsplit(current).path.lstrip("/")).resolve()
        if checkout.resolve() not in path.parents or not path.is_file():
            continue
        body = path.read_text(encoding="utf-8", errors="replace")
        base = current
        refs = []
        if path.suffix.lower() in ('.html', '.htm', '.xhtml', '.xht', '.svg'):
            parser = ScriptReferences()
            parser.feed(body)
            refs = parser.scripts
            if parser.base is not None:
                try:
                    base = urljoin(current, parser.base)
                except ValueError:
                    continue
        refs += re.findall(r'^\s*//\s*META:\s*script=(\S+)', body, re.M)
        for args in re.findall(r'\bimportScripts\s*\(([^)]*)\)', body):
            refs += re.findall(r'["\']([^"\']+)["\']', args)
        # Literal module edges only. Runtime evidence remains authoritative for
        # computed specifiers and must not be replaced by this discovery pass.
        refs += re.findall(r'\b(?:import|export)\s+(?:[^;\n]*?\s+from\s*)?["\']([^"\']+)["\']', body)
        refs += re.findall(r'\bimport\s*\(\s*["\']([^"\']+)["\']\s*\)', body)
        for ref in refs:
            try:
                url = urljoin(base, ref)
                parsed = urlsplit(url)
            except ValueError:
                # Malformed URL fixtures are browser tests, not dependencies
                # the static discovery pass can safely fetch.
                continue
            if parsed.scheme or parsed.netloc or "{{" in url:
                continue
            dependencies.add(url)
            pending.append(url)
    return sorted(dependencies)
