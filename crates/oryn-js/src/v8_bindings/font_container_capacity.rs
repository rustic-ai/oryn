// Included in font_loading. Preparation admits replacement buffers while the
// old buffer and its lease remain owned; callbacks never run under State borrow.
fn reserve_font_container(
    scope:&mut v8::PinScope,state:&Rc<V8BindingState>,bytes:usize,
)->Result<Option<std::sync::Arc<dyn Send+Sync>>,V8BindingError>{
    let original_owner=owner(state);
    let _entry=state.enter_retained_allocation(scope)?;
    let allocation=state.retained_allocation.borrow().clone();
    let lease=allocation.map(|allocation|allocation.reserve(bytes)).transpose().map_err(V8BindingError::EventCallback)?;
    if owner(state)!=original_owner||state.execution_retired()||state.execution_suspended()||scope.is_execution_terminating(){drop(lease);return Err(V8BindingError::Allocation("font container owner retired"))}
    Ok(lease)
}
fn font_prepare_event_capacity(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,additional:usize)->Result<(),V8BindingError>{
    let (length,capacity)={let fonts=state.font_loading.borrow();(fonts.events.len(),fonts.events.capacity())};
    let needed=length.checked_add(additional).ok_or(V8BindingError::Allocation("font event capacity overflow"))?;
    if needed<=capacity{return Ok(())}
    let target=capacity.checked_mul(2).map(|n|n.max(needed)).ok_or(V8BindingError::Allocation("font event capacity overflow"))?;
    let lease=reserve_font_container(scope,state,target.checked_mul(std::mem::size_of::<native_platform::publication::Notification>()).ok_or(V8BindingError::Allocation("font event capacity overflow"))?)?;
    let mut replacement=VecDeque::new();replacement.try_reserve_exact(target).map_err(|_|V8BindingError::Allocation("font event capacity"))?;
    if replacement.capacity()>target{return Err(V8BindingError::Allocation("font event capacity admission"))}
    let old={let mut fonts=state.font_loading.borrow_mut();
        if fonts.events.len()!=length||fonts.events.capacity()!=capacity{return Err(V8BindingError::Allocation("font event capacity changed"))}
        replacement.append(&mut fonts.events);fonts.events=replacement;
        std::mem::replace(&mut fonts.event_capacity,lease)
    };drop(old);Ok(())
}
fn prepare_font_request_capacity(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError>{
    let (length,capacity)={let fonts=state.font_loading.borrow();(fonts.requests.len(),fonts.requests.capacity())};
    if length<capacity{return Ok(())}
    let target=capacity.checked_mul(2).and_then(|n|n.checked_add(usize::from(n==0))).ok_or(V8BindingError::Allocation("font request capacity overflow"))?;
    let lease=reserve_font_container(scope,state,target.checked_mul(std::mem::size_of::<font_loading_transport::FontLoadRequest>()).ok_or(V8BindingError::Allocation("font request capacity overflow"))?)?;
    let mut replacement=VecDeque::new();replacement.try_reserve_exact(target).map_err(|_|V8BindingError::Allocation("font request capacity"))?;
    if replacement.capacity()>target{return Err(V8BindingError::Allocation("font request capacity admission"))}
    let old={let mut fonts=state.font_loading.borrow_mut();
        if fonts.requests.len()!=length||fonts.requests.capacity()!=capacity{return Err(V8BindingError::Allocation("font request capacity changed"))}
        replacement.append(&mut fonts.requests);fonts.requests=replacement;
        std::mem::replace(&mut fonts.request_capacity,lease)
    };drop(old);Ok(())
}
fn font_prepare_load_capacity(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError>{
    let (length,capacity)={let fonts=state.font_loading.borrow();(fonts.loads.len(),fonts.loads.capacity())};
    if length<capacity{return Ok(())}
    let target=capacity.checked_mul(2).map(|n|n.max(1)).ok_or(V8BindingError::Allocation("font load capacity overflow"))?;
    let lease=reserve_font_container(scope,state,target.checked_mul(std::mem::size_of::<LoadRecord>()).ok_or(V8BindingError::Allocation("font load capacity overflow"))?)?;
    let mut replacement=Vec::new();replacement.try_reserve_exact(target).map_err(|_|V8BindingError::Allocation("font load capacity"))?;
    if replacement.capacity()>target{return Err(V8BindingError::Allocation("font load capacity admission"))}
    let old={let mut fonts=state.font_loading.borrow_mut();
        if fonts.loads.len()!=length||fonts.loads.capacity()!=capacity{return Err(V8BindingError::Allocation("font load capacity changed"))}
        replacement.append(&mut fonts.loads);fonts.loads=replacement;
        std::mem::replace(&mut fonts.load_capacity,lease)
    };drop(old);Ok(())
}
fn font_prepare_membership_capacity(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,id:u64,additional:usize)->Result<(),V8BindingError>{
    let(length,capacity,order_length,order_capacity,revision)={let fonts=state.font_loading.borrow();let set=fonts.sets.get(&id).ok_or(V8BindingError::Allocation("font set"))?;(set.faces.len(),set.faces.capacity(),set.iteration_order.len(),set.iteration_order.capacity(),set.membership_revision)};
    let needed=length.checked_add(additional).ok_or(V8BindingError::Allocation("font members overflow"))?;
    let order_needed=order_length.checked_add(additional).ok_or(V8BindingError::Allocation("font iteration overflow"))?;
    let target=capacity.checked_mul(2).map(|n|n.max(needed)).ok_or(V8BindingError::Allocation("font members overflow"))?;
    let order_target=order_capacity.checked_mul(2).map(|n|n.max(order_needed)).ok_or(V8BindingError::Allocation("font iteration overflow"))?;
    let mut members=if needed>capacity{let lease=reserve_font_container(scope,state,target.checked_mul(std::mem::size_of::<u64>()).ok_or(V8BindingError::Allocation("font members bytes"))?)?;let mut values=Vec::new();values.try_reserve_exact(target).map_err(|_|V8BindingError::Allocation("font members"))?;if values.capacity()>target{return Err(V8BindingError::Allocation("font members admission"))}Some((values,lease))}else{None};
    let mut order=if order_needed>order_capacity{let lease=reserve_font_container(scope,state,order_target.checked_mul(std::mem::size_of::<(u64,u64)>()).ok_or(V8BindingError::Allocation("font iteration bytes"))?)?;let mut values=Vec::new();values.try_reserve_exact(order_target).map_err(|_|V8BindingError::Allocation("font iteration"))?;if values.capacity()>order_target{return Err(V8BindingError::Allocation("font iteration admission"))}Some((values,lease))}else{None};
    let old={let mut fonts=state.font_loading.borrow_mut();let set=fonts.sets.get_mut(&id).ok_or(V8BindingError::Allocation("font set"))?;
        if (set.faces.len(),set.faces.capacity(),set.iteration_order.len(),set.iteration_order.capacity(),set.membership_revision)!=(length,capacity,order_length,order_capacity,revision){return Err(V8BindingError::Allocation("font membership changed"))}
        let old_members=if let Some((values,lease))=members.as_mut(){values.append(&mut set.faces);set.faces=std::mem::take(values);std::mem::replace(&mut set.members_capacity,lease.take())}else{None};
        let old_order=if let Some((values,lease))=order.as_mut(){values.append(&mut set.iteration_order);set.iteration_order=std::mem::take(values);std::mem::replace(&mut set.iteration_capacity,lease.take())}else{None};
        (old_members,old_order)
    };drop(old);Ok(())
}
