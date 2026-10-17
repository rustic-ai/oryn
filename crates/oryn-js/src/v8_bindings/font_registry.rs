// Exact-capacity sorted native registry. All allocations are prepared by the
// owning binding scope before insertion; keys retain BTreeMap ordering.
struct FontRegistry<T>{entries:Vec<(u64,T)>,allocation:Option<std::sync::Arc<dyn Send+Sync>>}
impl<T> Default for FontRegistry<T>{fn default()->Self{Self{entries:Vec::new(),allocation:None}}}
impl<'a,T> IntoIterator for &'a FontRegistry<T>{
    type Item=&'a(u64,T);type IntoIter=std::slice::Iter<'a,(u64,T)>;
    fn into_iter(self)->Self::IntoIter{self.entries.iter()}
}
impl<T> FontRegistry<T>{
    fn len(&self)->usize{self.entries.len()}
    fn is_empty(&self)->bool{self.entries.is_empty()}
    fn contains_key(&self,id:&u64)->bool{self.entries.binary_search_by_key(id,|(id,_)|*id).is_ok()}
    fn get(&self,id:&u64)->Option<&T>{self.entries.binary_search_by_key(id,|(id,_)|*id).ok().map(|i|&self.entries[i].1)}
    fn get_mut(&mut self,id:&u64)->Option<&mut T>{self.entries.binary_search_by_key(id,|(id,_)|*id).ok().map(|i|&mut self.entries[i].1)}
    fn iter(&self)->impl Iterator<Item=(&u64,&T)>{self.entries.iter().map(|(id,value)|(id,value))}
    fn values(&self)->impl Iterator<Item=&T>{self.entries.iter().map(|(_,value)|value)}
    fn insert(&mut self,id:u64,value:T)->Result<Option<T>,V8BindingError>{
        match self.entries.binary_search_by_key(&id,|(id,_)|*id){
            Ok(i)=>Ok(Some(std::mem::replace(&mut self.entries[i].1,value))),
            Err(i)=>{if self.entries.len()==self.entries.capacity(){return Err(V8BindingError::Allocation("font registry capacity not prepared"))}self.entries.insert(i,(id,value));Ok(None)}
        }
    }
}
fn font_prepare_face_registry(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError>{
    let(length,capacity)={let fonts=state.font_loading.borrow();(fonts.face_objects.len(),fonts.face_objects.entries.capacity())};
    if length<capacity{return Ok(())}
    let target=capacity.checked_mul(2).map(|n|n.max(1)).ok_or(V8BindingError::Allocation("font registry capacity overflow"))?;
    let allocation=reserve_font_container(scope,state,target.checked_mul(std::mem::size_of::<(u64,FaceRecord)>()).ok_or(V8BindingError::Allocation("font registry bytes"))?)?;
    let mut entries=Vec::new();entries.try_reserve_exact(target).map_err(|_|V8BindingError::Allocation("font registry allocation"))?;
    if entries.capacity()>target{return Err(V8BindingError::Allocation("font registry admission"))}
    let old={let mut fonts=state.font_loading.borrow_mut();let registry=&mut fonts.face_objects;
        if registry.len()!=length||registry.entries.capacity()!=capacity{return Err(V8BindingError::Allocation("font registry changed"))}
        entries.append(&mut registry.entries);registry.entries=entries;std::mem::replace(&mut registry.allocation,allocation)
    };drop(old);Ok(())
}
fn font_prepare_set_registry(scope:&mut v8::PinScope,state:&Rc<V8BindingState>)->Result<(),V8BindingError>{
    let(length,capacity)={let fonts=state.font_loading.borrow();(fonts.sets.len(),fonts.sets.entries.capacity())};
    if length<capacity{return Ok(())}
    let target=capacity.checked_mul(2).map(|n|n.max(1)).ok_or(V8BindingError::Allocation("font registry capacity overflow"))?;
    let allocation=reserve_font_container(scope,state,target.checked_mul(std::mem::size_of::<(u64,SetRecord)>()).ok_or(V8BindingError::Allocation("font registry bytes"))?)?;
    let mut entries=Vec::new();entries.try_reserve_exact(target).map_err(|_|V8BindingError::Allocation("font registry allocation"))?;
    if entries.capacity()>target{return Err(V8BindingError::Allocation("font registry admission"))}
    let old={let mut fonts=state.font_loading.borrow_mut();let registry=&mut fonts.sets;
        if registry.len()!=length||registry.entries.capacity()!=capacity{return Err(V8BindingError::Allocation("font registry changed"))}
        entries.append(&mut registry.entries);registry.entries=entries;std::mem::replace(&mut registry.allocation,allocation)
    };drop(old);Ok(())
}
