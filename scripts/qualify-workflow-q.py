#!/usr/bin/env python3
"""Reconcile resource/performance/workflow Q components from pinned raw evidence."""
import argparse
import json
import os
from pathlib import Path
from workflow_input_lock import for_candidate
from completion_provenance import load_candidate
from workflow_evidence import artifact,reconcile
from workflow_resources import reconcile_resources
from workflow_qualification import planned_profiles,validate_profiles,integrated_q_checks,validate_profile_custody,validate_profile_candidate


def read(path):return json.loads(path.read_text())


def resource_accounting(root,wave):
    stored=json.loads(artifact(root,wave['accounting']))
    samples=json.loads(artifact(root,wave['samples']))
    native=json.loads(artifact(root,wave['native_evidence']))
    computed=reconcile_resources(samples,native)
    if computed!=stored:
        raise ValueError('resource accounting differs from pinned native/OS evidence')
    return computed


def profile_accounting(root,row,raw):
    samples=json.loads(artifact(root,row['artifacts']['resource-samples.json']))
    if samples.get('errors')!=[] or samples.get('overflow') is not False or not samples.get('samples'):
        raise ValueError('profile OS sampling incomplete')
    lifecycle=raw.get('process_lifecycle')
    if lifecycle is None:
        if 'resource-accounting.json' in row['artifacts']:
            raise ValueError('profile accounting has no native lifecycle')
        return
    if not isinstance(lifecycle,dict) or not isinstance(lifecycle.get('processes'),list):
        raise ValueError('malformed profile lifecycle')
    contexts=set()
    for process in lifecycle['processes']:
        for lease in process.get('leases',[]):
            if process.get('role')!='verification':contexts.add(lease['context_id'])
    computed=reconcile_resources(samples,dict(parent_pid=samples.get('parent_pid'),
        runtime=raw.get('runtime'),worker_lifecycle=lifecycle,
        sessions=[dict(context_id=context) for context in contexts]))
    stored=json.loads(artifact(root,row['artifacts']['resource-accounting.json']))
    if stored!=computed or computed.get('complete') is not True:
        raise ValueError('profile accounting differs from raw evidence or is incomplete')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('workflows','profiles','output','candidate'):parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();plan=read(args.workflows/'plan.json')
    records=[json.loads(line) for line in (args.workflows/'attempts.jsonl').read_text().splitlines() if line]
    bound=load_candidate(args.candidate)
    if plan['candidate'].get('candidate_manifest_sha256')!=bound['sha256']:raise ValueError('Q workflow candidate differs')
    workflows=reconcile(for_candidate(bound),plan['candidate'],records,args.workflows,read(args.workflows/'expected-oracles.json'))
    profile_records=[];problems=[]
    inputs=read(args.profiles/'candidate-inputs.json')
    try:validate_profile_custody(args.profiles, plan['candidate'].get('candidate_manifest_sha256'))
    except (ValueError, KeyError, OSError) as error:problems.append('profile custody: '+str(error))
    if any(inputs['candidate'].get(key)!=value for key,value in plan['candidate'].items()):problems.append('workflow/profile candidate mismatch')
    if read(args.profiles/'plan.json')!=planned_profiles():problems.append('profile plan differs from fixed source')
    for expected in planned_profiles():
        path=args.profiles/expected['id'].replace(':','-')/'attempt.json'
        if not path.exists():continue
        row=read(path)
        try:
            for reference in row.get('artifacts',{}).values():artifact(args.profiles,reference)
            raw=json.loads(artifact(args.profiles,row['artifacts']['stdout.json']))
            if raw!=row['result']:raise ValueError('profile summary differs from raw output')
            required={'stdout.json','stderr.txt','resource-samples.json'}
            if expected['kind']!='churn' or raw.get('process_lifecycle') is not None:required.add('resource-accounting.json')
            if set(row.get('artifacts',{}))!=required:raise ValueError('missing raw profile evidence')
            profile_accounting(args.profiles,row,raw)
            if row['id']!=expected['id']:raise ValueError('profile attempt ID differs from path')
        except (OSError,ValueError,KeyError,TypeError,AttributeError) as error:
            problems.append(str(error));row=dict(row,returncode=-1)
        profile_records.append(row)
    try:validate_profile_candidate(inputs,profile_records,bound)
    except (ValueError,KeyError,TypeError) as error:problems.append('profile candidate: '+str(error))
    profiles=validate_profiles(profile_records);resources=[]
    for record in records:
        try:
            topology=json.loads(artifact(args.workflows,record.get('context_topology')))
            waves=json.loads(artifact(args.workflows,record.get('resource_evidence')))
            if len(waves)!=len(topology):raise ValueError('resource/topology wave count differs')
            for index,wave in enumerate(waves):
                if wave['wave']!=index:raise ValueError('resource wave order differs')
                accounting=resource_accounting(args.workflows,wave)
                resources.append(dict(context_topology=topology[index],accounting=accounting))
        except (OSError,ValueError,KeyError,TypeError,AttributeError) as error:problems.append(record.get('attempt_id','unknown')+': '+str(error))
    result=integrated_q_checks(profiles,workflows,resources)
    identity=plan['candidate'].get('candidate_manifest_sha256')
    if identity is not None:
        result['candidate_manifest_sha256']=identity
        if inputs.get('candidate_file_sha256')!=identity:
            problems.append('profile candidate file differs from combined candidate')
    result['errors'].extend(problems)
    if problems:result['q_component_checks_passed']=False
    with args.output.open('x') as stream:
        json.dump(result,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
    print(json.dumps(result));return int(not result['q_component_checks_passed'])

if __name__=='__main__':raise SystemExit(main())
