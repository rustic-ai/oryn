"""One-shot response withholding controlled only by the fixture's stdin oracle.

The HTTP handler records actual entry/release times in SQLite. Configuring a
fault without a matching request never produces an injected event.
"""
import json
import os
from pathlib import Path
from contextlib import closing
import sqlite3
import threading
import time


class ResponseFaults:
    def __init__(self, database):
        self.database = str(database)
        self.lock = threading.Lock()
        self.pending = {}
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('CREATE TABLE IF NOT EXISTS workflow_faults(case_id TEXT, token TEXT UNIQUE, route TEXT, delay_ms INTEGER, started_ns INTEGER, released_ns INTEGER, terminal TEXT, expected_status INTEGER, skip_matches INTEGER)')

    def arm(self, case_id, token, route, delay_ms, expected_status=None, skip_matches=0, response_gate=None):
        if not isinstance(token, str) or not token or not isinstance(route, str) or not route:
            raise ValueError('fault requires token and exact route')
        if type(delay_ms) is not int or not 1 <= delay_ms <= 30000:
            raise ValueError('fault delay must be 1..30000 ms')
        if expected_status is not None and (type(expected_status) is not int or not 100<=expected_status<=599):raise ValueError("invalid response status gate")
        if type(skip_matches) is not int or not 0<=skip_matches<=128:raise ValueError("invalid response match gate")
        if response_gate is not None:
            if (not isinstance(response_gate, dict) or set(response_gate) != {'token', 'marker', 'route'}
                    or response_gate.get('token') != token or response_gate.get('route') != route
                    or not isinstance(response_gate.get('marker'), str)):
                raise ValueError('response gate identity differs from armed fault')
            marker = Path(response_gate['marker'])
            if not marker.is_absolute() or marker.resolve() != marker or marker.exists():
                raise ValueError('response gate requires a fresh canonical absolute marker')
            response_gate = dict(response_gate)
        with self.lock, closing(sqlite3.connect(self.database)) as db, db:
            if case_id in self.pending:
                raise ValueError('case already has an armed response fault')
            db.execute('INSERT INTO workflow_faults VALUES(?,?,?,?,NULL,NULL,?,?,?)',
                       (case_id, token, route, delay_ms, 'armed',expected_status,skip_matches))
            self.pending[case_id] = [token, route, delay_ms, threading.Event(),expected_status,skip_matches,response_gate]

    def before_response(self, case_id, route, status=None):
        with self.lock:
            pending = self.pending.get(case_id)
            if pending is None or pending[1] != route:
                return
            if pending[4] is not None and pending[4]!=status:return
            if pending[5]>0:
                pending[5]-=1
                return
            del self.pending[case_id]
        token, _, delay_ms, release, _, _, gate = pending
        started = time.monotonic_ns()
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('UPDATE workflow_faults SET started_ns=?,terminal=? WHERE token=?',
                       (started, 'withholding', token))
        if gate is not None:
            marker = Path(gate['marker'])
            temporary = marker.with_name(marker.name + '.publishing')
            with temporary.open('x', encoding='utf-8') as stream:
                json.dump(dict(schema_version=1, token=token, route=route,
                               phase='response_entered', started_ns=started), stream)
                stream.flush(); os.fsync(stream.fileno())
            if marker.exists():
                raise ValueError('response marker unexpectedly exists')
            temporary.rename(marker)
            directory = os.open(marker.parent, os.O_RDONLY)
            try: os.fsync(directory)
            finally: os.close(directory)
        # This is actual HTTP response withholding, outside application DB locks.
        release.wait(delay_ms / 1000)
        ended = time.monotonic_ns()
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('UPDATE workflow_faults SET released_ns=?,terminal=? WHERE token=?',
                       (ended, 'released', token))

    def snapshot(self, case_id):
        with closing(sqlite3.connect(f'file:{self.database}?mode=ro', uri=True)) as db, db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('SELECT * FROM workflow_faults WHERE case_id=? ORDER BY rowid', (case_id,))]


class CommitGates:
    """Private, one-shot commit barriers; configuration is never HTTP accessible.

    Marker publication follows a committed ledger entry. A before-commit gate
    always aborts its original request, including on timeout; it cannot race a
    recovery request into a late durable write. After-commit never rolls back.
    """
    def __init__(self, database):
        self.database = str(database)
        self.lock = threading.Lock()
        self.pending = {}
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('CREATE TABLE IF NOT EXISTS workflow_commit_gates(case_id TEXT, token TEXT UNIQUE, route TEXT, phase TEXT, body_sha256 TEXT, body BLOB, cookie TEXT, content_type TEXT, method TEXT, started_ns INTEGER, released_ns INTEGER, terminal TEXT, response_status INTEGER)')

    def arm(self, case_id, token, route, phase, marker, release, skip_matches=0, timeout_ms=30000):
        from pathlib import Path
        if not token or not route or phase not in ('before_commit', 'after_commit'):
            raise ValueError('invalid commit gate identity or phase')
        if type(skip_matches) is not int or not 0 <= skip_matches <= 128:
            raise ValueError('invalid commit gate ordinal')
        if type(timeout_ms) is not int or not 1 <= timeout_ms <= 30000:
            raise ValueError('invalid commit gate deadline')
        marker, release = Path(marker), Path(release)
        if not marker.is_absolute() or not release.is_absolute() or marker == release:
            raise ValueError('commit gate requires distinct absolute marker paths')
        if marker.exists() or release.exists():
            raise ValueError('commit gate paths must be fresh')
        with self.lock, closing(sqlite3.connect(self.database)) as db, db:
            if case_id in self.pending:
                raise ValueError('case already has an armed commit gate')
            db.execute('INSERT INTO workflow_commit_gates(case_id,token,route,phase,terminal) VALUES(?,?,?,?,?)',
                       (case_id, token, route, phase, 'armed'))
            self.pending[case_id] = dict(token=token, route=route, phase=phase, marker=marker,
                                        release=release, skip=skip_matches, timeout_ms=timeout_ms)

    def claim(self, case_id, route, body, cookie='', content_type='application/x-www-form-urlencoded', method='POST'):
        import hashlib
        with self.lock:
            gate = self.pending.get(case_id)
            if gate is None or gate['route'] != route:
                return None
            if gate['skip']:
                gate['skip'] -= 1
                return None
            del self.pending[case_id]
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('UPDATE workflow_commit_gates SET body_sha256=?,body=?,cookie=?,content_type=?,method=?,terminal=? WHERE token=?',
                       (hashlib.sha256(body).hexdigest(), body, cookie, content_type, method, 'claimed', gate['token']))
        return gate

    def hold(self, gate, status=None):
        import os
        started = time.monotonic_ns()
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('UPDATE workflow_commit_gates SET started_ns=?,response_status=?,terminal=? WHERE token=?',
                       (started, status, 'withholding', gate['token']))
        marker = dict(schema_version=1, token=gate['token'], phase=gate['phase'], started_ns=started)
        # Atomic rename prevents the native reader seeing a partial JSON object.
        temporary = gate['marker'].with_name(gate['marker'].name + '.publishing')
        with temporary.open('x', encoding='utf-8') as stream:
            json.dump(marker, stream); stream.flush(); os.fsync(stream.fileno())
        if gate['marker'].exists():
            raise ValueError('commit marker unexpectedly exists')
        temporary.rename(gate['marker'])
        deadline = time.monotonic() + gate['timeout_ms'] / 1000
        terminal = 'deadline_aborted' if gate['phase'] == 'before_commit' else 'deadline_response_lost'
        while time.monotonic() < deadline:
            try:
                with gate['release'].open(encoding='utf-8') as stream:
                    release = json.load(stream)
                if release.get('schema_version') != 1 or release.get('token') != gate['token'] or release.get('phase') != gate['phase'] or release.get('worker_reaped') is not True or release.get('interrupted_inflight') is not True:
                    terminal = 'invalid_release'
                    break
                terminal = 'aborted_before_commit' if gate['phase'] == 'before_commit' else 'released_after_commit'
                break
            except FileNotFoundError:
                time.sleep(0.005)
            except (ValueError, OSError):
                terminal = 'invalid_release'
                break
        with closing(sqlite3.connect(self.database)) as db, db:
            db.execute('UPDATE workflow_commit_gates SET released_ns=?,terminal=? WHERE token=?',
                       (time.monotonic_ns(), terminal, gate['token']))
        done = gate['marker'].with_name(gate['marker'].name + '.done')
        with done.open('x', encoding='utf-8') as stream:
            json.dump(dict(token=gate['token'],phase=gate['phase'],terminal=terminal),stream)
            stream.flush();os.fsync(stream.fileno())
        return terminal

    def snapshot(self, case_id):
        # Original request bytes and session cookies are private recovery material,
        # never part of public evidence or logs.
        with closing(sqlite3.connect(f'file:{self.database}?mode=ro', uri=True)) as db, db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('SELECT case_id,token,route,phase,body_sha256,method,started_ns,released_ns,terminal,response_status FROM workflow_commit_gates WHERE case_id=? ORDER BY rowid', (case_id,))]
