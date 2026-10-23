// Private layout-input ownership; no public LayoutInput contract extension.
struct PublicFontProjection {
    input: oryn_layout::LayoutInput,
    _allocation: Option<Arc<dyn Send + Sync>>,
}

impl LiveLayoutProvider {
    fn append_public_font_projection(
        &self,
        input: &mut oryn_layout::LayoutInput,
        document: NodeId,
    ) -> Result<Option<Arc<dyn Send + Sync>>, String> {
        let identity = self.native_document_identity.get().ok_or("font projection owner retired")?;
        if identity.document != document { return Err("foreign font projection owner".into()); }
        let revision = self.font_revision.get();
        let (count, bytes) = {
            let fonts = self.public_fonts.borrow();
            let mut bytes = input.web_fonts.len().checked_add(fonts.len())
                .and_then(|n| n.checked_mul(std::mem::size_of::<oryn_layout::WebFont>()))
                .ok_or("font projection capacity overflow")?;
            bytes = bytes.checked_add(fonts.len().checked_mul(std::mem::size_of::<(
                oryn_js::v8_bindings::font_loading_transport::FontLoadToken,
                Weak<dyn oryn_js::v8_bindings::font_loading::NativeFontFace>, bool,
            )>()).ok_or("font projection identity overflow")?).ok_or("font projection identity overflow")?;
            for entry in fonts.iter() {
                for length in [entry.font.descriptors.len(), entry.font.family.len(),
                    entry.font.source_url.len(), entry.font.sha256.len(), entry.font.bytes.len()] {
                    bytes = bytes.checked_add(length).ok_or("font projection payload overflow")?;
                }
            }
            (fonts.len(), bytes)
        };
        if count == 0 { return Ok(None); }
        let admission = self.shared_retained_admission.borrow().as_ref().cloned()
            .ok_or("font projection admission absent")?;
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        if self.native_document_identity.get() != Some(identity) || self.font_revision.get() != revision {
            return Err("font projection changed during admission".into());
        }
        let target = input.web_fonts.len().checked_add(count).ok_or("font projection capacity overflow")?;
        let mut witnesses = Vec::new();
        witnesses.try_reserve_exact(count).map_err(|_| "font projection identity allocation")?;
        if witnesses.capacity() > count { return Err("font projection identities exceeded admission".into()); }
        {
            let fonts = self.public_fonts.borrow();
            if fonts.len() != count || self.font_revision.get() != revision { return Err("font projection source changed".into()); }
            for entry in fonts.iter() { witnesses.push((entry.token, entry.current.clone(), false)); }
        }
        // Native face revision queries run without any provider-store borrow.
        for (token, weak, current) in &mut witnesses {
            *current = weak.upgrade().is_some_and(|face| face.revision() == token.revision);
        }
        if self.native_document_identity.get() != Some(identity) || self.font_revision.get() != revision {
            return Err("font projection changed during identity checks".into());
        }
        // Retain the incumbent Vec until replacement capacity is admitted.
        let mut replacement = Vec::new();
        replacement.try_reserve_exact(target).map_err(|_| "font projection allocation")?;
        if replacement.capacity() > target { return Err("font projection exceeded admission".into()); }
        replacement.append(&mut input.web_fonts);
        {
            let fonts = self.public_fonts.borrow();
            if fonts.len() != count || self.font_revision.get() != revision {
                return Err("font projection source changed".into());
            }
            for (entry, (token, _, current)) in fonts.iter().zip(&witnesses) {
                if entry.token != *token { return Err("font projection identity replaced".into()); }
                if entry.token.owner.document != document
                    || entry.token.owner.generation != identity.document_generation
                    || entry.token.owner.epoch != 0
                    || !current { continue; }
                let copy_text = |source: &str| -> Result<String, String> {
                    let mut text = String::new();
                    text.try_reserve_exact(source.len()).map_err(|_| "font text projection allocation")?;
                    if text.capacity() > source.len() { return Err("font text projection exceeded admission".into()); }
                    text.push_str(source);
                    Ok(text)
                };
                let font = &entry.font;
                let mut payload = Vec::new();
                payload.try_reserve_exact(font.bytes.len()).map_err(|_| "font byte projection allocation")?;
                if payload.capacity() > font.bytes.len() { return Err("font byte projection exceeded admission".into()); }
                payload.extend_from_slice(&font.bytes);
                replacement.push(oryn_layout::WebFont {
                    pending: font.pending, blocked: font.blocked, order: font.order,
                    descriptors: copy_text(&font.descriptors)?, family: copy_text(&font.family)?,
                    source_url: copy_text(&font.source_url)?, sha256: copy_text(&font.sha256)?, bytes: payload,
                });
            }
        }
        admission.checkpoint()?;
        if self.native_document_identity.get() != Some(identity) || self.font_revision.get() != revision {
            return Err("font projection retired before publication".into());
        }
        input.web_fonts = replacement;
        Ok(Some(allocation))
    }
}
