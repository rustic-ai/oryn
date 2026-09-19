"""Source-defined fault boundaries and exact oracle changes for recovery cells.

Idle worker loss and interrupted durable-state reads are explicit mechanisms;
these do not mislabel the idle boundary as an interrupted write. Response delays
can occur after the actual application mutation commits but before its reply.
"""
import copy

PREFERRED = {
 'research.stale':'generation_bound_stale_reference',
 'forms.validation':'async_email_recovery','forms.reentrancy':'native_reentrant',
 'shopping.stale_quote':'framed_price_change_recovery','shopping.duplicate':'framed_duplicate_checkout',
 'saas.mfa':'pkce_mfa_create_update_reload_logout','saas.session':'pkce_expired_session_save_recovery',
 'saas.save_recovery':'pkce_concurrent_edit_recovery',
 'booking.stale_slot':'client_stale_recovery','booking.cancel_retry':'client_cancel_retry',
}


def decorate(family,case,lines,fault,base):
    case=copy.deepcopy(case);lines=list(lines);recovery=[];arm=None
    index=len(lines)
    if family=='research':route=case['final'];normal_reads=sum(row['path']==route for row in case['visits'])
    elif family=='forms':route='receipt';normal_reads=1 if case.get('extended') else 2
    elif family=='shopping':route='order';normal_reads=3 if case['duplicate'] else 2
    elif family=='booking':route='confirmation';normal_reads=3 if case['duplicate'] else 2
    else:route=case['final'];normal_reads=sum(row['page']==route for row in case['expected_views'])
    if family=='forms':
        final=base+'/receipt' if case.get('extended') else base.replace('/form/','/receipt/')
    else:final=base+'/'+route
    if fault=='delayed_response':
        mutation_route={'research':'article/beta','forms':'submit','shopping':'checkout','booking':'reserve','saas':'update'}[family]
        arm=dict(route=mutation_route,expected_status=200 if family=='research' else 303,skip_matches=0,delay_ms=300)
        index=0
    elif fault in ('worker_failure','cancellation'):
        lines += [f'goto "{final}"']
        extra_reads=2 if fault=='cancellation' else 1
        if fault=='cancellation':
            recovery=[dict(oil=f'goto "{final}"'),dict(oil='observe')]
            arm=dict(route=route,expected_status=200,skip_matches=normal_reads,delay_ms=2000)
            case.setdefault('step_options',{})[str(index)]={'expected_error':'native operation deadline exceeded'}
        else:lines+=['observe']
        if family=='research':case['visits'] += [dict(path=route,query={},status=200)]*extra_reads
        elif family=='shopping':case['extra_order_views']=extra_reads
        elif family=='booking':case['extra_confirmation_views']=extra_reads
        elif family=='saas':
            last=next(row for row in reversed(case['expected_views']) if row['page']==route)
            case['expected_views'] += [dict(last)]*extra_reads
    elif fault=='duplicate_delivery':
        label={'research':'Canonical source','forms':'Repeat document request','shopping':'Repeat order request','booking':'Repeat reservation request','saas':'Repeat saved request'}[family]
        if family=='saas':
            index=next(i for i,line in enumerate(lines) if line=='click "Sign out"')
            lines[index:index]=[f'click "{label}"','wait visible css(#duplicate-status[data-count="2"][data-status="409"])']
            position=next(i for i,row in enumerate(case['expected_requests']) if row['action']=='logout')
            previous=next(row for row in reversed(case['expected_requests'][:position]) if row['action'] in ('create','update') and row['outcome']==303)
            case['expected_requests'][position:position]=[dict(action=previous['action'],outcome=409,authorized=1)]*2
            # Existing per-step descriptors after insertion retain their target.
            case['step_options']={str(int(key)+2 if int(key)>=index else int(key)):value for key,value in case.get('step_options',{}).items()}
        else:
            lines += [f'click "{label}"']
            if family=='forms':lines += [f'goto "{final}"','observe']
            else:lines += ['observe']
            if family=='research':
                case['visits'] += [dict(path=route,query={},status=200)]*2
                case['expected_action_count']=case.get('expected_action_count',1)+1
            elif family=='forms':
                case['extra_native_actions']=1
                if case.get('extended'):case['expected_requests'] += [dict(method='POST',path='submit',status=409)]*2
                else:case['extra_request_statuses']=[409,409]
            elif family=='shopping':case['checkout_outcomes'] += [303,303];case['extra_order_views']=2
            elif family=='booking':case['statuses'] += [303,303];case['extra_confirmation_views']=2
    else:raise ValueError('unknown required fault')
    return dict(case=case,lines=lines,recovery_steps=recovery,
                fault=dict(kind=fault,before_step=index,delay_ms=50 if fault=='cancellation' else 300),arm=arm)
