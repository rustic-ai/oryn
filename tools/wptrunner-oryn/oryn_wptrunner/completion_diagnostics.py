"""Opt-in post-result observations; not fixture outcomes or admission authority."""
import json
import os
from urllib.parse import urlsplit

SCRIPT = """return {
  ready_state: document.readyState,
  frame_count: frames.length,
  script_count: document.querySelectorAll('script').length,
  location_kind: location.href === 'about:blank' ? 'initial' :
    location.pathname === '/common/dispatcher/executor.html' ? 'dispatcher' : 'other',
  executor_type: typeof executeOrders,
  receive_type: typeof receive,
  send_queue_count: typeof sendQueues === 'undefined' ? null : sendQueues.size,
  uuid_is_string: typeof uuid === 'undefined' ? null : typeof uuid === 'string'
};"""


def capture(protocol, url):
    if os.environ.get('ORYN_WPT_COMPLETION_DIAGNOSTICS') != '1':
        return
    # Explicitly selected diagnostic population; never add commands to ordinary
    # or official runs. Observe before upstream finally closes the test windows.
    if not isinstance(url, str):
        return
    try:
        selected = urlsplit(url).path == '/FileAPI/BlobURL/cross-partition.https.html'
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
                for name in ('frame_count', 'script_count', 'send_queue_count'):
                    count = value.get(name)
                    if type(count) in (int, float) and 0 <= count <= 1_000_000 and int(count) == count:
                        observation[name] = int(count)
                if value.get('ready_state') in ('loading', 'interactive', 'complete'):
                    observation['ready_state'] = value['ready_state']
                for name in ('executor_type', 'receive_type'):
                    if value.get(name) in ('undefined', 'function'):
                        observation[name] = value[name]
                if type(value.get('uuid_is_string')) is bool:
                    observation['uuid_is_string'] = value['uuid_is_string']
                if value.get('location_kind') in ('initial', 'dispatcher', 'other'):
                    observation['location_kind'] = value['location_kind']
            records.append({'window_index': index, 'observation': observation})
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
