"""Bounded OS process-tree samples, reconciled with native worker ownership.

RSS is sampled, not an asserted high-water mark. Aggregate values always sum a
single timestamp, never unrelated per-process peaks. No process names identify
workers: only native resource-audit PID ownership establishes that attribution.
"""
import math
import subprocess
import sys
import threading
import time


def parse_processes(text):
    rows={}
    for line in text.splitlines():
        parts=line.strip().split()
        if len(parts)!=3:raise ValueError('malformed process sample')
        pid,parent,rss=map(int,parts)
        if pid<=0 or parent<0 or rss<0 or pid in rows:raise ValueError('invalid process identity/RSS')
        rows[pid]=dict(pid=pid,parent_pid=parent,rss_bytes=rss*1024)
    return rows


def descendants(rows,parent):
    included={parent};changed=True
    while changed:
        children={pid for pid,row in rows.items() if row['parent_pid'] in included}
        changed=not children.issubset(included);included|=children
    return [rows[pid] for pid in sorted(included) if pid in rows]


class ProcessSampler:
    def __init__(self,parent_pid,interval_ms=50,limit=6000, custody=None):
        self.custody = custody
        self.parent_pid=parent_pid;self.interval_ms=interval_ms;self.limit=limit
        self.samples=[];self.errors=[];self.overflow=False;self.stop=threading.Event()
        self.backend_counts = {'macos_libproc': 0, 'ps': 0, 'macos_libproc_fallback': 0}
        self.backend_fallbacks = []
        self.thread=threading.Thread(target=self.collect,daemon=True)
    def start(self):self.thread.start()
    def collect(self):
        collect_tree = None
        while not self.stop.is_set():
            try:
                unix_started_ns=time.time_ns()
                if sys.platform == 'darwin':
                    try:
                        if collect_tree is None:
                            from workflow_macos_resources import MacOSProcessTree, ParentReusedError
                            collect_tree = MacOSProcessTree(self.parent_pid, query_budget_seconds=0.25)
                        rows = collect_tree()
                        self.backend_counts['macos_libproc'] += 1
                    except ParentReusedError:
                        raise
                    except OSError as error:
                        # Preserve the original OS snapshot on transient exit/exec
                        # permission races. Never fill a failed query with zero RSS.
                        self.backend_counts['macos_libproc_fallback'] += 1
                        if len(self.backend_fallbacks) < 64:
                            self.backend_fallbacks.append(dict(sample_index=len(self.samples), error=str(error)[:1024]))
                        output=subprocess.check_output(['ps','-axo','pid=,ppid=,rss='],text=True,timeout=3)
                        rows=descendants(parse_processes(output),self.parent_pid)
                else:
                    output=subprocess.check_output(['ps','-axo','pid=,ppid=,rss='],text=True,timeout=3)
                    rows=descendants(parse_processes(output),self.parent_pid)
                    self.backend_counts['ps'] += 1
                if self.custody is not None:
                    self.custody.observe(rows)
                if len(self.samples)>=self.limit:self.overflow=True;return
                if rows:self.samples.append(dict(monotonic_ns=time.monotonic_ns(),unix_started_ns=unix_started_ns,unix_ended_ns=time.time_ns(),processes=rows))
            except (ValueError,OSError,subprocess.SubprocessError) as error:
                self.errors.append(type(error).__name__+': '+str(error))
                if self.custody is not None:
                    self.custody.mark_uncertain('resource/ownership observation: '+repr(error))
                return
            self.stop.wait(self.interval_ms/1000)
    def finish(self):
        self.stop.set();self.thread.join(timeout=4)
        if self.thread.is_alive():self.errors.append('sampler did not stop')
        return dict(schema_version=1,parent_pid=self.parent_pid,interval_ms=self.interval_ms,
                    sampled_not_high_water=True,overflow=self.overflow,errors=self.errors,samples=self.samples,
                    backend_counts=self.backend_counts, backend_fallbacks=self.backend_fallbacks,
                    fallback_diagnostics_truncated=self.backend_counts['macos_libproc_fallback'] > len(self.backend_fallbacks))


def percentile(values,quantile):
    if not values or any(type(value) not in (int,float) or not math.isfinite(value) or value<0 for value in values):raise ValueError('missing/nonfinite measurement')
    return sorted(values)[max(0,math.ceil(len(values)*quantile)-1)]


def reconcile_resources(raw,native):
    if 'worker_lifecycle' in native:return reconcile_lifecycle_resources(raw,native)
    errors=[];owners={};peaks={};groups=[]
    if raw.get('schema_version')!=1 or raw.get('overflow') or raw.get('errors') or not raw.get('samples'):errors.append('incomplete OS resource sampling')
    if raw.get('parent_pid')!=native.get('parent_pid'):errors.append('resource parent identity mismatch')
    for session in native.get('sessions',[]):
        audit=session.get('resource_audit') or {}
        if audit.get('complete') is not True or audit.get('overflow') or audit.get('gaps'):errors.append('native resource ownership audit incomplete')
        for row in audit.get('records',[]):
            pid=row.get('worker_pid');page=row.get('page_id')
            if not pid or page is None:errors.append('native request lacks process/page attribution');continue
            owner=(str(session['context_id']),str(page))
            # Multiple page leases may share a process in a placement profile;
            # retain every lease, never count the process RSS twice.
            owners.setdefault(pid,set()).add(owner)
    for sample in raw.get('samples',[]):
        rows=sample['processes'];ids={row['pid'] for row in rows}
        if len(ids)!=len(rows):errors.append('duplicate process in instantaneous RSS sample');continue
        parent=[row['rss_bytes'] for row in rows if row['pid']==raw['parent_pid']]
        if len(parent)!=1:errors.append('sample lacks original live parent');continue
        workers=[row for row in rows if row['pid'] in owners]
        unknown=[row['pid'] for row in rows if row['pid']!=raw['parent_pid'] and row['pid'] not in owners]
        if unknown:errors.append('OS descendant lacks native resource ownership')
        for row in workers:peaks[row['pid']]=max(peaks.get(row['pid'],0),row['rss_bytes'])
        groups.append(dict(monotonic_ns=sample['monotonic_ns'],parent_rss_bytes=parent[0],worker_rss_bytes=sum(row['rss_bytes'] for row in workers),aggregate_rss_bytes=sum(row['rss_bytes'] for row in rows),unknown_descendants=unknown,live_workers=len(workers)))
    missing=set(owners)-set(peaks)
    if missing:errors.append('native worker lifetime escaped OS sampling')
    if not owners:errors.append('no native worker identities')
    return dict(complete=not errors,errors=errors,sampled_not_high_water=True,
                workers=[dict(pid=pid,leases=[dict(context_id=context,page_id=page) for context,page in sorted(owners[pid])],sampled_peak_rss_bytes=peaks.get(pid)) for pid in sorted(owners)],
                instantaneous_groups=groups,
                worker_sampled_peak_p50_bytes=percentile(list(peaks.values()),.5) if peaks else None,
                worker_sampled_peak_p95_bytes=percentile(list(peaks.values()),.95) if peaks else None,
                aggregate_sampled_peak_bytes=max((row['aggregate_rss_bytes'] for row in groups),default=None))


def reconcile_lifecycle_resources(raw,native):
    """Use native spawn/lease/reap records, including non-HTTP startup workers."""
    try:
        validate_lifecycle_shape(raw,native)
    except ValueError as error:
        return dict(complete=False,errors=[str(error)],sampled_not_high_water=True,
                    workers=[],auxiliary_processes=[],instantaneous_groups=[],
                    worker_sampled_peak_p50_bytes=None,worker_sampled_peak_p95_bytes=None,
                    aggregate_sampled_peak_bytes=None)
    errors=[];journal=native['worker_lifecycle'];processes=journal.get('processes',[])
    if raw.get('schema_version')!=1 or raw.get('overflow') or raw.get('errors') or not raw.get('samples'):errors.append('incomplete OS resource sampling')
    if raw.get('parent_pid')!=native.get('parent_pid'):errors.append('resource parent identity mismatch')
    if journal.get('schema_version')!=1 or journal.get('complete') is not True or journal.get('overflow') or journal.get('gaps'):errors.append('incomplete native process lifecycle journal')
    identities=set();contexts={str(row['context_id']) for row in native.get('sessions',[])};by_pid={};peaks={};groups=[]
    worker_hash=native.get('runtime',{}).get('worker_binary_sha256')
    for process in processes:
        identity=process.get('launch_id');pid=process.get('worker_pid');role=process.get('role')
        if identity in identities:errors.append('duplicate native process launch');continue
        identities.add(identity);by_pid.setdefault(pid,[]).append(process)
        if process.get('parent_pid')!=native.get('parent_pid') or not worker_hash or process.get('binary_sha256')!=worker_hash:errors.append('process parent/binary identity mismatch')
        start,end=process.get('spawned_unix_ns'),process.get('reaped_unix_ns')
        if not start or not end or end<start or (process.get('exit_code') is None and process.get('signal') is None):errors.append('process lacks checked exit lifecycle')
        if role not in ('verification','prewarmed','page'):errors.append('unknown native process role')
        if role=='page' and not process.get('leases'):errors.append('page worker has no actual leases')
        for lease in process.get('leases',[]) if role!='verification' else []:
            if str(lease.get('context_id')) not in contexts:errors.append('worker lease belongs to unreported context')
            if lease.get('ended_unix_ns') is None or lease.get('started_unix_ns',0)>lease.get('ended_unix_ns',0):errors.append('worker lease retirement missing')
    # Lifecycle event samples contribute only to the individual launch peak.
    # They do not share an instantaneous tree timestamp with periodic samples.
    for process in processes:
        if process.get('resource_identity_version') == 1:
            if process.get('kernel_birth') is None:
                errors.append('native launch lacks kernel birth identity')
            if any('identity mismatch' in error for error in process.get('resource_sample_errors', [])):
                errors.append('native resource event identity mismatch')
            for event in process.get('event_samples', []):
                identity=process['launch_id']
                peaks[identity]=max(peaks.get(identity,0),event['rss_bytes'])
    last_wall=None
    for sample in raw.get('samples',[]):
        start,end=sample.get('unix_started_ns'),sample.get('unix_ended_ns')
        if not start or not end or end<start or (last_wall is not None and start<last_wall):errors.append('invalid/backwards resource wall clock');continue
        last_wall=end;rows=sample['processes'];unknown=[];worker_rss=0;auxiliary_rss=0
        parent=[row['rss_bytes'] for row in rows if row['pid']==raw['parent_pid']]
        if len(parent)!=1:errors.append('resource sample lacks original parent');continue
        if len({row['pid'] for row in rows})!=len(rows):errors.append('duplicate OS process sample');continue
        for row in rows:
            if row['pid']==raw['parent_pid']:continue
            matches=[process for process in by_pid.get(row['pid'],[])
                     if (process.get('launch_started_unix_ns') if process.get('resource_identity_version') == 1 else process.get('spawned_unix_ns',end+1))<=end
                     and (process.get('reaped_unix_ns') or start)>=start
                     and (process.get('resource_identity_version') != 1 or
                          row.get('kernel_birth') == process.get('kernel_birth') and row.get('kernel_birth') is not None)]
            if len(matches)!=1:unknown.append(row['pid']);continue
            process=matches[0];identity=process['launch_id'];peaks[identity]=max(peaks.get(identity,0),row['rss_bytes'])
            if process['role']=='page':worker_rss+=row['rss_bytes']
            else:auxiliary_rss+=row['rss_bytes']
        groups.append(dict(monotonic_ns=sample['monotonic_ns'],parent_rss_bytes=parent[0],worker_rss_bytes=worker_rss,auxiliary_worker_rss_bytes=auxiliary_rss,aggregate_rss_bytes=sum(row['rss_bytes'] for row in rows),unknown_descendants=unknown))
        if unknown:errors.append('OS descendant has no unique native launch lifetime')
    page_workers=[process for process in processes if process.get('role')=='page']
    if not page_workers:errors.append('no native page worker processes')
    if any(process['launch_id'] not in peaks for process in page_workers):errors.append('page worker lifetime escaped RSS sampling')
    values=[peaks[process['launch_id']] for process in page_workers if process['launch_id'] in peaks]
    return dict(complete=not errors,errors=errors,sampled_not_high_water=True,
                workers=[dict(pid=process['worker_pid'],launch_id=process['launch_id'],role=process['role'],leases=[dict(context_id=str(lease['context_id']),page_id=str(lease['page_id'])) for lease in process['leases']],sampled_peak_rss_bytes=peaks.get(process['launch_id']),event_sample_count=len(process.get('event_samples', []))) for process in page_workers],
                auxiliary_processes=[dict(process,sampled_peak_rss_bytes=peaks.get(process['launch_id'])) for process in processes if process.get('role')!='page'],
                instantaneous_groups=groups,worker_sampled_peak_p50_bytes=percentile(values,.5) if values else None,
                worker_sampled_peak_p95_bytes=percentile(values,.95) if values else None,
                aggregate_sampled_peak_bytes=max((row['aggregate_rss_bytes'] for row in groups),default=None))


def validate_lifecycle_shape(raw,native):
    """Validate typed identities and time intervals before attribution arithmetic."""
    def obj(value):
        if not isinstance(value,dict):raise ValueError('malformed lifecycle evidence object')
        return value
    def rows(value):
        if not isinstance(value,list):raise ValueError('malformed lifecycle evidence population')
        return [obj(row) for row in value]
    def integer(value,minimum=0):
        if type(value) is not int or value<minimum:raise ValueError('invalid lifecycle identity/time/measurement')
        return value
    obj(raw);obj(native)
    integer(raw.get('parent_pid'),1);integer(native.get('parent_pid'),1)
    runtime=obj(native.get('runtime'))
    digest=runtime.get('worker_binary_sha256')
    if not isinstance(digest,str) or len(digest)!=64 or any(c not in '0123456789abcdef' for c in digest):
        raise ValueError('invalid lifecycle worker digest')
    for session in rows(native.get('sessions')):integer(session.get('context_id'))
    journal=obj(native.get('worker_lifecycle'))
    for process in rows(journal.get('processes')):
        for field in ('launch_id','worker_pid','parent_pid'):integer(process.get(field),1)
        start=integer(process.get('spawned_unix_ns'),1)
        end=integer(process.get('reaped_unix_ns'),1)
        if end<start:raise ValueError('process reap precedes launch')
        version=process.get('resource_identity_version',0)
        if type(version) is not int or version not in (0,1):raise ValueError('unsupported resource identity version')
        if version == 1:
            launch=integer(process.get('launch_started_unix_ns'),1)
            if launch>start:raise ValueError('launch bounds reversed')
            birth=process.get('kernel_birth')
            def valid_birth(value):
                if not isinstance(value,list) or len(value)!=2:raise ValueError('invalid kernel birth')
                integer(value[0],1);integer(value[1])
                if value[1]>=1000000:raise ValueError('invalid kernel birth microseconds')
            if birth is not None:
                valid_birth(birth)
                born=birth[0]*1000000000+birth[1]*1000
                if born+999<launch or born>start:raise ValueError('kernel birth escapes launch bounds')
            events=rows(process.get('event_samples'))
            if len(events)>2:raise ValueError('native event sample bound exceeded')
            phases=set()
            for event in events:
                phase=event.get('phase')
                if phase not in ('launch','retirement') or phase in phases:raise ValueError('invalid resource event phase')
                phases.add(phase)
                began=integer(event.get('unix_started_ns'),1);ended=integer(event.get('unix_ended_ns'),1)
                integer(event.get('rss_bytes'));valid_birth(event.get('kernel_birth'))
                if birth is None or event['kernel_birth']!=birth:raise ValueError('event kernel identity mismatch')
                if not launch<=began<=ended<=end:raise ValueError('resource event escapes launch lifetime')
            sampling_errors=process.get('resource_sample_errors')
            if not isinstance(sampling_errors,list) or len(sampling_errors)>2 or any(not isinstance(e,str) for e in sampling_errors):
                raise ValueError('invalid native resource sample errors')

        code,signal=process.get('exit_code'),process.get('signal')
        if not (type(code) is int and code>=0 and signal is None
                or type(signal) is int and signal>0 and code is None):
            raise ValueError('process lacks typed checked exit')
        for lease in rows(process.get('leases')):
            integer(lease.get('context_id'));integer(lease.get('page_id'))
            began=integer(lease.get('started_unix_ns'),1)
            ended=integer(lease.get('ended_unix_ns'),1)
            if not start<=began<=ended<=end:
                raise ValueError('worker lease escapes process lifetime')
    for sample in rows(raw.get('samples')):
        integer(sample.get('monotonic_ns'))
        integer(sample.get('unix_started_ns'),1);integer(sample.get('unix_ended_ns'),1)
        for process in rows(sample.get('processes')):
            integer(process.get('pid'),1);integer(process.get('rss_bytes'))
            if 'kernel_birth' in process:
                birth=process['kernel_birth']
                if not isinstance(birth,list) or len(birth)!=2:raise ValueError('invalid sampled kernel birth')
                integer(birth[0],1);integer(birth[1])
                if birth[1]>=1000000:raise ValueError('invalid sampled birth microseconds')

