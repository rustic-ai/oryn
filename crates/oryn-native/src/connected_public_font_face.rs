// Adapter retains the original parsed occurrence, not a serialized stylesheet.
struct ConnectedPublicFontFace {
    identity:usize,
    rule:oryn_layout::cssom::native_import_graph::NativeRule,
    publication:Rc<RetainedStylesheetRecord>,
    admission:Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
    _metadata:Arc<dyn Send+Sync>,
    backing:ImplicitFontBackingStore,
}
impl ConnectedPublicFontFace {
    fn read_context(&self)->Result<oryn_layout::cssom::native_import_graph::NativeConnectedFontReadContext,String>{
        if self.publication.owner_publication_retired().map_err(|e|e.to_string())? {
            return Err("connected font publication retired".into());
        }
        self.publication.graph.borrow().connected_font_context(&self.rule)
    }
}
impl oryn_js::v8_bindings::font_loading::NativeFontFace for ConnectedPublicFontFace {
    fn decoded_backing(&self)->bool{
        // This proof requires an original face and an active exact publication;
        // the entry exists only after native decoding and admitted publication.
        self.backing.try_borrow().is_ok_and(|entries|entries.entries.iter().any(|entry|entry.matches(self.identity,&self.publication)))
    }
    fn binary_source(&self)->bool{false}
    fn initial_error(&self)->bool{false}
    fn identity(&self)->usize {self.identity}
    fn same_face(&self,other:&dyn oryn_js::v8_bindings::font_loading::NativeFontFace)->bool {
        self.identity()==other.identity()
    }
    fn revision(&self)->u64 {self.publication.graph.borrow().font_revision()}
    fn descriptor(&self,name:&str)->Result<oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue,String>{
        let context=self.read_context()?;
        let output=context.descriptor(public_font_descriptor(name),self.admission.as_ref())?;
        let (value,allocation)=output.into_parts();
        Ok(oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue{value,allocation})
    }
    fn family(&self)->Result<oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue,String>{
        let context=self.read_context()?;
        let (value,allocation)=context.family(self.admission.as_ref())?.into_parts();
        Ok(oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue{value,allocation})
    }
    fn axes(&self)->Result<oryn_js::v8_bindings::font_loading_transport::FontAxes,String>{
        use oryn_layout::cssom::native_import_graph::NativeFontStyle;
        use oryn_js::v8_bindings::font_loading_transport::{FontAxes,FontStyleAxes};
        let axes=self.read_context()?.axes()?;
        Ok(FontAxes{width:axes.width,weight:axes.weight,style:match axes.style{
            NativeFontStyle::Normal=>FontStyleAxes::Normal,
            NativeFontStyle::Italic=>FontStyleAxes::Italic,
            NativeFontStyle::Oblique(min,max)=>FontStyleAxes::Oblique(min,max),
        }})
    }
    fn covers(&self,text:&str)->Result<bool,String>{self.read_context()?.intersects_text(text)}
    fn set_descriptor(&self,name:&str,value:&str)->Result<bool,String>{
        let context=self.read_context()?;
        let prepared=context.prepare_descriptor(public_font_descriptor(name),value,self.admission.as_ref())?;
        let publication_revision=self.publication.mutation_revision.get();
        let next=publication_revision.checked_add(1).ok_or("font publication revision exhausted")?;
        let result={self.publication.graph.borrow_mut().commit_connected_font_descriptor(prepared)};
        match result {
            Ok(receipt)=>{
                let changed=receipt.changed;
                if changed {
                    self.publication.mutation_revision.set(next);
                }
                drop(receipt);
                Ok(changed)
            },
            Err(prepared)=>{drop(prepared);Err("connected font edit stale".into())},
        }
    }
    fn request(&self,token:oryn_js::v8_bindings::font_loading_transport::FontLoadToken,text:&str)
        ->Result<oryn_js::v8_bindings::font_loading_transport::FontLoadRequest,String>{
        let count=self.publication.graph.borrow().completion_context_count();
        let snapshot_charge=self.admission.reserve(count.checked_mul(std::mem::size_of::<oryn_layout::cssom::native_import_graph::NativeImport>())
            .ok_or("connected font snapshot overflow")?)?;
        let mut entries=Vec::new();entries.try_reserve_exact(count).map_err(|_|"connected font snapshot allocation")?;
        let context=self.publication.graph.borrow().completion_context(entries)?;
        let owned=context.connected_font_demand(&self.rule,self.admission.as_ref())?;
        let face=owned.face();
        let source_bytes=face.sources.iter().try_fold(0usize,|n,source|{
            use oryn_layout::cssom::native_import_graph::NativeFontSource;
            n.checked_add(match source {NativeFontSource::Url{url,..}=>url.len(),NativeFontSource::Local{name}=>name.len()})
        }).ok_or("connected font source overflow")?;
        let bytes=face.family.len().checked_add(face.base_url.len()).and_then(|n|n.checked_add(face.descriptors.view.css_text.len()))
            .and_then(|n|n.checked_add(text.len())).and_then(|n|n.checked_add(face.sources.len().checked_mul(std::mem::size_of::<oryn_js::v8_bindings::font_loading_transport::FontLoadSource>())?))
            .and_then(|n|n.checked_add(source_bytes))
            .ok_or("connected font request overflow")?;
        let allocation=self.admission.reserve(bytes)?;
        let mut sources=Vec::new();sources.try_reserve_exact(face.sources.len()).map_err(|_|"connected font sources allocation")?;
        for source in &face.sources {
            use oryn_layout::cssom::native_import_graph::NativeFontSource;
            sources.push(match source {
                NativeFontSource::Url{url,..}=>oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Url(url.clone()),
                NativeFontSource::Local{name}=>oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Local(name.clone()),
            });
        }
        let request=oryn_js::v8_bindings::font_loading_transport::FontLoadRequest{
            token,family:face.family.clone(),descriptors:face.descriptors.view.css_text.clone(),
            sources,base_url:face.base_url.clone(),text:text.to_owned(),allocation,
            source_allocation:snapshot_charge,
        };
        drop(owned);
        Ok(request)
    }
}
