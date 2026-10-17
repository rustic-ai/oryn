impl LiveLayoutProvider {
    fn install_retained_font(
        &self, document: NodeId, owner: NodeId, revision: u64,
        font: oryn_js::v8_bindings::BindingFontResource,
        custody: Arc<dyn std::any::Any + Send + Sync>,
        epoch: dynamic_stylesheet_preparation::RetainedFontDemandEpoch,
    ) -> Result<(), String> {
        if !epoch.current() { return Ok(()); }
        let next = self.font_revision.get().checked_add(1)
            .ok_or_else(|| "native font revision exhausted".to_owned())?;
        let font = oryn_layout::WebFont {pending:font.pending,blocked:font.blocked,
            order:font.order,descriptors:font.descriptors,family:font.family,
            source_url:font.source_url,sha256:font.sha256,bytes:font.bytes};
        let mut fonts = self.fonts.try_borrow_mut().map_err(|_|"native font custody borrow conflict")?;
        fonts.retain(|(_,_,_,_,_,epoch)| epoch.as_ref().is_none_or(
            dynamic_stylesheet_preparation::RetainedFontDemandEpoch::current));
        let replaces = fonts.iter().any(|(doc,node,source,old,_,old_epoch)|
            *doc == document && *node == owner && *source == revision && old.order == font.order
                && old_epoch.as_ref().is_some_and(|old_epoch| old_epoch.same(&epoch)));
        if !replaces && fonts.len().checked_add(self.public_fonts.borrow().len()).ok_or("native layout font count overflow")? >= NATIVE_LAYOUT_FONT_CAPACITY { return Err("native layout font capacity".into()); }
        fonts.try_reserve(1).map_err(|_|"native layout font allocation")?;
        if !epoch.current() { return Ok(()); }
        fonts.retain(|(doc,node,source,old,_,old_epoch)|
            !(*doc == document && *node == owner && *source == revision && old.order == font.order
                && old_epoch.as_ref().is_some_and(|old_epoch| old_epoch.same(&epoch))));
        // Native identity/revision is captured before publication. No fallible
        // accounting/custody operation follows the commit.
        fonts.push((document,owner,revision,font,custody,Some(epoch)));
        self.font_revision.set(next);
        Ok(())
    }
}
