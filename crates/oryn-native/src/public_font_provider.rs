// Root-only installation of native loading service. CSS-connected records are
// original native rule handles, not a reparse of published stylesheet strings.
struct NativeFontLoadingProvider {
    provider_identity:usize,
    provider:Weak<dyn oryn_js::v8_bindings::BindingLayoutProvider>,
    state:Weak<oryn_js::v8_bindings::V8BindingState>,
    dom:SharedDomRuntime,
    registry:Rc<RefCell<RetainedStylesheets>>,
    identity:ParserDocumentIdentity,
    admission:Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
    _metadata:Arc<dyn Send+Sync>,
    backing:ImplicitFontBackingStore,
}
struct PublicFontBase { value:String, _allocation:Arc<dyn Send+Sync> }
impl NativeFontLoadingProvider {
    fn live_base(&self)->Result<PublicFontBase,String>{
        let state=self.state.upgrade().ok_or("font base owner retired")?;
        let document=state.document_root();
        let generation=state.document_generation();let epoch=state.document_stream_epoch();
        if state.execution_retired()||document!=self.identity.document||generation!=self.identity.document_generation
            ||epoch!=self.identity.stream_epoch{return Err("font base owner retired".into());}
        let (cursor,bytes)={let dom=self.dom.borrow();let mut bytes=0usize;
            for (_,node) in dom.arena().iter(){
                bytes=bytes.checked_add(std::mem::size_of::<NodeId>()).ok_or("font base traversal overflow")?;
                for (_,value) in node.attributes.iter(){bytes=bytes.checked_add(value.len()).ok_or("font base input overflow")?;}
                if let Some(metadata)=node.document_metadata.as_ref(){
                    bytes=bytes.checked_add(metadata.url.as_ref().map_or(0,String::len))
                        .and_then(|n|n.checked_add(metadata.inherited_base_url.as_ref().map_or(0,String::len)))
                        .ok_or("font base metadata overflow")?;
                }
            }
            (dom.mutation_cursor(),bytes)};
        // Known borrowed inputs and traversal scratch only. Canonical URL
        // parser allocations remain covered by the existing native policy.
        let allocation=self.admission.reserve(bytes)?;
        if state.execution_retired()||state.document_root()!=document||state.document_generation()!=generation
            ||state.document_stream_epoch()!=epoch||self.dom.borrow().mutation_cursor()!=cursor{return Err("font base owner changed".into());}
        let value=state.native_document_base_url(true).map_err(|error|error.to_string())?;
        if state.execution_retired()||self.dom.borrow().mutation_cursor()!=cursor{return Err("font base changed during capture".into());}
        Ok(PublicFontBase{value,_allocation:allocation})
    }
    fn retained_base(&self,document:NodeId)->Result<PublicFontBase,String>{
        let state=self.state.upgrade().ok_or("font base owner retired")?;
        if state.execution_retired(){return Err("font base owner retired".into());}
        let (cursor,bytes)={let dom=self.dom.borrow();dom.arena().document_id(document).map_err(|error|error.to_string())?;
            let mut bytes=0usize;
            for (_,node) in dom.arena().iter(){
                bytes=bytes.checked_add(std::mem::size_of::<NodeId>()).ok_or("font base traversal overflow")?;
                for (_,value) in node.attributes.iter(){bytes=bytes.checked_add(value.len()).ok_or("font base input overflow")?;}
                if let Some(metadata)=node.document_metadata.as_ref(){
                    bytes=bytes.checked_add(metadata.url.as_ref().map_or(0,String::len))
                        .and_then(|n|n.checked_add(metadata.inherited_base_url.as_ref().map_or(0,String::len))).ok_or("font base metadata overflow")?;
                }
            }
            (dom.mutation_cursor(),bytes)};
        let allocation=self.admission.reserve(bytes)?;
        if state.execution_retired()||self.dom.borrow().mutation_cursor()!=cursor{return Err("font base owner changed".into());}
        let value=state.native_retained_document_base_url(document).map_err(|error|error.to_string())?;
        if state.execution_retired()||self.dom.borrow().mutation_cursor()!=cursor{return Err("font base changed during capture".into());}
        Ok(PublicFontBase{value,_allocation:allocation})
    }
}
impl oryn_js::v8_bindings::font_loading::NativeFontLoading for NativeFontLoadingProvider {
    fn same_provider(&self,identity:usize)->bool{
        self.state.upgrade().is_some_and(|state|!state.execution_retired()
            &&state.document_root()==self.identity.document
            &&state.document_generation()==self.identity.document_generation
            &&state.document_stream_epoch()==self.identity.stream_epoch)
        &&self.provider_identity==identity&&self.provider.upgrade().is_some_and(|provider|
            Rc::as_ptr(&provider) as *const () as usize==identity)
    }
    fn construct(&self,family:&str,source:oryn_js::v8_bindings::font_loading_transport::FontFaceSourceInput,descriptors:&[(String,String)])
        ->Result<Rc<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>,String>{
        let base=self.live_base()?;
        let pair_charge=self.admission.reserve(descriptors.len().checked_mul(std::mem::size_of::<(&str,&str)>())
            .ok_or("font descriptor metadata overflow")?)?;
        let mut pairs=Vec::new();pairs.try_reserve_exact(descriptors.len()).map_err(|error|error.to_string())?;
        if pairs.capacity()>descriptors.len(){return Err("font descriptor capacity exceeded admission".into());}
        pairs.extend(descriptors.iter().map(|(name,value)|(public_font_descriptor(name),value.as_str())));
        use oryn_js::v8_bindings::font_loading_transport::FontFaceSourceInput;
        use oryn_layout::cssom::native_import_graph::NativeConstructedFontFace;
        let (native,binary)=match source {
            FontFaceSourceInput::Css{text,units,allocation}=>{
                let parsed=NativeConstructedFontFace::parse(family,&text,&pairs,&base.value,self.admission.as_ref())?;
                drop(text);drop(units);drop(allocation);(parsed,None)
            },
            FontFaceSourceInput::Binary{bytes,allocation}=>(NativeConstructedFontFace::parse_binary(
                family,&pairs,&base.value,self.admission.as_ref())?,Some((bytes,allocation))),
        };
        let metadata=self.admission.reserve(std::mem::size_of::<PublicFontFace>()+2*std::mem::size_of::<usize>())?;
        drop(pair_charge);
        Ok(Rc::new(PublicFontFace{native:RefCell::new(native),admission:self.admission.clone(),binary,_metadata:metadata}))
    }
    fn connected(&self)->Result<oryn_js::v8_bindings::font_loading_transport::ConnectedFontFaces,String>{
        let count=self.registry.borrow().records.len();
        let records_charge=self.admission.reserve(count.checked_mul(std::mem::size_of::<Rc<RetainedStylesheetRecord>>())
            .ok_or("font publication snapshot overflow")?)?;
        let mut records=Vec::new();records.try_reserve_exact(count).map_err(|error|error.to_string())?;
        if records.capacity()>count{return Err("font publication capacity exceeded admission".into());}
        {let registry=self.registry.borrow();
            if registry.records.len()!=count{return Err("font publication snapshot changed".into());}
            records.extend(registry.records.iter().cloned());}
        let mut result=Vec::new();let mut result_allocation=None;
        for publication in records {
            if publication.owner.document!=self.identity || publication.owner_publication_retired().map_err(|e|e.to_string())? {continue;}
            let count=publication.graph.borrow().completion_context_count();
            let _charge=self.admission.reserve(count.checked_mul(std::mem::size_of::<oryn_layout::cssom::native_import_graph::NativeImport>()).ok_or("font snapshot overflow")?)?;
            let mut entries=Vec::new();entries.try_reserve_exact(count).map_err(|e|e.to_string())?;
            if entries.capacity()>count{return Err("font context capacity exceeded admission".into());}
            let context=publication.graph.borrow().completion_context(entries)?;
            let faces=context.font_demands(self.admission.as_ref())?;
            let target=result.len().checked_add(faces.faces.len()).ok_or("connected font capacity overflow")?;
            let allocation=self.admission.reserve(target.checked_mul(std::mem::size_of::<Rc<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>>())
                .ok_or("connected font metadata overflow")?)?;
            let mut replacement=Vec::new();replacement.try_reserve_exact(target).map_err(|error|error.to_string())?;
            if replacement.capacity()>target{return Err("connected font capacity exceeded admitted layout".into());}
            replacement.append(&mut result);
            drop(result);result=replacement;drop(result_allocation.take());result_allocation=Some(allocation);
            for face in faces.faces {
                let identity=publication.graph.borrow().connected_font_context(&face.rule)?.identity();
                let metadata=self.admission.reserve(std::mem::size_of::<ConnectedPublicFontFace>()+2*std::mem::size_of::<usize>())?;
                result.push(Rc::new(ConnectedPublicFontFace{identity,rule:face.rule,publication:publication.clone(),
                    admission:self.admission.clone(),_metadata:metadata,backing:self.backing.clone()}) as Rc<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>);
            }
        }
        drop(records_charge);
        Ok(oryn_js::v8_bindings::font_loading_transport::ConnectedFontFaces{faces:result,allocation:result_allocation})
    }
    fn select(&self,font:&str,text:&str,faces:&[Rc<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>])
        ->Result<oryn_js::v8_bindings::font_loading_transport::FontSelection,oryn_js::v8_bindings::font_loading_transport::FontSelectionError>{
        self.select_for_document(self.identity.document,font,text,faces)
    }
    fn select_for_document(&self,document:NodeId,font:&str,text:&str,faces:&[Rc<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>])
        ->Result<oryn_js::v8_bindings::font_loading_transport::FontSelection,oryn_js::v8_bindings::font_loading_transport::FontSelectionError>{
        use oryn_js::v8_bindings::font_loading_transport::FontSelectionError;
        let result=(||->Result<_,String>{
        let base=if document==self.identity.document{self.live_base()?}else{self.retained_base(document)?};
        let query=match oryn_layout::cssom::native_import_graph::parse_native_font_shorthand_typed(font,&base.value,self.admission.as_ref()){
            Ok(query)=>query,
            Err(oryn_layout::cssom::native_import_graph::NativeFontShorthandError::Syntax)=>return Ok(None),
            Err(oryn_layout::cssom::native_import_graph::NativeFontShorthandError::Resource(message))=>return Err(message),
        };
        let query_axes=query.axes()?;
        let allocation=self.admission.reserve(faces.len().checked_mul(std::mem::size_of::<usize>())
            .ok_or("font selection capacity overflow")?)?;
        let mut selected=Vec::new();
        selected.try_reserve_exact(faces.len()).map_err(|error|error.to_string())?;
        if selected.capacity()>faces.len(){return Err("font selection capacity exceeded admitted layout".into());}
        for family_index in 0..query.family_count() {
        self.admission.checkpoint()?;
        let segment_start=selected.len();
        let mut rank:Option<[f32;7]>=None;
        for (index,face) in faces.iter().enumerate() {
            let family=face.family()?;
            if !query.family_matches_at(family_index,&family.value) || !face.covers(text)? {continue;}
            let axes=face.axes()?;
            use oryn_layout::cssom::native_import_graph::{NativeFontAxes,NativeFontStyle,native_font_axes_rank};
            use oryn_js::v8_bindings::font_loading_transport::FontStyleAxes;
            let candidate=native_font_axes_rank(NativeFontAxes{
                width:axes.width,weight:axes.weight,style:match axes.style{
                    FontStyleAxes::Normal=>NativeFontStyle::Normal,
                    FontStyleAxes::Italic=>NativeFontStyle::Italic,
                    FontStyleAxes::Oblique(min,max)=>NativeFontStyle::Oblique(min,max),
                },
            },query_axes);
            match rank {
                None=>{rank=Some(candidate);if !selected.contains(&index){selected.push(index);}}
                Some(old) if candidate<old=>{selected.truncate(segment_start);if !selected.contains(&index){selected.push(index);}rank=Some(candidate);}
                Some(old) if candidate==old=>{if !selected.contains(&index){selected.push(index);}},
                _=>{}
            }
        }
        }
        Ok(Some(oryn_js::v8_bindings::font_loading_transport::FontSelection{indices:selected,allocation}))
        })();
        match result{Ok(Some(selection))=>Ok(selection),Ok(None)=>Err(FontSelectionError::Syntax),Err(message)=>Err(FontSelectionError::Resource(message))}
    }
}
