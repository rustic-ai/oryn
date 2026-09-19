"""Pinned concrete behavior recipes and independent outcome projections.

Each projection reads durable backend facts; original family validators additionally
check identity relationships, browser observations, action counts and authority logs.
"""
import json

RECIPES = {
    "saas.oauth_rejection": [("saas",4,"oauth_reject_"+mode) for mode in ("state","redirect","verifier","replay","expiry")],
    "saas.isolation": [("saas",4,"pkce_isolated_records")],
    "forms.delivery": [("forms",4,"native_get"),("forms",4,"native_post"),("forms",4,"native_multipart")],
    "forms.submitters": [("forms",4,"native_submitter"),("forms",4,"native_cancel")],
    "forms.reentrancy": [("forms",4,"native_reentrant")],
    "forms.confinement": [("forms",4,"native_confinement")],
    "forms.controls": [("forms",3,"multistep_custom_role"),("forms",3,"multistep_authorized_upload")],
    "forms.validation": [("forms",3,"async_email_recovery"),("forms",3,"multistep_cancel"),("forms",3,"multistep_cancel_recovery")],
    'shopping.discovery': [('shopping',2,'framed_persisted_checkout')],
    'shopping.cart': [('shopping',2,'framed_persisted_checkout')],
    'shopping.checkout': [('shopping',2,'framed_persisted_checkout'),('shopping',2,'framed_required_blocks')],
    'shopping.payment_frame': [('shopping',2,'framed_persisted_checkout')],
    'shopping.stale_quote': [('shopping',2,'framed_price_change_blocks'),('shopping',2,'framed_price_change_recovery'),('shopping',2,'framed_stock_change_blocks'),('shopping',2,'framed_stock_change_recovery')],
    'shopping.duplicate': [('shopping',2,'framed_duplicate_checkout')],
    'saas.pkce': [('saas',2,'pkce_create_update_reload_logout')],
    'saas.mfa': [('saas',2,'pkce_mfa_create_update_reload_logout')],
    'saas.records': [('saas',2,'pkce_create_update_reload_logout')],
    'saas.session': [('saas',2,'pkce_logged_out_access_denied'),('saas',2,'pkce_expired_session_save_recovery')],
    'saas.save_recovery': [('saas',2,'pkce_concurrent_edit_recovery')],
    'booking.timezones': [('booking',2,name) for name in ('client_fold_first','client_fold_second','client_vancouver','client_utc','client_new_york_gap','client_vancouver_gap')],
    'booking.reserve': [('booking',2,'client_phone_option'),('booking',2,'client_required_validation')],
    'booking.reload': [('booking',2,'client_fold_first')],
    'booking.stale_slot': [('booking',2,'client_stale_recovery')],
    'booking.cancel_retry': [('booking',2,'client_cancel_retry'),('booking',2,'client_duplicate_request')],
}

DIMENSIONS = {'forms':['portals_custom_controls','async_persistence','client_rendering'],'shopping':['ssr_hydration','client_rendering','routing_chunks','cross_origin_frames','async_persistence'],
              'saas':['client_rendering','cross_origin_frames','async_persistence'],
              'booking':['client_timezones','portals_custom_controls','async_persistence']}


def select(rows, keys):
    return [{key:row.get(key) for key in keys} for row in rows]


def observed(family, case, data):
    if family=="forms" and case.get("extended"):
        return dict(receipts=data["receipts"],requests=select(data["requests"],("method","path","status")),accepted=data["case"]["accepted"],denials=data["denials"])
    if family=="forms":
        return dict(receipts=[dict(fields=row["fields"],files=row["files"]) for row in data["receipts"]],statuses=[row["status"] for row in data["requests"]],validations=data.get("validations",[]))
    if family=='shopping':
        return dict(carts=select(data['carts'],('variant','quantity')),
                    orders=select(data['orders'],('variant','quantity','unit_price','total','name','email')),
                    requests=select(data['requests'],('action','outcome','cookie_valid')),
                    stock=select(data['cases'],('blue_stock','green_stock','unit_price')),
                    payments=select(data.get('payment_events',[]),('action','outcome')))
    if family=='saas' and case.get('isolation_probe'):
        clean=dict(data)
        denied=[row for row in data['requests'] if row['action']=='update' and row['outcome']==401 and row['authorized']==0]
        views=[row for row in data['views'] if row['page']=='records' and row['authorized']==0]
        clean['requests']=[row for row in data['requests'] if row not in denied];clean['views']=[row for row in data['views'] if row not in views]
        result=observed(family,dict(case,isolation_probe=False),clean)
        result['probe_requests']=select(denied,('action','outcome','authorized'));result['probe_views']=select(views,('page','authorized'))
        return result
    if family=='saas':
        return dict(cases=select(data['cases'],('authenticated','logins')),
                    records=select(data['records'],('title','version')),
                    requests=select(data['requests'],('action','outcome','authorized')),
                    views=select(data['views'],('page','authorized')),
                    expirations=[row['reason'] for row in data.get('expirations',[])],
                    oauth=data.get('oauth',[]),login_events=data.get('login_events',[]),
                    concurrent_edits=data.get('concurrent_edits',[]))
    if family=='booking':
        return dict(reservations=select(data['reservations'],('slot','utc','local','zone')),
                    statuses=[row['status'] for row in data['requests']],
                    unavailable=sorted(row['slot'] for row in data['slots'] if not row['available']),
                    client_selections=select(data.get('client_selections',[]),('local','zone','utc','valid')),
                    options=[row['appointment_option'] for row in data.get('offer_options',[])],
                    cancellations=len(data.get('cancellations',[])))
    raise ValueError('unknown family projection')


def expected(family, case, receiver):
    if family=='shopping':
        stock=dict(blue_stock=0 if case['scenario']=='stock' else 10,green_stock=10,unit_price=case['unit_price'])
        if case['orders']: stock[case['variant']+'_stock']-=2
        requests=[dict(action='cart',outcome=303,cookie_valid=1)]*2
        # Preserve actual interleaving: stale-stock recovery edits the cart after
        # the rejected checkout, before its accepted replacement checkout.
        for index,status in enumerate(case['checkout_outcomes']):
            if index==1 and case['recovery']=='stock':requests.append(dict(action='cart',outcome=303,cookie_valid=1))
            requests.append(dict(action='checkout',outcome=status,cookie_valid=1))
        payments=[dict(action=action,outcome=200) for _ in range(2 if case['recovery'] else 1)
                  for action in (('frame_view','authorized') if case['fill'] else ('frame_view',))]
        return dict(carts=[dict(variant=case['variant'],quantity=2)],
                    orders=[dict(variant=case['variant'],quantity=2,unit_price=case['unit_price'],total=2*case['unit_price'],name='Zoë Test',email='zoe@example.test')]*case['orders'],
                    requests=requests,stock=[stock],payments=payments)
    if family=='saas' and case.get('isolation_probe'):
        result=expected(family,dict(case,isolation_probe=False),receiver)
        result['probe_requests']=[dict(action='update',outcome=401,authorized=0)];result['probe_views']=[dict(page='records',authorized=0)]
        return result
    if family=='saas':
        return dict(cases=[dict(authenticated=case['expected_authenticated'],logins=case['expected_logins'])],
                    records=[] if case['expected_record'] is None else [case['expected_record']],
                    requests=case['expected_requests'],views=case['expected_views'],expirations=case.get('expected_expirations',[]),
                    oauth=case.get('expected_oauth',[]),login_events=case.get('expected_login_events',[]),
                    concurrent_edits=case.get('expected_concurrent_edits',[]))
    if family=='booking':
        if case.get('gap'):
            return dict(reservations=[],statuses=[],unavailable=[],client_selections=[],options=[],cancellations=0)
        unavailable=set()
        if case['scenario']=='stale':unavailable.add(case['slot'])
        if case['reservations']:unavailable.add(case['final_slot'])
        return dict(reservations=[dict(slot=case['final_slot'],utc=receiver.SLOTS[case['final_slot']],local=receiver.local_time(case['final_slot'],case['zone']),zone=case['zone'])]*case['reservations'],
                    statuses=case['statuses'],unavailable=sorted(unavailable),client_selections=case.get('client_selections',[]),
                    options=case.get('appointment_options',[]),cancellations=case.get('cancellations',0))
    raise ValueError('unknown family projection')


def forms_expected(contract,case,upload):
    if case.get('extended'):
        receipts=[]
        for row in case['expected_receipts']:
            receipts.append(dict(method=row['method'],path=row['path'],fields=row['fields'],files=[upload] if row['authorized_upload'] else []))
        return dict(receipts=receipts,requests=case['expected_requests'],accepted=case['expected_accepted'],denials=case['expected_denials'])
    values={**contract['fields'],**case.get('expected_fields',{})}
    fields=[[key,values[key]] for key in ('name','email','role','message','quantity','date','time','reference')]
    fields += [['skill',value] for value in values['skill']]
    fields += [['case_id','{case_id}'],['intent','apply']]
    receipts=case.get('expected_receipts',1)
    return dict(receipts=[dict(fields=fields,files=[upload] if case['upload'] else [])]*receipts,
                statuses=[303]*receipts+case.get("extra_request_statuses",[]),validations=case.get('expected_validations',[]))
