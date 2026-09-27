"""Join real upstream results with native shared-backing process diagnostics."""
from urllib.parse import urlsplit

TEST = '/__oryn_shared_memory__/shared-memory/engine-lifecycle.https.html?placement=1'
NAMES = {
    'A worker waiter can be notified through another mapping and is removed exactly once',
    'A remote notify settles the originating agent asynchronous waiter',
    'Concurrent shared growth publishes length and zeroed bytes in every mapping',
    'Shared Wasm growth returns distinct previous sizes and refreshes remote memory bounds',
}


def digest(value):
    return isinstance(value, str) and len(value) == 64 and all(c in '0123456789abcdef' for c in value)


def indexed(rows):
    result = {}
    for row in rows:
        identifier = row.get('id')
        if type(identifier) is not int or identifier <= 0 or identifier in result:
            raise ValueError('invalid or duplicate native principal')
        result[identifier] = row
    return result


def placement(row, binary, parent):
    item = row.get('placement')
    if not isinstance(item, dict):
        raise ValueError('native process placement missing or busy')
    pid = item.get('process_id')
    if (item.get('mode') != 'child_process' or item.get('attested') is not True
            or item.get('binary_sha256') != binary or type(pid) is not int or pid <= 0 or pid == parent):
        raise ValueError('placement does not attest the candidate child process')
    return pid


def qualify(record, report, context, binary):
    if not digest(binary) or not digest(context.get('nonce')):
        raise ValueError('missing candidate or run nonce')
    for key in ('nonce', 'candidate_sha256', 'run_contract_sha256', 'test'):
        if record.get(key) != context.get(key):
            raise ValueError('placement run binding changed: ' + key)
    if record.get('schema') != 1 or record.get('error') or record.get('test') != TEST:
        raise ValueError('native placement capture failed')
    info = report.get('run_info', {})
    for key in ('candidate_sha256', 'run_contract_sha256'):
        if info.get('oryn_' + key) != context[key]:
            raise ValueError('upstream report candidate differs')
    if (info.get('oryn_executor') != 'upstream-wptrunner'
            or info.get('oryn_wpt_revision') != '719d5e38fdd0903a18ed9007aba816c98cc491e0'):
        raise ValueError('upstream executor evidence required')
    results = report.get('results', [])
    if len(results) != 1 or results[0].get('test') != TEST or results[0].get('status') != 'OK':
        raise ValueError('placement fixture did not complete through upstream')
    subtests = results[0].get('subtests', [])
    if (len(subtests) != len(NAMES) or {item.get('name') for item in subtests} != NAMES
            or any(item.get('status') != 'PASS' for item in subtests)):
        raise ValueError('shared-memory conformance assertions did not all pass')
    snapshot = record.get('snapshot', {})
    if snapshot.get('schema') != 1 or snapshot.get('nonce') != context['nonce'] or not digest(snapshot.get('capture_id')):
        raise ValueError('native snapshot identity missing')
    parent = snapshot.get('parent_pid')
    if type(parent) is not int or parent <= 0:
        raise ValueError('native parent placement missing')
    pages, agents = indexed(snapshot.get('pages', [])), indexed(snapshot.get('agents', []))
    pages = {key: value for key, value in pages.items()
             if urlsplit(value.get('url', '')).path + '?' + urlsplit(value.get('url', '')).query == TEST}
    if len(pages) != 1:
        raise ValueError('exact fixture browsing context missing or ambiguous')
    page_id, page = next(iter(pages.items()))
    page_pid = placement(page, binary, parent)
    page_url = urlsplit(page['url'])
    eligible = {}
    for identifier, agent in agents.items():
        url = urlsplit(agent.get('script_url', ''))
        if (agent.get('kind') == 'Dedicated' and agent.get('live') is True
                and (url.scheme, url.netloc) == (page_url.scheme, page_url.netloc)
                and url.path == '/__oryn_shared_memory__/shared-memory/worker.js' and url.query == 'placement=1'
                and any(owner.get('kind') == 'window' and owner.get('id') == page_id
                        and owner.get('frame') is None for owner in agent.get('owners', []))):
            pid = placement(agent, binary, parent)
            if not digest(agent['placement'].get('bootstrap_sha256')) or pid == page_pid:
                raise ValueError('worker bootstrap or physical process separation missing')
            eligible[identifier] = pid
    witnesses, seen = {}, set()
    for backing in snapshot.get('shared_backings', []):
        identity = backing.get('backing_sha256')
        if not digest(identity) or identity in seen:
            raise ValueError('invalid or duplicate native backing identity')
        seen.add(identity)
        if any(type(backing.get(key)) is not bool for key in ('wasm', 'growable', 'memory64')):
            raise ValueError('native backing kind metadata missing')
        holders = backing.get('holders', [])
        if not any(owner.get('kind') == 'window' and owner.get('id') == page_id
                   and owner.get('frame') is None for owner in holders):
            continue
        peers = {owner.get('id') for owner in holders if owner.get('kind') == 'agent' and owner.get('id') in eligible}
        if not peers:
            continue
        kind = 'wasm' if backing.get('wasm') is True else 'growable' if backing.get('growable') is True else 'fixed'
        required = 2 if kind in {'wasm', 'growable'} else 1
        if len({eligible[peer] for peer in peers}) >= required:
            witnesses.setdefault(kind, {'backing_sha256': identity, 'page_pid': page_pid,
                                        'worker_pids': sorted({eligible[peer] for peer in peers})})
    if set(witnesses) != {'fixed', 'growable', 'wasm'}:
        raise ValueError('native cross-process backing witnesses incomplete')
    return {'schema': 1, 'qualification_accepted': True, 'test': TEST,
            'candidate_sha256': context['candidate_sha256'], 'capture_id': snapshot['capture_id'],
            'nonce': context['nonce'], 'witnesses': witnesses}
