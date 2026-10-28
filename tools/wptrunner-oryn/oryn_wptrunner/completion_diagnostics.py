"""Opt-in post-result observations; not fixture outcomes or admission authority."""
import json
import os
from urllib.parse import urlsplit

SCRIPT = """return {
  ready_state: document.readyState,
  body_present: document.body !== null,
  iframe_task_parses: typeof add_iframe_js !== 'function' ? null : (() => {
    try { new Function('return (async()=>{' + add_iframe_js('https://diagnostic.invalid', 'diagnostic') + '})'); return true; }
    catch (_) { return false; }
  })(),
  import_source_native: typeof importScript !== 'function' ? null : String(importScript).includes('[native code]'),
  frame_count: frames.length,
  child_dispatcher: (() => {
    if (!frames.length) return null;
    try {
      const child = frames[0];
      return {accessible:true, ready_state:child.document.readyState,
        script_count:child.document.querySelectorAll('script').length,
        executor_type:typeof child.executeOrders, receive_type:typeof child.receive,
        request_count:typeof child.__oryn_context_trace === 'undefined' ? 0 :
          child.__oryn_context_trace.filter(e=>e.kind==='request_started').length};
    } catch (_) { return {accessible:false}; }
  })(),
  script_count: document.querySelectorAll('script').length,
  location_kind: location.href === 'about:blank' ? 'initial' :
    location.pathname === '/common/dispatcher/executor.html' ? 'dispatcher' : 'other',
  executor_type: typeof executeOrders,
  worker_path_present: typeof executor_worker_path === 'string',
  token_function: typeof token === 'function',
  receive_type: typeof receive,
  send_queue_count: typeof sendQueues === 'undefined' ? null : sendQueues.size,
  uuid_is_string: typeof uuid === 'undefined' ? null : typeof uuid === 'string',
  encoding_span_count: document.getElementById('scrwin')?.contentDocument?.querySelectorAll('span').length,
  encoding_live_span_count: document.getElementById('scrwin')?.contentDocument?.getElementsByTagName('span').length,
  encoding_child_ready_state: document.getElementById('scrwin')?.contentDocument?.readyState,
  encoding_test_count: typeof tests === 'undefined' || !Array.isArray(tests) ? null : tests.filter(Boolean).length,
  recent_requests: typeof __oryn_context_trace === 'undefined' ? [] :
    __oryn_context_trace.filter(e => ['request_started','response_received','stream_chunk','capability_failure'].includes(e.kind)).slice(-8).map(e => ({
      kind: e.kind,
      method: ['GET','POST'].includes(e.detail?.method) ? e.detail.method : null,
      status: typeof e.detail?.status === 'number' ? e.detail.status : null,
      bytes: typeof e.detail?.bytes === 'number' ? e.detail.bytes : null,
      dispatcher: typeof e.detail?.url === 'string' && new URL(e.detail.url,location.href).pathname === '/common/dispatcher/dispatcher.py',
      uuid_matches: typeof uuid === 'string' && typeof e.detail?.url === 'string' ? new URL(e.detail.url,location.href).searchParams.get('uuid') === uuid : null
    }))
};"""


def capture(protocol, url):
    if os.environ.get('ORYN_WPT_COMPLETION_DIAGNOSTICS') != '1':
        return
    # Explicitly selected diagnostic population; never add commands to ordinary
    # or official runs. Observe before upstream finally closes the test windows.
    if not isinstance(url, str):
        return
    try:
        selected = urlsplit(url).path in (
            '/FileAPI/BlobURL/cross-partition.https.html',
            '/content-security-policy/inside-worker/dedicatedworker-connect-src.html',
            '/encoding/legacy-mb-japanese/euc-jp/eucjp-decode-cseucpkdfmtjapanese.html',
        )
    except ValueError:
        return
    if not selected:
        return
    driver = None
    original = None
    records = []
    try:
        driver = protocol.webdriver
        original = driver.send_session_command('GET', 'window', timeout=2)
        handles = driver.send_session_command('GET', 'window/handles', timeout=2)
        for index, handle in enumerate(handles[:4]):
            driver.send_session_command('POST', 'window', {'handle': handle}, timeout=2)
            value = driver.send_session_command('POST', 'execute/sync',
                                                {'script': SCRIPT, 'args': []}, timeout=2)
            observation = {}
            if isinstance(value, dict):
                for name in ('frame_count', 'script_count', 'send_queue_count', 'encoding_span_count', 'encoding_live_span_count', 'encoding_test_count'):
                    count = value.get(name)
                    if type(count) in (int, float) and 0 <= count <= 1_000_000 and int(count) == count:
                        observation[name] = int(count)
                if value.get('ready_state') in ('loading', 'interactive', 'complete'):
                    observation['ready_state'] = value['ready_state']
                if value.get('encoding_child_ready_state') in ('loading', 'interactive', 'complete'):
                    observation['encoding_child_ready_state'] = value['encoding_child_ready_state']
                for name in ('executor_type', 'receive_type'):
                    if value.get(name) in ('undefined', 'function'):
                        observation[name] = value[name]
                for name in ('uuid_is_string', 'body_present', 'iframe_task_parses', 'import_source_native', 'worker_path_present', 'token_function'):
                    if type(value.get(name)) is bool:
                        observation[name] = value[name]
                if value.get('location_kind') in ('initial', 'dispatcher', 'other'):
                    observation['location_kind'] = value['location_kind']
                child = value.get('child_dispatcher')
                if isinstance(child, dict) and type(child.get('accessible')) is bool:
                    safe_child = {'accessible': child['accessible']}
                    if child['accessible']:
                        for name in ('script_count', 'request_count'):
                            count = child.get(name)
                            if type(count) in (int, float) and 0 <= count <= 1_000_000 and int(count) == count:
                                safe_child[name] = int(count)
                        for name in ('executor_type', 'receive_type'):
                            if child.get(name) in ('undefined', 'function'):
                                safe_child[name] = child[name]
                        if child.get('ready_state') in ('loading', 'interactive', 'complete'):
                            safe_child['ready_state'] = child['ready_state']
                    observation['child_dispatcher'] = safe_child
                recent = value.get('recent_requests')
                if isinstance(recent, list):
                    safe = []
                    for entry in recent[:8]:
                        if not isinstance(entry, dict):
                            continue
                        if entry.get('kind') not in ('request_started', 'response_received', 'stream_chunk', 'capability_failure'):
                            continue
                        item = {'kind': entry['kind']}
                        if entry.get('method') in ('GET', 'POST'):
                            item['method'] = entry['method']
                        for key in ('status', 'bytes'):
                            number = entry.get(key)
                            if type(number) in (int, float) and 0 <= number <= 100_000_000 and int(number) == number:
                                item[key] = int(number)
                        for key in ('dispatcher', 'uuid_matches'):
                            if type(entry.get(key)) is bool:
                                item[key] = entry[key]
                        safe.append(item)
                    observation['recent_requests'] = safe
            records.append({'window_index': index, 'observation': observation})
            if observation.get('frame_count', 0) > 0:
                # WindowProxy property forwarding is not evidence of globals
                # in the child realm. Observe at most the first child through
                # the driver's existing realm selection, then always restore.
                try:
                    driver.send_session_command('POST', 'frame', {'id': 0}, timeout=2)
                    child_value = driver.send_session_command('POST', 'execute/sync',
                        {'script': SCRIPT, 'args': []}, timeout=2)
                    child_observation = {}
                    if isinstance(child_value, dict):
                        for name in ('script_count', 'send_queue_count', 'frame_count'):
                            count = child_value.get(name)
                            if type(count) in (int, float) and 0 <= count <= 1_000_000 and int(count) == count:
                                child_observation[name] = int(count)
                        for name in ('executor_type', 'receive_type'):
                            if child_value.get(name) in ('undefined', 'function'):
                                child_observation[name] = child_value[name]
                        if child_value.get('ready_state') in ('loading', 'interactive', 'complete'):
                            child_observation['ready_state'] = child_value['ready_state']
                        for name in ('worker_path_present', 'token_function'):
                            if type(child_value.get(name)) is bool:
                                child_observation[name] = child_value[name]
                    records.append({'window_index': index, 'frame_index': 0,
                                    'observation': child_observation})
                finally:
                    driver.send_session_command('POST', 'frame', {'id': None}, timeout=2)
    except Exception as error:
        records.append({'collection_error_type': type(error).__name__})
    finally:
        if original is not None:
            try:
                driver.send_session_command('POST', 'window', {'handle': original}, timeout=2)
            except Exception as error:
                records.append({'restore_error_type': type(error).__name__})
        # No UUIDs, handles, tokens, DOM text or exception payloads are recorded.
        try:
            protocol.logger.info('oryn-completion-diagnostic ' + json.dumps(records))
        except Exception:
            pass
    if os.environ.get('ORYN_WPT_COMPLETION_REPLAY') in ('1', 'load'):
        from .completion_replay import replay
        replay(protocol, url)
