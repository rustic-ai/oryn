// Native provider adopts existing exact descriptor, shorthand and source
// algorithms. This is a pure adapter: network work remains in page font jobs.
struct PublicFontFace {
    native:RefCell<oryn_layout::cssom::native_import_graph::NativeConstructedFontFace>,
    admission:Rc<dyn oryn_layout::cssom::native_import_graph::ImportAdmission>,
    binary:Option<(Vec<u8>,Arc<dyn Send+Sync>)>,
    _metadata:Arc<dyn Send+Sync>,
}
impl oryn_js::v8_bindings::font_loading::NativeFontFace for PublicFontFace {
    fn decoded_backing(&self)->bool{false}
    fn binary_source(&self)->bool{self.binary.is_some()}
    fn initial_error(&self)->bool{self.native.borrow().initial_error.is_some()}
    fn same_face(&self,other:&dyn oryn_js::v8_bindings::font_loading::NativeFontFace)->bool {
        self.identity()==other.identity()
    }
    fn identity(&self)->usize {self.native.borrow().identity()}
    fn revision(&self)->u64 {self.native.borrow().revision()}
    fn descriptor(&self,name:&str)->Result<oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue,String> {
        let context=self.native.borrow().edit_context();
        let output=context.descriptor(public_font_descriptor(name),self.admission.as_ref())?;
        let (value,allocation)=output.into_parts();
        Ok(oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue{value,allocation})
    }
    fn set_descriptor(&self,name:&str,value:&str)->Result<bool,String>{
        let context=self.native.borrow().edit_context();
        let prepared=context.prepare(public_font_descriptor(name),value,self.admission.as_ref())?;
        let committed={self.native.borrow_mut().commit_descriptor(prepared)};
        match committed {
            Ok(receipt)=>{
                let changed=receipt.changed;
                drop(receipt);
                Ok(changed)
            },
            Err(prepared)=>{
                drop(prepared);
                Err("constructed descriptor edit stale".to_owned())
            },
        }
    }
    fn family(&self)->Result<oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue,String>{
        let context=self.native.borrow().edit_context();
        let (value,allocation)=context.family(self.admission.as_ref())?.into_parts();
        Ok(oryn_js::v8_bindings::font_loading_transport::FontDescriptorValue{value,allocation})
    }
    fn covers(&self,text:&str)->Result<bool,String>{
        let context=self.native.borrow().edit_context();
        context.intersects_text(text)
    }
    fn axes(&self)->Result<oryn_js::v8_bindings::font_loading_transport::FontAxes,String>{
        use oryn_layout::cssom::native_import_graph::NativeFontStyle;
        use oryn_js::v8_bindings::font_loading_transport::{FontAxes,FontStyleAxes};
        let context=self.native.borrow().edit_context();
        let axes=context.axes()?;
        Ok(FontAxes{weight:axes.weight,width:axes.width,style:match axes.style{
            NativeFontStyle::Normal=>FontStyleAxes::Normal,
            NativeFontStyle::Italic=>FontStyleAxes::Italic,
            NativeFontStyle::Oblique(min,max)=>FontStyleAxes::Oblique(min,max),
        }})
    }
    fn request(&self,token:oryn_js::v8_bindings::font_loading_transport::FontLoadToken,text:&str)
        ->Result<oryn_js::v8_bindings::font_loading_transport::FontLoadRequest,String>{
        use oryn_layout::cssom::native_import_graph::NativeFontSource;
        let context=self.native.borrow().edit_context();
        let demand=context.demand(self.admission.as_ref())?;
        let source_allocation=demand.allocation();
        let allocation=self.admission.reserve(demand.family.len().checked_add(demand.descriptors.len())
            .and_then(|n|n.checked_add(demand.base_url.len())).and_then(|n|n.checked_add(text.len()))
            .and_then(|n|n.checked_add(demand.sources.len().checked_add(usize::from(self.binary.is_some()))?.checked_mul(std::mem::size_of::<oryn_js::v8_bindings::font_loading_transport::FontLoadSource>())?))
            .and_then(|n|n.checked_add(self.binary.as_ref().map_or(0,|(bytes,_)|bytes.len())))
            .ok_or("public font request size overflow")?)?;
        let mut sources=Vec::new();
        sources.try_reserve_exact(demand.sources.len()+usize::from(self.binary.is_some())).map_err(|_|"public font source allocation")?;
        let mut owned_text=String::new();
        owned_text.try_reserve_exact(text.len()).map_err(|_|"public font text allocation")?;
        owned_text.push_str(text);
        for source in demand.sources {
            sources.push(match source {
                NativeFontSource::Url{url,..}=>oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Url(url),
                NativeFontSource::Local{name}=>oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Local(name),
            });
        }
        if let Some((bytes,_))=&self.binary {
            let mut owned=Vec::new();
            owned.try_reserve_exact(bytes.len()).map_err(|_|"public binary font source allocation")?;
            owned.extend_from_slice(bytes);
            sources.push(oryn_js::v8_bindings::font_loading_transport::FontLoadSource::Bytes(owned));
        }
        Ok(oryn_js::v8_bindings::font_loading_transport::FontLoadRequest{
            token,family:demand.family,descriptors:demand.descriptors,sources,base_url:demand.base_url,
            text:owned_text,allocation,source_allocation})
    }
}
fn public_font_descriptor(name:&str)->&str{
    match name {
        "family"=>"font-family","style"=>"font-style","weight"=>"font-weight","stretch"=>"font-stretch",
        "unicodeRange"=>"unicode-range","featureSettings"=>"font-feature-settings","variationSettings"=>"font-variation-settings",
        "display"=>"font-display","ascentOverride"=>"ascent-override","descentOverride"=>"descent-override",
        "lineGapOverride"=>"line-gap-override","sizeAdjust"=>"size-adjust",_=>name,
    }
}
