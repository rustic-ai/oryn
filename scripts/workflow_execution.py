"""Durable full-contract scheduling, independent of native family adapters.

Adapters own the actual OIL/fixture/native context operations and return pinned
artifacts consumed by workflow_evidence. Scheduling a callback is never evidence
of native context concurrency. Missing adapters and failed attempts stay visible.
"""
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import threading

from workflow_contract import planned_attempts
from workflow_evidence import reconcile


class ExecutionJournal:
    """Exclusive new evidence directory; fsynced append-only attempt records."""
    def __init__(self, directory, plan, candidate):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=False)
        self.lock = threading.Lock()
        encoded = (json.dumps(dict(plan_schema_version=2, candidate=candidate,
                                   attempts=plan), sort_keys=True, indent=2)+'\n').encode()
        with (self.directory/'plan.json').open('xb') as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        self.plan_sha256 = hashlib.sha256(encoded).hexdigest()
        self.identities = set()
        self.records = []
        self.stream = (self.directory/'attempts.jsonl').open('x', encoding='utf-8')

    def append(self, record):
        with self.lock:
            identity = record['attempt_id']
            if not isinstance(identity, str) or not identity:
                raise ValueError('invalid attempt identity')
            if identity in self.identities:
                raise ValueError('attempt already persisted: '+identity)
            encoded = json.dumps(record, sort_keys=True)+'\n'
            # Reconciliation must see precisely the durable JSON observation,
            # not mutable nested dictionaries still owned by an adapter.
            persisted = json.loads(encoded)
            self.stream.write(encoded)
            self.stream.flush()
            os.fsync(self.stream.fileno())
            self.identities.add(identity)
            self.records.append(persisted)

    def close(self):
        self.stream.close()


def execute_contract(contract, candidate, adapters, output, oracles, *, workers=5,
                     stop=None, candidate_probe=None):
    """Execute all frozen attempts, without silently substituting narrower cases.

    adapters maps each complete behavior-cell ID to an adapter callable accepting
    (attempt, artifact_directory, stop_event). It must implement every requested
    mode/context count/fault or return an explicit failed result. Returned action,
    oracle, native context activity and fault artifacts use paths relative to the
    output directory. Per-cell adapter/corpus identities must be frozen by callers.

    A callable returning a successful result without required evidence still
    fails reconciliation. This entry point does not manufacture per-cell adapters
    from controlled-case names; the remaining unimplemented family cells need real
    application tasks and runtime instrumentation before acceptance.
    """
    if type(workers) is not int or not 1 <= workers <= 20:
        raise ValueError('worker count must be 1..20')
    # Adapters and callers can retain references to the supplied dictionaries.
    # Freeze the complete JSON inputs before planning/persisting, so neither
    # the denominator nor the independent oracle can change during execution.
    contract = json.loads(json.dumps(contract))
    candidate = json.loads(json.dumps(candidate))
    oracles = json.loads(json.dumps(oracles))
    stop = stop if stop is not None else threading.Event()
    plan = planned_attempts(contract)
    journal = ExecutionJournal(output, plan, candidate)
    infrastructure = []

    def perform(attempt):
        record = dict(attempt, **candidate, passed=False, timed_out=False, errors=[])
        artifact_name = hashlib.sha256(attempt['attempt_id'].encode()).hexdigest()
        directory = journal.directory/'artifacts'/artifact_name
        directory.mkdir(parents=True, exist_ok=False)
        if stop.is_set():
            record['errors'] = ['not executed: cancellation requested']
        elif attempt['cell'] not in adapters:
            record['errors'] = ['not executed: complete behavior adapter missing']
        else:
            try:
                if candidate_probe is not None:
                    try:
                        if candidate_probe() != candidate:
                            raise ValueError('candidate differs before attempt')
                    except Exception:
                        # Once custody is lost, restoring the bytes cannot
                        # readmit later attempts under this execution receipt.
                        stop.set()
                        raise
                if stop.is_set():
                    raise ValueError('cancellation requested before adapter admission')
                result = adapters[attempt['cell']](dict(attempt), directory, stop)
                if not isinstance(result, dict):
                    raise ValueError('malformed adapter result')
                if any(key in result and result[key] != value for key, value in {**attempt, **candidate}.items()):
                    raise ValueError('adapter changed attempt or candidate identity')
                if ('errors' in result and (not isinstance(result['errors'], list)
                        or any(not isinstance(error, str) for error in result['errors']))):
                    raise ValueError('malformed adapter errors')
                for flag in ('passed', 'timed_out'):
                    if flag in result and type(result[flag]) is not bool:
                        raise ValueError('malformed adapter ' + flag)
                # The controller, not the adapter, owns identity and the plan.
                record.update({key: value for key, value in result.items() if key not in attempt and key not in candidate})
            except Exception as error:
                record['passed'] = False
                record['errors'] = ['adapter failure: '+str(error)]
        # Capture drift after this attempt even if later attempts restore bytes.
        # Preserve native outcomes, but an attempt with lost custody cannot pass.
        if candidate_probe is not None:
            try:
                if candidate_probe() != candidate:
                    raise ValueError('candidate differs after attempt')
            except Exception as error:
                record['passed'] = False
                record.setdefault('errors', []).append('post-attempt custody: ' + str(error))
                stop.set()
        journal.append(record)
        return record

    try:
        # Oracle publication is part of execution admission. A failure must
        # close the journal and preserve every unexecuted obligation, just as
        # a scheduler failure does; no adapter can run without frozen oracles.
        with (journal.directory/"expected-oracles.json").open("x") as frozen_oracles:
            json.dump(oracles,frozen_oracles,sort_keys=True,indent=2)
            frozen_oracles.write("\n")
            frozen_oracles.flush()
            os.fsync(frozen_oracles.fileno())
        if candidate_probe is not None and candidate_probe() != candidate:
            raise ValueError('candidate differs before execution')
        # Each phase is explicit; mixed scheduling cannot replace serial/recovery
        # repetitions or fault injections. Native activity is measured by adapters.
        for mode in ('serial', 'mixed_family', 'recovery_repeat', 'fault_injection'):
            selected = [attempt for attempt in plan if attempt['mode'] == mode]
            if mode == 'mixed_family':
                with ThreadPoolExecutor(max_workers=workers) as pool:
                    futures = []
                    try:
                        for attempt in selected:
                            futures.append(pool.submit(perform, attempt))
                        for future in as_completed(futures):
                            future.result()
                    except BaseException:
                        # Signal active adapters before executor shutdown waits
                        # for them. Otherwise a failed journal/worker can leave
                        # peers waiting for cancellation that arrives too late.
                        stop.set()
                        for future in futures:
                            future.cancel()
                        raise
            else:
                for attempt in selected:
                    perform(attempt)
        if candidate_probe is not None and candidate_probe() != candidate:
            raise ValueError('candidate changed during execution')
    except BaseException as error:
        stop.set()
        infrastructure.append(str(error))
        # Interrupted infrastructure does not erase the denominator. Preserve a
        # result for every not-started attempt before handing control back.
        for attempt in plan:
            if attempt['attempt_id'] not in journal.identities:
                journal.append(dict(attempt, **candidate, passed=False, timed_out=False,
                                    errors=['not executed: infrastructure interrupted']))
        if not isinstance(error, Exception):
            raise
    finally:
        journal.close()
    summary = reconcile(contract, candidate, journal.records, journal.directory, oracles)
    if 'candidate_manifest_sha256' in candidate:
        summary['candidate_manifest_sha256'] = candidate['candidate_manifest_sha256']
    summary.update(scope='full_contract_execution', plan_sha256=journal.plan_sha256,
                   infrastructure_errors=infrastructure, family_qualified=False,
                   goal_complete=False)
    # Structural validation is only one input to signed native qualification.
    if infrastructure:
        summary['structurally_valid'] = False
    with (journal.directory/'summary.json').open('x') as stream:
        json.dump(summary, stream, indent=2)
        stream.write('\n')
    return summary
