struct PublicLayoutFont {
    token:oryn_js::v8_bindings::font_loading_transport::FontLoadToken,
    current:Weak<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>,
    font:oryn_layout::WebFont,
    _custody:Arc<dyn std::any::Any+Send+Sync>,
}
const NATIVE_LAYOUT_FONT_CAPACITY:usize=1024;
impl LiveLayoutProvider {
    fn install_public_font(
        &self,token:oryn_js::v8_bindings::font_loading_transport::FontLoadToken,
        current:Weak<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>,
        font:oryn_layout::WebFont,custody:Arc<dyn std::any::Any+Send+Sync>,
    )->Result<(),String>{
        let identity=self.native_document_identity.get().ok_or("public font document retired")?;
        if identity.document!=token.owner.document || identity.document_generation!=token.owner.generation
            || identity.stream_epoch!=token.owner.epoch {return Err("public font owner retired".into());}
        let next=self.font_revision.get().checked_add(1).ok_or("native font revision exhausted")?;
        let (length,capacity,replacing)={let fonts=self.public_fonts.borrow();(fonts.len(),fonts.capacity(),fonts.iter().any(|entry|entry.token.owner==token.owner&&entry.token.face==token.face))};
        if !replacing&&length.checked_add(self.fonts.borrow().len()).ok_or("native layout font count overflow")?>=NATIVE_LAYOUT_FONT_CAPACITY{return Err("native layout font capacity".into())}
        if length==capacity&&!replacing{
            let target=capacity.checked_mul(2).unwrap_or(NATIVE_LAYOUT_FONT_CAPACITY).max(1).min(NATIVE_LAYOUT_FONT_CAPACITY);
            if target<=capacity{return Err("native layout font capacity".into())}
            let admission=self.shared_retained_admission.borrow().as_ref().cloned().ok_or("font store admission absent")?;
            let charge=admission.reserve(target.checked_mul(std::mem::size_of::<PublicLayoutFont>()).ok_or("font store capacity overflow")?)?;
            let mut replacement=Vec::new();replacement.try_reserve_exact(target).map_err(|_|"font store capacity allocation")?;
            if replacement.capacity()>target{return Err("font store exceeded capacity admission".into())}
            if self.native_document_identity.get()!=Some(identity){return Err("public font owner retired".into())}
            let old={let mut fonts=self.public_fonts.borrow_mut();
                if fonts.len()!=length||fonts.capacity()!=capacity{return Err("font store changed during admission".into())}
                replacement.append(&mut fonts);*fonts=replacement;
                self.public_font_capacity.borrow_mut().replace(charge)
            };drop(old);
        }
        let mut fonts=self.public_fonts.borrow_mut();
        // The pending face itself retains data through the actual weak wrapper
        // graph; this store must not invent a strong provider/face cycle.
        // Publication-time pruning is performed before this mutation by the
        // owning binding checkpoint. No native face callback under this borrow.
        let replacing=fonts.iter().any(|entry|entry.token.owner==token.owner && entry.token.face==token.face);
        if !replacing && fonts.len().checked_add(self.fonts.borrow().len())
            .ok_or("native layout font count overflow")? >= NATIVE_LAYOUT_FONT_CAPACITY {return Err("native layout font capacity".into());}
        if fonts.len()==fonts.capacity()&&!replacing{return Err("native public font capacity".into())}
        let old=fonts.iter().position(|entry|entry.token.owner==token.owner && entry.token.face==token.face)
            .map(|index|fonts.remove(index));
        fonts.push(PublicLayoutFont{token,current,font,_custody:custody});
        drop(fonts);
        drop(old);
        self.font_revision.set(next);
        Ok(())
    }
}
