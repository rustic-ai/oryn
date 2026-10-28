"""Opt-in original-command replay after the original result; never verification."""
import json
import os
import uuid
from urllib.parse import urlsplit


def replay(protocol, url):
    mode = os.environ.get('ORYN_WPT_COMPLETION_REPLAY')
    if mode not in ('1', 'load'):
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
        popup_handles = []
        task = None
        task_kind = 'iframe'
        task_uuid = None
        for handle in handles[:4]:
            driver.send_session_command('POST', 'window', {'handle': handle}, timeout=2)
            kind = driver.send_session_command('POST', 'execute/sync', {'script':
                "return location.pathname === '/FileAPI/BlobURL/cross-partition.https.html' ? 'root' : location.pathname === '/common/dispatcher/executor.html' ? 'popup' : 'other';",
                'args': []}, timeout=2)
            if kind == 'root':
                root = handle
                task_uuid = str(uuid.uuid4())
                task = driver.send_session_command('POST', 'execute/sync', {'script':
                    "return typeof create_dedicated_worker_js === 'function' ? {kind:'worker',source:create_dedicated_worker_js(location.origin,arguments[0])} : typeof add_iframe_js === 'function' ? add_iframe_js(location.origin, arguments[0]) : null;",
                    'args': [task_uuid]}, timeout=2)
                if isinstance(task, dict) and task.get('kind') == 'worker':
                    task_kind = 'worker'
                    task = task.get('source')
            elif kind == 'popup':
                popup_handles.append(handle)
                if popup is None:
                    popup = handle
        if root is None or popup is None or not isinstance(task, str) or len(task) > 16384:
            observations.append({'target_available': False})
        else:
            if task_kind == 'worker':
                driver.send_session_command('POST', 'window', {'handle': root}, timeout=2)
                probe = driver.send_session_command('POST', 'execute/async', {
                    'script': "const done=arguments[arguments.length-1];let sent=false;const finish=value=>{if(!sent){sent=true;done({limiter_available:value})}};setTimeout(()=>finish(false),500);limiter(async()=>true).then(()=>finish(true),()=>finish(false));",
                    'args': []}, timeout=2)
                if isinstance(probe, dict) and type(probe.get('limiter_available')) is bool:
                    observations.append({'root_limiter_available': probe['limiter_available']})
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
            if task_kind == 'worker' and passive.get('frame_count', 0) < 1:
                observations.append({'worker_target_available': False})
                return
            if mode == 'load':
                # Causal probe only AFTER the recorded original result. A
                # synthetic load must never repair or qualify an original.
                try:
                    driver.send_session_command('POST', 'frame', {'id': 0}, timeout=2)
                    value = driver.send_session_command('POST', 'execute/async', {
                        'script': "const done=arguments[arguments.length-1],scripts=document.querySelectorAll('script'),last=scripts[scripts.length-1];const handler_present=typeof last?.onload==='function';if(handler_present)last.dispatchEvent(new Event('load'));setTimeout(()=>done({handler_present,script_count:document.querySelectorAll('script').length,send_queue_count:typeof sendQueues==='undefined'?0:sendQueues.size}),1000);",
                        'args': []}, timeout=3)
                finally:
                    driver.send_session_command('POST', 'frame', {'id': None}, timeout=2)
                safe = {'synthetic_load_probe': True}
                if isinstance(value, dict):
                    if type(value.get('handler_present')) is bool:
                        safe['handler_present'] = value['handler_present']
                    for key in ('script_count', 'send_queue_count'):
                        count = value.get(key)
                        if type(count) in (int, float) and 0 <= count <= 1000000 and int(count) == count:
                            safe[key] = int(count)
                observations.append(safe)
                # Observe whether the original moves on to its second iframe,
                # without introducing a fresh task or receiving its queue.
                for handle in popup_handles:
                    if handle == popup:
                        continue
                    driver.send_session_command('POST', 'window', {'handle': handle}, timeout=2)
                    try:
                        driver.send_session_command('POST', 'frame', {'id': 0}, timeout=2)
                        value = driver.send_session_command('POST', 'execute/sync', {
                            'script': "return {script_count:document.querySelectorAll('script').length};", 'args': []}, timeout=2)
                        count = value.get('script_count') if isinstance(value, dict) else None
                        if type(count) in (int, float) and 0 <= count <= 1000000 and int(count) == count:
                            observations.append({'other_child_after_load': True, 'script_count': int(count)})
                    finally:
                        driver.send_session_command('POST', 'frame', {'id': None}, timeout=2)
                return
            # This intentionally executes the original parent-generated command
            # only after the original result. Never turn its outcome into PASS.
            try:
                if task_kind == 'worker':
                    driver.send_session_command('POST', 'frame', {'id': 0}, timeout=2)
                value = driver.send_session_command('POST', 'execute/async', {'script':
                    "const task=arguments[0],done=arguments[arguments.length-1];Promise.resolve().then(()=>eval('(async()=>{'+task+'})()')).then(()=>done({settled:true,failed:false,script_count:document.querySelectorAll('script').length,frame_count:frames.length}),error=>done({settled:true,failed:true,exception_type:error?.name,body_present:document.body!==null,script_count:document.querySelectorAll('script').length,frame_count:frames.length}));",
                    'args': [task]}, timeout=20)
            finally:
                if task_kind == 'worker':
                    driver.send_session_command('POST', 'frame', {'id': None}, timeout=2)
            safe = {'target_available': True, 'worker_creation': task_kind == 'worker'}
            if isinstance(value, dict):
                for key in ('settled', 'failed', 'body_present'):
                    if type(value.get(key)) is bool:
                        safe[key] = value[key]
                for key in ('script_count', 'frame_count'):
                    count = value.get(key)
                    if type(count) in (int, float) and 0 <= count <= 1000000 and int(count) == count:
                        safe[key] = int(count)
                if value.get('exception_type') in ('Error', 'TypeError', 'SyntaxError', 'ReferenceError', 'SecurityError', 'NetworkError', 'AbortError', 'InvalidStateError', 'QuotaExceededError', 'NotSupportedError'):
                    safe['exception_type'] = value['exception_type']
            observations.append(safe)
            if task_kind == 'worker' and safe.get('settled') and safe.get('failed') is False:
                driver.send_session_command('POST', 'window', {'handle': root}, timeout=2)
                reply = driver.send_session_command('POST', 'execute/async', {
                    'script': "const done=arguments[arguments.length-1];let sent=false;const finish=value=>{if(!sent){sent=true;done({reply_received:value})}};setTimeout(()=>finish(false),5000);receive(arguments[0]).then(value=>finish(typeof value==='string'&&value.length>0),()=>finish(false));",
                    'args': [task_uuid]}, timeout=8)
                if isinstance(reply, dict) and type(reply.get('reply_received')) is bool:
                    observations.append({'root_reply_received': reply['reply_received']})
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
