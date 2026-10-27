"""Opt-in original-command replay after the original result; never verification."""
import json
import os
import uuid
from urllib.parse import urlsplit


def replay(protocol, url):
    if os.environ.get('ORYN_WPT_COMPLETION_REPLAY') != '1':
        return
    if not isinstance(url, str):
        return
    try:
        if urlsplit(url).path != '/FileAPI/BlobURL/cross-partition.https.html':
            return
    except ValueError:
        return
    driver = None
    original = None
    observations = []
    try:
        driver = protocol.webdriver
        original = driver.send_session_command('GET', 'window', timeout=2)
        handles = driver.send_session_command('GET', 'window/handles', timeout=2)
        root = popup = None
        task = None
        for handle in handles[:4]:
            driver.send_session_command('POST', 'window', {'handle': handle}, timeout=2)
            kind = driver.send_session_command('POST', 'execute/sync', {'script':
                "return location.pathname === '/FileAPI/BlobURL/cross-partition.https.html' ? 'root' : location.pathname === '/common/dispatcher/executor.html' ? 'popup' : 'other';",
                'args': []}, timeout=2)
            if kind == 'root':
                root = handle
                task = driver.send_session_command('POST', 'execute/sync', {'script':
                    "return typeof add_iframe_js === 'function' ? add_iframe_js(location.origin, arguments[0]) : null;",
                    'args': [str(uuid.uuid4())]}, timeout=2)
            elif kind == 'popup' and popup is None:
                popup = handle
        if root is None or popup is None or not isinstance(task, str) or len(task) > 16384:
            observations.append({'target_available': False})
        else:
            driver.send_session_command('POST', 'window', {'handle': popup}, timeout=2)
            # First allow the existing executor to advance without injecting a
            # dispatcher task. A second frame after replay can otherwise hide
            # that the original task resumed only during a selected command.
            baseline = driver.send_session_command('POST', 'execute/async', {
                'script': "const done=arguments[arguments.length-1];setTimeout(()=>done({frame_count:frames.length,script_count:document.querySelectorAll('script').length}),500);",
                'args': []}, timeout=2)
            passive = {'passive_before_replay': True}
            if isinstance(baseline, dict):
                for key in ('script_count', 'frame_count'):
                    count = baseline.get(key)
                    if type(count) in (int, float) and 0 <= count <= 1000000 and int(count) == count:
                        passive[key] = int(count)
            observations.append(passive)
            # This intentionally executes the original parent-generated command
            # only after the original result. Never turn its outcome into PASS.
            value = driver.send_session_command('POST', 'execute/async', {'script':
                "const task=arguments[0],done=arguments[arguments.length-1];Promise.resolve().then(()=>eval('(async()=>{'+task+'})()')).then(()=>done({settled:true,failed:false,script_count:document.querySelectorAll('script').length,frame_count:frames.length}),error=>done({settled:true,failed:true,exception_type:error?.name,body_present:document.body!==null,script_count:document.querySelectorAll('script').length,frame_count:frames.length}));",
                'args': [task]}, timeout=20)
            safe = {'target_available': True}
            if isinstance(value, dict):
                for key in ('settled', 'failed', 'body_present'):
                    if type(value.get(key)) is bool:
                        safe[key] = value[key]
                for key in ('script_count', 'frame_count'):
                    count = value.get(key)
                    if type(count) in (int, float) and 0 <= count <= 1000000 and int(count) == count:
                        safe[key] = int(count)
                if value.get('exception_type') in ('Error', 'TypeError', 'SyntaxError', 'ReferenceError', 'SecurityError', 'NetworkError', 'AbortError'):
                    safe['exception_type'] = value['exception_type']
            observations.append(safe)
    except Exception as error:
        observations.append({'collection_error_type': type(error).__name__})
    finally:
        if original is not None:
            try:
                driver.send_session_command('POST', 'window', {'handle': original}, timeout=2)
            except Exception as error:
                observations.append({'restore_error_type': type(error).__name__})
        try:
            protocol.logger.info('oryn-post-result-command-replay ' + json.dumps(observations))
        except Exception:
            pass
