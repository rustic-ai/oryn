// Included inside Servo selector_parser; native AST, never a CSS source cache.
#[derive(Clone, Debug, Eq, PartialEq, ToShmem)]
/// Original parsed selector argument to the functional slotted-state predicate.
pub struct SlottedSelector(pub selectors::parser::SelectorList<SelectorImpl>);

impl std::hash::Hash for SlottedSelector {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        struct Writer<'a, H>(&'a mut H);
        impl<H: std::hash::Hasher> fmt::Write for Writer<'_, H> {
            fn write_str(&mut self, value: &str) -> fmt::Result {
                self.0.write(value.as_bytes()); Ok(())
            }
        }
        // Eq compares the original typed AST; equal AST serializes equally.
        // Unequal AST collision is legal. No temporary String allocation.
        self.0.to_css(&mut Writer(hasher)).expect("Hasher writer is infallible");
    }
}

impl SlottedSelector {
    /// Expose nested selector dependencies to the native invalidation visitor.
    pub fn visit<V: SelectorVisitor<Impl=SelectorImpl>>(&self, visitor:&mut V)->bool {
        self.0.slice().iter().all(|selector|selector.visit(visitor))
    }
}

impl malloc_size_of::MallocSizeOf for SlottedSelector {
    fn size_of(&self, ops:&mut malloc_size_of::MallocSizeOfOps)->usize {
        use selectors::parser::{Component,Selector,SelectorList};
        struct Heap<'a,'s>{ops:&'a mut malloc_size_of::MallocSizeOfOps,bytes:usize,pending:Vec<&'s Selector<SelectorImpl>>}
        impl<'s> Heap<'_,'s> {
            fn enqueue(&mut self, selector:&'s Selector<SelectorImpl>) {
                self.pending.try_reserve(1).unwrap_or_else(|_| std::alloc::handle_alloc_error(
                    std::alloc::Layout::new::<&Selector<SelectorImpl>>()));
                self.pending.push(selector);
            }
            fn add_bytes(&mut self, bytes:usize) {
                self.bytes = self.bytes.checked_add(bytes).expect("native selector retained-byte overflow");
            }
            fn measure<T:malloc_size_of::MallocSizeOf + ?Sized>(&mut self, value:&T) {
                let bytes = value.size_of(self.ops); self.add_bytes(bytes);
            }
            fn allocation<T:?Sized>(&mut self,ptr:*const T)->bool {
                if self.ops.have_seen_ptr(ptr as *const std::ffi::c_void){return false;}
                // The accessor returns the allocation base, not a slice interior.
                let bytes = unsafe{self.ops.malloc_size_of(ptr)};
                self.add_bytes(bytes);true
            }
            fn list(&mut self,list:&'s SelectorList<SelectorImpl>){
                if self.allocation(list.thin_arc_heap_ptr()) {
                    for selector in list.slice() { self.enqueue(selector); }
                }
            }
            fn selector(&mut self,selector:&'s Selector<SelectorImpl>){
                if self.allocation(selector.thin_arc_heap_ptr()){
                    for component in selector.iter_raw_match_order(){self.component(component);}
                }
            }
            fn component(&mut self,component:&'s Component<SelectorImpl>) {
                use selectors::attr::{NamespaceConstraint,ParsedAttrSelectorOperation};
                match component {
                    Component::LocalName(name)=>{self.measure(&name.name);self.measure(&name.lower_name);},
                    Component::ID(value)|Component::Class(value)=>self.measure(value),
                    Component::AttributeInNoNamespaceExists{local_name,local_name_lower}=>{self.measure(local_name);self.measure(local_name_lower);},
                    Component::AttributeInNoNamespace{local_name,value,..}=>{self.measure(local_name);self.measure(value);},
                    Component::DefaultNamespace(url)=>self.measure(url),
                    Component::Namespace(prefix,url)=>{self.measure(prefix);self.measure(url);},
                    Component::AttributeOther(attribute)=>{
                        if self.allocation(&**attribute as *const _){
                            if let Some(NamespaceConstraint::Specific((prefix,url)))=&attribute.namespace{
                                self.measure(prefix);self.measure(url);
                            }
                            self.measure(&attribute.local_name);self.measure(&attribute.local_name_lower);
                            if let ParsedAttrSelectorOperation::WithValue{value,..}=&attribute.operation{self.measure(value);}
                        }
                    }
                    Component::Negation(list)|Component::Is(list)|Component::Where(list)=>self.list(list),
                    Component::NthOf(data)=>{
                        if self.allocation(data.thin_arc_heap_ptr()) {
                            for selector in data.selectors() { self.enqueue(selector); }
                        }
                    }
                    Component::Has(relative)=>{
                        // Box<[T]> data pointer is the Box allocation base; not
                        // a selector Arc interior. Empty dangling boxes skip.
                        if !relative.is_empty() && self.allocation(&**relative as *const _) {
                            for entry in relative.iter() { self.enqueue(&entry.selector); }
                        }
                    }
                    Component::Slotted(selector)|Component::Host(Some(selector))=>self.enqueue(selector),
                    Component::NonTSPseudoClass(NonTSPseudoClass::HasSlotted(Some(argument)))=>{
                        self.list(&argument.0);
                    }
                    Component::NonTSPseudoClass(pseudo)=>self.measure(pseudo),
                    Component::PseudoElement(pseudo)=>self.measure(pseudo),
                    Component::Part(names)=>{if !names.is_empty() && self.allocation(&**names as *const _){for name in names.iter(){self.measure(name);}}}
                    Component::Invalid(value)=>{if self.allocation(value.heap_ptr()){self.measure(value.as_ref());}}
                    Component::ExplicitUniversalType|Component::ExplicitAnyNamespace|
                    Component::ExplicitNoNamespace|Component::Root|Component::Empty|
                    Component::Scope|Component::ImplicitScope|Component::ParentSelector|
                    Component::Nth(_)|Component::Host(None)|Component::Combinator(_)|
                    Component::RelativeSelectorAnchor=>{}
                }
            }
        }
        let mut heap=Heap{ops,bytes:0,pending:Vec::new()};
        heap.list(&self.0);
        while let Some(selector) = heap.pending.pop() { heap.selector(selector); }
        // Reporting scratch is freed, not a retained-byte or V8 grant claim.
        heap.bytes
    }
}
