// Failure evidence only: no browser commands, retries, or result classification.
export function safeLocation(value) {
  try {
    const url = new URL(value);
    if (url.protocol === 'about:' && ['blank','srcdoc'].includes(url.pathname)) return {scheme:'about:', page:url.pathname};
    if (!['http:','https:'].includes(url.protocol)) return {scheme:url.protocol.slice(0,64), opaque_body_redacted:true};
    return {origin:url.origin.slice(0,512), path:url.pathname.slice(0,2048),
      path_truncated:url.pathname.length>2048, query_present:!!url.search, fragment_present:!!url.hash};
  } catch { return {invalid_url:true}; }
}

const text = value => typeof value === 'string' ? value.slice(0, 256) : undefined;
const number = value => Number.isFinite(value) ? value : undefined;
const errorName = error => ['Error','TypeError','RangeError','SyntaxError','ReferenceError','URIError','EvalError','AbortError','TimeoutError'].includes(error?.name) ? error.name : 'Error';

export function createBrowserStderrCollector({limit=32, byteLimit=65536, clock=() => performance.now()} = {}) {
  if (!Number.isInteger(limit) || limit < 1 || limit > 64 || !Number.isInteger(byteLimit) || byteLimit < 1 || byteLimit > 65536) throw new Error('invalid browser stderr bounds');
  let bytesSeen=0, bytesInspected=0, linesSeen=0, unclassified=0, omitted=0, oversized=0;
  let tail='', discarding=false, ended=false;
  const start=clock();
  const records=[];
  function line(value) {
    linesSeen++;
    const prefix=value.match(/^\[(\d{1,10}):\d{1,10}:[^\]\r\n]{1,80}:(WARNING|ERROR|FATAL):([^\]\r\n]{1,256})\]\s*(.*)$/);
    if (!prefix) { unclassified++; return; }
    const source=prefix[3], message=prefix[4];
    const component=/network_service/i.test(source) ? 'network_service' : /child_process|process_launcher/i.test(source) ? 'child_process' : /sandbox/i.test(source) ? 'sandbox' : 'other';
    const category=/network service.*crash|network_service.*crash/i.test(message) ? 'network_service_crashed'
      : /network service.*restart/i.test(message) ? 'network_service_restarting'
      : /launch.*fail|fail.*launch/i.test(message) ? 'launch_failed'
      : /permission denied|operation not permitted/i.test(message) ? 'permission_denied'
      : /sandbox.*fail|sandbox.*error/i.test(message) ? 'sandbox_failure' : 'unclassified';
    const net=message.match(/\bnet::ERR_[A-Z0-9_]{1,80}\b/);
    const os=message.match(/\b(?:errno|os_error)\s*[=:]\s*(-?\d{1,6})\b/);
    const entry={at_ms:Math.max(0,Math.round(clock()-start)),pid:Number(prefix[1]),severity:prefix[2],component,category};
    if (net) entry.net_error=net[0];
    if (os) entry.os_error=Number(os[1]);
    if (records.length < limit) records.push(entry); else omitted++;
  }
  return {
    write(chunk) {
      if (ended) return;
      const data=Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      bytesSeen+=data.length;
      const selected=data.subarray(0,Math.max(0,byteLimit-bytesInspected));
      bytesInspected+=selected.length;
      const parts=selected.toString('utf8').split('\n');
      for (let index=0;index<parts.length;index++) {
        const part=parts[index];
        if (!discarding && tail.length+part.length <= 4096) tail+=part;
        else { tail=''; discarding=true; }
        if (index < parts.length-1) {
          if (discarding) { linesSeen++; oversized++; } else line(tail);
          tail=''; discarding=false;
        }
      }
      // A byte-cap boundary is not an observed complete stderr line.
      if (selected.length < data.length && tail) { tail=''; discarding=true; }
    },
    end() {
      if (ended) return;
      ended=true;
      if (discarding) oversized++; else if (tail) line(tail);
      tail='';
    },
    snapshot() {
      return {bytes_seen:bytesSeen,bytes_inspected:bytesInspected,bytes_omitted:bytesSeen-bytesInspected,
        lines_seen:linesSeen,unclassified_lines:unclassified,oversized_or_partial_lines:oversized,
        records_omitted:omitted,records:records.map(record=>({...record})),stream_ended:ended};
    },
  };
}

export function createDiagnostics(url, {clock=() => performance.now(), limit=256} = {}) {
  if (!Number.isInteger(limit) || limit < 1 || limit > 1024) throw new Error('invalid diagnostic limit');
  const start = clock();
  const events = [], requests = new Map(), documents = new Set();
  const browserStderr = createBrowserStderrCollector({clock});
  let dropped = 0, documentOverflow = 0, requestOverflow = 0, phase = 'initialization', lastTimeout = null, failure = null;
  const elapsed = () => Math.max(0, Math.round(clock() - start));
  const record = (kind, fields={}) => {
    events.push({at_ms:elapsed(), kind, ...fields});
    if (events.length > limit) { events.shift(); dropped++; }
  };
  return {
    browserStderr(chunk) { browserStderr.write(chunk); },
    browserStderrEnd() { browserStderr.end(); },
    phase(value) { phase = text(value); record('phase', {name:phase}); },
    browser(value, fields={}) {
      record('browser', {event:text(value), pid:number(fields.pid), code:number(fields.code),
        signal:text(fields.signal), error_code:text(fields.error_code)});
    },
    failure(error) {
      if (!failure) { failure = {phase, name:errorName(error)}; record('failure', failure); }
    },
    request(id, method, sessionId) {
      const entry = {id, method:text(method), session_id:text(sessionId), started_ms:elapsed()};
      if (requests.size >= limit) { requests.delete(requests.keys().next().value); requestOverflow++; }
      requests.set(id, entry);
      record('cdp_request', entry);
      return entry;
    },
    response(id, failed=false) {
      const entry = requests.get(id);
      if (!entry) return;
      requests.delete(id);
      record('cdp_response', {...entry, elapsed_ms:elapsed()-entry.started_ms, failed:!!failed});
    },
    timeout(id, timeoutMs, original) {
      const entry = requests.get(id) || (original ? {id, method:text(original.method),
        session_id:text(original.session_id), started_ms:number(original.started_ms)} : null);
      if (!entry) return;
      requests.delete(id);
      lastTimeout = {...entry, phase, elapsed_ms:elapsed()-entry.started_ms, timeout_ms:timeoutMs};
      record('cdp_timeout', lastTimeout);
    },
    event(message) {
      const p = message.params || {}, session_id = text(message.sessionId);
      const documentKey = typeof p.requestId === 'string' && p.requestId.length <= 256 &&
        (message.sessionId === undefined || (typeof message.sessionId === 'string' && message.sessionId.length <= 256))
        ? JSON.stringify([message.sessionId || '', p.requestId]) : null;
      switch (message.method) {
        case 'Target.attachedToTarget':
          record('target_attached', {session_id, child_session_id:text(p.sessionId),
            type:text(p.targetInfo?.type), target_id:text(p.targetInfo?.targetId),
            waiting_for_debugger:!!p.waitingForDebugger, location:safeLocation(p.targetInfo?.url)}); break;
        case 'Target.detachedFromTarget':
          record('target_detached', {session_id, child_session_id:text(p.sessionId), target_id:text(p.targetId)}); break;
        case 'Runtime.executionContextCreated':
          record('context_created', {session_id, context_id:number(p.context?.id),
            unique_id:text(p.context?.uniqueId), frame_id:text(p.context?.auxData?.frameId),
            is_default:!!p.context?.auxData?.isDefault}); break;
        case 'Runtime.executionContextDestroyed':
          record('context_destroyed', {session_id, context_id:number(p.executionContextId)}); break;
        case 'Runtime.executionContextsCleared': record('contexts_cleared', {session_id}); break;
        case 'Page.frameNavigated':
          record('frame_navigated', {session_id, frame_id:text(p.frame?.id), parent_id:text(p.frame?.parentId),
            loader_id:text(p.frame?.loaderId), location:safeLocation(p.frame?.url)}); break;
        case 'Page.frameStartedLoading': case 'Page.frameStoppedLoading':
          record('frame_loading', {session_id, event:message.method, frame_id:text(p.frameId)}); break;
        case 'Network.requestWillBeSent':
          if (p.type !== 'Document') break;
          if (documentKey !== null && documents.size < limit) documents.add(documentKey); else documentOverflow++;
          record('document_request', {session_id, request_id:text(p.requestId), loader_id:text(p.loaderId),
            frame_id:text(p.frameId), method:text(p.request?.method), location:safeLocation(p.request?.url)}); break;
        case 'Network.responseReceived':
          if (p.type !== 'Document') break;
          record('document_response', {session_id, request_id:text(p.requestId), loader_id:text(p.loaderId),
            status:number(p.response?.status), protocol:text(p.response?.protocol)}); break;
        case 'Network.loadingFailed': case 'Network.loadingFinished':
          if (documentKey === null || !documents.has(documentKey)) break;
          documents.delete(documentKey);
          record('document_terminal', {session_id, event:message.method, request_id:text(p.requestId),
            error_text:/^net::ERR_[A-Z0-9_]+$/.test(p.errorText || '') ? p.errorText : undefined, canceled:!!p.canceled, blocked_reason:text(p.blockedReason)}); break;
      }
    },
    serialize(error, stage, budget=65536) {
      if (!Number.isInteger(budget) || budget < 8192 || budget > 65536) throw new Error('invalid diagnostic budget');
      const snapshot = this.snapshot(error);
      snapshot.stage = ['failure','cleanup','uncaught'].includes(stage) ? stage : 'uncaught';
      snapshot.serialization_budget_bytes = budget;
      snapshot.serialization_omitted_events = 0;
      snapshot.serialization_omitted_pending = 0;
      snapshot.serialization_omitted_browser_records = 0;
      let output = JSON.stringify(snapshot);
      while (Buffer.byteLength(output) > budget && snapshot.timeline.length) {
        snapshot.timeline.shift(); snapshot.serialization_omitted_events++; output = JSON.stringify(snapshot);
      }
      while (Buffer.byteLength(output) > budget && snapshot.pending_cdp.length) {
        snapshot.pending_cdp.shift(); snapshot.serialization_omitted_pending++; output = JSON.stringify(snapshot);
      }
      while (Buffer.byteLength(output) > budget && snapshot.browser_stderr.records.length) {
        snapshot.browser_stderr.records.shift(); snapshot.serialization_omitted_browser_records++; output = JSON.stringify(snapshot);
      }
      if (Buffer.byteLength(output) > budget) throw new Error('diagnostic fixed fields exceed budget');
      return output;
    },
    snapshot(error) {
      const pending = [...requests.values()];
      return {schema_version:1, kind:'chromium_reference_failure', phase, requested_location:safeLocation(url),
        elapsed_ms:elapsed(), failure_phase:failure?.phase ?? phase, error:{name:failure?.name ?? errorName(error)}, last_timeout:lastTimeout,
        pending_cdp:pending.slice(0,limit), pending_cdp_omitted:Math.max(0,pending.length-limit),
        browser_stderr:browserStderr.snapshot(),
        timeline:events.slice(), timeline_dropped:dropped, document_tracking_overflow:documentOverflow, request_tracking_overflow:requestOverflow};
    },
  };
}
