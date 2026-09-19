"""Fixed Q performance/reliability checks over retained raw benchmark attempts."""
from workflow_resources import percentile
from math import fsum

SAMPLES=30
WARM_ACTIONS=10_000
CONTEXT_COUNTS=(1,5,20)
TARGETS=dict(cold_p50_ms=250,warm_actions_per_second=300,binding_p50_ms=50,
             worker_rss_p50_bytes=64*1024*1024,worker_rss_p95_bytes=160*1024*1024)


def planned_profiles():
    return ([dict(id=f'{mode}:{sample}',kind='page',mode=mode,sample=sample,pages=5,actions=WARM_ACTIONS)
             for mode in ('isolated','isolated-prewarmed','recycled') for sample in range(SAMPLES)]
            +[dict(id=f'group:{count}:{sample}',kind='group',pages=count,sample=sample)
              for count in CONTEXT_COUNTS for sample in range(SAMPLES)]
            +[dict(id=f'churn:{sample}',kind='churn',sample=sample) for sample in range(SAMPLES)])


def validate_profiles(records):
    expected=planned_profiles();expected_ids={row['id'] for row in expected};by_id={};errors=[]
    if not isinstance(records,list):
        records=[];errors.append('malformed raw profile population')
    for row in records:
        if not isinstance(row,dict):errors.append('malformed raw attempt');continue
        identity=row.get('id')
        if not isinstance(identity,str) or identity not in expected_ids or identity in by_id:errors.append('unknown/duplicate raw attempt: '+str(identity));continue
        by_id[identity]=row
    if set(by_id)!=expected_ids:errors.append('missing frozen performance/churn attempts')
    cold=[];warm=[];binding=[];rss=[];groups=[]
    for plan in expected:
        row=by_id.get(plan['id'])
        if row is None:continue
        if type(row.get('returncode')) is not int or row['returncode']!=0 or row.get('timed_out') is not False or row.get('cleanup_complete') is not True or row.get('errors')!=[]:
            errors.append(plan['id']+': execution or cleanup failed');continue
        data=row.get('result',{})
        if not isinstance(data,dict) or not isinstance(data.get('runtime',{}),dict):
            errors.append(plan['id']+': malformed native profile result');continue
        if data.get('runtime',{}).get('sandboxed') is not True:errors.append(plan['id']+': production sandbox attestation missing')
        worker_digest=data.get('runtime',{}).get('worker_binary_sha256')
        if not isinstance(worker_digest,str) or len(worker_digest)!=64 or any(char not in '0123456789abcdef' for char in worker_digest):
            errors.append(plan['id']+': native worker binary identity missing')
        if plan['kind'] in ('page','group'):
            lifecycle=data.get('process_lifecycle',{})
            if (not isinstance(lifecycle,dict) or not isinstance(lifecycle.get('processes',[]),list)
                    or any(not isinstance(process,dict) for process in lifecycle.get('processes',[]))):
                errors.append(plan['id']+': malformed native process lifecycle');continue
            if lifecycle.get('complete') is not True or lifecycle.get('gaps') or lifecycle.get('overflow') or not lifecycle.get('processes'):errors.append(plan['id']+': incomplete native process lifecycle')
            for process in lifecycle.get('processes',[]):
                exit_code,signal=process.get('exit_code'),process.get('signal')
                checked_exit=(type(exit_code) is int and exit_code>=0 and signal is None
                              or type(signal) is int and signal>0 and exit_code is None)
                if (process.get('binary_sha256')!=worker_digest or type(process.get('reaped_unix_ns')) is not int
                        or process['reaped_unix_ns']<=0 or not checked_exit):errors.append(plan['id']+': missing process identity/checked reap')
        if plan['kind']=='page':
            if not isinstance(data.get('cached_isolated_launch_ms',[]),list) or not isinstance(data.get('reset_or_close_ms',[]),list):
                errors.append(plan['id']+': malformed page timing samples');continue
            expected_mode={'isolated':'isolated_on_demand','isolated-prewarmed':'isolated_prewarmed','recycled':'recycled'}[plan['mode']]
            if data.get('worker_lifecycle')!=expected_mode:errors.append(plan['id']+': native placement profile differs')
            if data.get('pages')!=plan['pages'] or data.get('warm_action_count')!=WARM_ACTIONS or data.get('completed_warm_action_count')!=WARM_ACTIONS:errors.append(plan['id']+': sampling changed');continue
            if plan['mode']=='isolated':cold.append(data.get('first_cold_verification_launch_handshake_ms'));warm.append(data.get('warm_actions_per_second'))
            else:
                samples=data.get('cached_isolated_launch_ms',[])
                if len(samples)!=plan['pages']-1:errors.append(plan['id']+': missing binding samples')
                binding.extend(samples)
                if plan['mode']=='recycled' and data.get('recycled_javascript_state_erased') is not True:errors.append(plan['id']+': recycled state survived')
            if len(data.get('reset_or_close_ms',[]))!=plan['pages']:errors.append(plan['id']+': incomplete page cleanup timing')
        elif plan['kind']=='group':
            if not isinstance(data.get('page_measurements',[]),list) or not isinstance(data.get('worker_rss_bytes',[]),list):
                errors.append(plan['id']+': malformed group samples');continue
            if data.get('pages')!=plan['pages'] or data.get('passed') is not True or len(data.get('page_measurements',[]))!=plan['pages']:errors.append(plan['id']+': incomplete group/isolation result')
            samples=data.get('worker_rss_bytes',[])
            if len(samples)!=plan['pages'] or data.get('parent_rss_bytes') is None:errors.append(plan['id']+': missing process-group RSS')
            if all(type(value) is int and value>=0 for value in samples) and data.get('total_worker_rss_bytes')!=sum(samples):errors.append(plan['id']+': group RSS undercounts/differs from native workers')
            rss.extend(samples);groups.append(data)
        else:
            if not isinstance(data.get('false_preservation',{}),dict) or not isinstance(data.get('worker_replacement',{}),dict):
                errors.append(plan['id']+': malformed churn result');continue
            if data.get('status')!='passed' or data.get('false_preservation',{}).get('numerator')!=0 or data.get('worker_replacement',{}).get('status')!='passed':errors.append(plan['id']+': churn/restart failed')
    metrics={}
    for name,values,q in [('cold_p50_ms',cold,.5),('warm_actions_per_second',warm,.5),('binding_p50_ms',binding,.5),('worker_rss_p50_bytes',rss,.5),('worker_rss_p95_bytes',rss,.95)]:
        try:
            value=percentile(values,q)
            if name=='warm_actions_per_second':
                if any(rate<=0 for rate in values):raise ValueError('invalid warm duration')
                # Scale reciprocal durations by the fixed target. Equal target
                # samples stay exact without a tolerance that could admit a miss.
                scale=TARGETS[name]
                value=(len(values)*scale)/fsum(scale/rate for rate in values)
            metrics[name]=value
            if (value<TARGETS[name] if name=='warm_actions_per_second' else value>TARGETS[name]):errors.append(name+': fixed target missed')
        except ValueError:errors.append(name+': invalid/missing samples')
    # Existing benchmark populations are pages in a shared context. This fact is
    # explicit: independent 1/5/20-context isolation still needs workflow evidence.
    return dict(profile_checks_passed=not errors,errors=errors,targets=TARGETS,metrics=metrics,
                planned=len(expected),retained=len(by_id),group_population='pages_in_shared_context',
                independent_context_qualification=False,qualified=False)


def integrated_q_checks(profile,workflow,resource_waves):
    """Combine gates without turning a component pass into full certification."""
    errors=[];counts=set();peaks=[]
    if profile.get('profile_checks_passed') is not True:errors.append('performance/churn profiles not passing')
    if workflow.get('structurally_valid') is not True or workflow.get('planned')!=len(workflow.get('validated',[])):errors.append('full frozen workflow denominator not passing')
    for wave in resource_waves:
        topology=wave.get('context_topology',{});accounting=wave.get('accounting',{})
        count=topology.get('cohort_count');counts.add(count)
        contexts=topology.get('contexts',[])
        if topology.get('total_live_contexts')!=len(contexts) or count not in CONTEXT_COUNTS:errors.append('resource topology missing total live contexts')
        if accounting.get('complete') is not True:errors.append('resource ownership/sampling incomplete')
        known={str(row['context_id']) for row in contexts}
        for worker in accounting.get('workers',[]):
            if not worker.get('leases') or any(lease['context_id'] not in known for lease in worker['leases']):errors.append('worker RSS lacks native context ownership')
            peaks.append(worker.get('sampled_peak_rss_bytes'))
        for group in accounting.get('instantaneous_groups',[]):
            if group.get('unknown_descendants'):errors.append('unattributed process in aggregate resource profile')
            if group.get('aggregate_rss_bytes',0)<group.get('parent_rss_bytes',0)+group.get('worker_rss_bytes',0):errors.append('aggregate resource footprint undercounts processes')
    if counts!=set(CONTEXT_COUNTS):errors.append('independent 1/5/20-context resources missing')
    for q,limit in ((.5,TARGETS['worker_rss_p50_bytes']),(.95,TARGETS['worker_rss_p95_bytes'])):
        try:
            if percentile(peaks,q)>limit:errors.append('integrated worker RSS target missed')
        except ValueError:errors.append('integrated worker RSS samples invalid')
    return dict(q_component_checks_passed=not errors,errors=errors,qualified=False,
                remaining_gates=['full_core_assertions','eight_framework_gates','candidate_signature_and_complete_build_identity','security_and_manual_assessments'])


def validate_profile_custody(root, candidate):
    """Require original-run custody of the entire frozen profile population."""
    import json
    from pathlib import Path
    from completion_provenance import ROOT, digest, read_record, is_digest
    root = Path(root).resolve()
    receipt = json.loads((root / 'execution-custody.json').read_text())
    if (not is_digest(candidate) or receipt.get('schema_version') != 1 or receipt.get('candidate_manifest_sha256') != candidate
            or receipt.get('custody_complete') is not True or receipt.get('errors') != []):
        raise ValueError('profile execution custody incomplete')
    read_record(receipt.get('generation_gate'))
    if read_record(receipt.get('producer_source')) != ROOT / 'scripts/run-workflow-qualification-profiles.py':
        raise ValueError('profile custody producer substituted')
    expected = {item['id'].replace(':', '-') + '/attempt.json' for item in planned_profiles()}
    attempts = receipt.get('attempts', {})
    if set(attempts) != expected:
        raise ValueError('profile custody denominator incomplete')
    for relative, expected_hash in attempts.items():
        path = (root / relative).resolve()
        if root not in path.parents or digest(path) != expected_hash:
            raise ValueError('profile attempt changed after execution')
    return receipt


def validate_profile_candidate(inputs, records, candidate):
    """Recheck original producer identities at consumption, not just at launch."""
    from completion_provenance import workflow_identity
    if not isinstance(inputs, dict):
        raise ValueError('profile candidate inputs must be an object')
    expected = workflow_identity(candidate, candidate['artifacts']['workflow_executor'],
                                 candidate['artifacts']['worker'], candidate['record']['source_tree_sha256'])
    if inputs.get('candidate') != expected or inputs.get('candidate_file_sha256') != candidate['sha256']:
        raise ValueError('profile candidate inputs differ from final candidate')
    names = dict(page='page_benchmark', group='group_benchmark', churn='churn', worker='worker')
    executables = {name: candidate['record']['artifacts'][artifact]['sha256'] for name, artifact in names.items()}
    if inputs.get('executables') != executables:
        raise ValueError('profile executables differ from final candidate')
    if not isinstance(records, list):
        raise ValueError('profile records must be a list')
    for row in records:
        result = row.get('result') if isinstance(row, dict) else None
        runtime = result.get('runtime') if isinstance(result, dict) else None
        if not isinstance(runtime, dict) or runtime.get('worker_binary_sha256') != executables['worker']:
            raise ValueError('raw profile worker differs from final candidate')
