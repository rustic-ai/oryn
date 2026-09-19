#!/usr/bin/env python3
"""Run existing prebuilt native performance/churn executors without rebuilding.

Every attempt is retained. Missing or failed observations cannot be replaced by
retries. This profile pass does not certify Core, framework or workflow coverage.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from generation_gate import generation_gate, GenerationLease, inherited_generation_lease
from completion_provenance import load_candidate, workflow_identity
from completion_execution import reference
from workflow_qualification import planned_profiles,validate_profiles
from workflow_resources import ProcessSampler, reconcile_resources


def digest(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def persist(path,value):
    with path.open('x',encoding='utf-8') as stream:
        json.dump(value,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())


def cleanup_process_group(pid):
    """Retain a survivor failure even if it exits before forced cleanup."""
    try:
        os.killpg(pid, 0)
    except ProcessLookupError:
        return True
    except PermissionError:
        return False
    # Observation already proves that direct-child exit did not reap its group.
    # A subsequent exit race must not rewrite that observation as clean reap.
    try:
        os.killpg(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass
    return False


def run_profile_process(command, env, directory, admission=None):
    admission = admission or inherited_generation_lease()
    admission.check()
    process = sampler = None
    code = None
    timed_out = cleanup = False
    errors = []
    resources = dict(schema_version=1, samples=[], errors=['sampler did not start'])
    try:
        with (directory/'stdout.json').open('xb') as stdout, (directory/'stderr.txt').open('xb') as stderr:
            try:
                admission.check()
                process = subprocess.Popen(command, stdout=stdout, stderr=stderr,
                                           env=env, start_new_session=True)
                sampler = ProcessSampler(process.pid)
                sampler.start()
                try:
                    code = process.wait(timeout=300)
                except subprocess.TimeoutExpired:
                    timed_out = True
            finally:
                # A sampler-start or wait failure still owns a live child.
                # Complete cleanup independently of output parsing/accounting.
                if process is not None:
                    try:
                        if process.poll() is None:
                            try:
                                os.killpg(process.pid, signal.SIGKILL)
                            except ProcessLookupError:
                                pass
                            code = process.wait(timeout=15)
                        else:
                            code = process.returncode
                        cleanup = cleanup_process_group(process.pid)
                    except Exception as error:
                        errors.append('process cleanup: ' + str(error))
                if sampler is not None:
                    try:
                        resources = sampler.finish()
                    except Exception as error:
                        errors.append('resource sampler cleanup: ' + str(error))
    except Exception as error:
        errors.append('profile execution: ' + str(error))
    if resources.get('errors') or resources.get('overflow') or not resources.get('samples'):
        errors.append('OS resource sampling incomplete')
    return dict(returncode=code, timed_out=timed_out, cleanup_complete=cleanup,
                parent_pid=process.pid if process is not None else None,
                resources=resources, errors=errors)


def read_profile_result(directory, resources, parent_pid, worker_digest):
    """Malformed native output remains a failed observation, never a lost row."""
    errors = []
    result = {}
    try:
        result = json.loads((directory/'stdout.json').read_text())
        if not isinstance(result, dict):
            raise ValueError('native result is not an object')
    except (OSError, ValueError, UnicodeError) as error:
        return {}, ['native output: ' + str(error)]
    runtime = result.get('runtime')
    if not isinstance(runtime, dict) or runtime.get('worker_binary_sha256') != worker_digest:
        errors.append('native worker identity differs from pinned worker')
    if result.get('process_lifecycle') is not None:
        try:
            lifecycle = result['process_lifecycle']
            if not isinstance(lifecycle, dict) or not isinstance(lifecycle.get('processes'), list):
                raise ValueError('malformed process lifecycle')
            contexts = set()
            for entry in lifecycle['processes']:
                if not isinstance(entry, dict) or not isinstance(entry.get('leases', []), list):
                    raise ValueError('malformed process or lease population')
                for lease in entry.get('leases', []):
                    if not isinstance(lease, dict) or not isinstance(lease.get('context_id'), (str, int)):
                        raise ValueError('malformed context lease')
                    if entry.get('role') != 'verification':
                        contexts.add(lease['context_id'])
            accounting = reconcile_resources(resources, dict(
                parent_pid=parent_pid, runtime=runtime if isinstance(runtime, dict) else {},
                worker_lifecycle=lifecycle,
                sessions=[dict(context_id=context) for context in contexts]))
            persist(directory/'resource-accounting.json', accounting)
            if accounting.get('complete') is not True:
                errors.append('native resource accounting incomplete')
        except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
            errors.append('native resource accounting: ' + str(error))
    return result, errors


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generation-gate',type=Path,required=True)
    for name in ('page-benchmark','group-benchmark','churn','worker','candidate','output'):parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();admission = GenerationLease(args.generation_gate, validator=generation_gate)
    __import__('os').environ.update(admission.environment())
    # Freeze the declaration that admitted setup, before candidate inspection
    # can observe or trigger a replacement with another valid declaration.
    gate_identity=reference(args.generation_gate)
    binaries=dict(page=args.page_benchmark.resolve(),group=args.group_benchmark.resolve(),churn=args.churn.resolve(),worker=args.worker.resolve())
    bound=load_candidate(args.candidate)
    source=subprocess.check_output([str(Path(__file__).resolve().parents[1]/'scripts/source-tree-hash.sh')],text=True).strip()
    candidate=workflow_identity(bound,bound['artifacts']['workflow_executor'],args.worker,source)
    identity={name:digest(path) for name,path in binaries.items()}
    names=dict(page='page_benchmark',group='group_benchmark',churn='churn',worker='worker')
    for name,path in binaries.items():
        if path!=bound['artifacts'][names[name]] or identity[name]!=bound['record']['artifacts'][names[name]]['sha256']:
            raise ValueError('benchmark differs from combined candidate: '+name)
    custody_errors=[]
    def probe():
        admission.check()
        generation_gate(args.generation_gate)
        if digest(args.generation_gate)!=gate_identity['sha256']:
            raise ValueError('profile generation declaration changed')
        if load_candidate(args.candidate)['sha256']!=bound['sha256']:
            raise ValueError('combined candidate changed during profiles')
        current_source=subprocess.check_output([str(Path(__file__).resolve().parents[1]/'scripts/source-tree-hash.sh')],text=True).strip()
        if current_source!=source or {name:digest(path) for name,path in binaries.items()}!=identity:
            raise ValueError('profile source or executable changed')
    # Candidate loading and source inspection may outlive the declaration that
    # admitted them. Recheck before the first evidence-directory side effect.
    probe()
    args.output.mkdir(parents=True,exist_ok=False)
    # Exact benchmark executable identities are additional candidate inputs, not
    # inferred from a Git commit or an unchanged filename.
    persist(args.output/'candidate-inputs.json',dict(candidate=candidate,executables=identity,candidate_file_sha256=digest(args.candidate)))
    plan=planned_profiles();persist(args.output/'plan.json',plan);records=[]
    admission_error=None
    for item in plan:
        if admission_error is None:
            try:probe()
            except Exception as error:
                admission_error=str(error);custody_errors.append(admission_error)
        directory=args.output/item['id'].replace(':','-');directory.mkdir()
        if admission_error is not None:
            record=dict(id=item['id'],returncode=None,timed_out=False,cleanup_complete=False,
                        errors=['not executed: '+admission_error],result={},artifacts={})
            persist(directory/'attempt.json',record);records.append(record)
            continue
        command=[str(binaries[item['kind']])]
        if item['kind']=='page':command += [str(item['pages']),str(item['actions'])]
        elif item['kind']=='group':command += [str(item['pages'])]
        env=dict(os.environ,ORYN_PAGE_WORKER=str(binaries['worker']))
        env.pop('ORYN_BENCH_WORKER_MODE',None)
        if item['kind']=='page' and item['mode']!='isolated':env['ORYN_BENCH_WORKER_MODE']=item['mode']
        started=time.monotonic_ns()
        execution=run_profile_process(command,env,directory,admission)
        resources=execution['resources']
        persist(directory/'resource-samples.json',resources)
        result,errors=read_profile_result(directory,resources,execution['parent_pid'],identity['worker'])
        errors=execution['errors']+errors
        record=dict(id=item['id'],returncode=execution['returncode'],timed_out=execution['timed_out'],cleanup_complete=execution['cleanup_complete'],errors=errors,elapsed_ns=time.monotonic_ns()-started,result=result,artifacts={name:dict(path=str((directory/name).relative_to(args.output)),sha256=digest(directory/name)) for name in ('stdout.json','stderr.txt','resource-samples.json','resource-accounting.json') if (directory/name).exists()})
        persist(directory/'attempt.json',record);records.append(record)
        try:probe()
        except Exception as error:
            custody_errors.append(str(error))
            admission_error=str(error)
            persist(args.output/'candidate-drift.json',dict(after_attempt=item['id'],error=str(error)))
    try:probe()
    except Exception as error:custody_errors.append(str(error))
    persist(args.output/'execution-custody.json',dict(schema_version=1,candidate_manifest_sha256=bound['sha256'],generation_gate=gate_identity,producer_source=reference(Path(__file__)),custody_complete=not custody_errors,errors=custody_errors,attempts={str((args.output/item['id'].replace(':','-')/'attempt.json').relative_to(args.output)):digest(args.output/item['id'].replace(':','-')/'attempt.json') for item in records}))
    summary=validate_profiles(records);summary['errors'].extend(custody_errors)
    if custody_errors:summary['profile_checks_passed']=False
    summary['candidate_manifest_sha256']=bound['sha256'];persist(args.output/'summary.json',summary)
    print(json.dumps(summary));return int(not summary['profile_checks_passed'])

if __name__=='__main__':raise SystemExit(main())
