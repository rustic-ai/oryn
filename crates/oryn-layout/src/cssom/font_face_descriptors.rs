// Typed FontFace descriptor operations; included in native_import_graph.
// Parser-internal allocation is the existing opaque Stylo boundary; no deep
// clone or source-length upper-bound claim is made here.
#[derive(Default)]
struct FontDescriptorCount {
    bytes: usize,
}
impl std::fmt::Write for FontDescriptorCount {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.bytes = self.bytes.checked_add(text.len()).ok_or(std::fmt::Error)?;
        Ok(())
    }
}
struct FontBoundedWriter<'a> {
    text: &'a mut String,
    limit: usize,
}
impl std::fmt::Write for FontBoundedWriter<'_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let next = self
            .text
            .len()
            .checked_add(value.len())
            .ok_or(std::fmt::Error)?;
        if next > self.limit || next > self.text.capacity() {
            return Err(std::fmt::Error);
        }
        self.text.push_str(value);
        Ok(())
    }
}
fn font_descriptor_count(
    descriptors: &style::font_face::Descriptors,
) -> Result<FontDescriptorCount, String> {
    use style_traits::ToCss;
    let mut count = FontDescriptorCount::default();
    descriptors
        .to_css(&mut style_traits::CssWriter::new(&mut count))
        .map_err(|error| error.to_string())?;
    Ok(count)
}
fn font_projected_text<W: std::fmt::Write>(
    original: &style::font_face::Descriptors,
    staged: &style::font_face::Descriptors,
    replacement: Option<style::font_face::DescriptorId>,
    out: &mut W,
) -> Result<(), String> {
    use std::fmt::Write;
    for index in 0..style::font_face::DescriptorId::COUNT {
        let id =
            style::font_face::DescriptorId::at(index).ok_or("native descriptor index absent")?;
        let value = if replacement == Some(id) {
            staged
        } else {
            original
        };
        let mut count = FontDescriptorCount::default();
        value
            .get_with_writer(id, &mut count)
            .map_err(|error| error.to_string())?;
        if count.bytes != 0 {
            out.write_str(id.name())
                .map_err(|error| error.to_string())?;
            out.write_str(": ").map_err(|error| error.to_string())?;
            value
                .get_with_writer(id, out)
                .map_err(|error| error.to_string())?;
            out.write_str("; ").map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
impl ImportGraph {
    fn font_face_rule(
        &self,
        retained: &NativeRule,
    ) -> Result<Arc<Locked<style::font_face::FontFaceRule>>, String> {
        if !LeaseArc::ptr_eq(&retained.owner.leases, &self.leases) {
            return Err("foreign native rule".into());
        }
        match &retained.rule {
            CssRule::FontFace(rule) => Ok(rule.clone()),
            _ => Err("native rule is not a font-face rule".into()),
        }
    }
    fn font_face_declaration(
        &self,
        retained: &NativeRule,
        property: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeDeclarationView, String> {
        use style::font_face::DescriptorId;
        use style_traits::ToCss;
        admission.checkpoint()?;
        let original = self.font_face_rule(retained)?;
        let revision = self
            .font_revision
            .load(std::sync::atomic::Ordering::Acquire);
        let id = DescriptorId::from_ident(property).ok();
        let (count, value_bytes, names_bytes, names_count) = {
            let guard = self.lock.read();
            let descriptors = &original.read_with(&guard).descriptors;
            let count = font_descriptor_count(descriptors)?;
            let mut value = FontDescriptorCount::default();
            if let Some(id) = id {
                descriptors
                    .get_with_writer(id, &mut value)
                    .map_err(|e| e.to_string())?;
            }
            let mut names = 0usize;
            for index in 0..descriptors.len() {
                names = names
                    .checked_add(
                        descriptors
                            .at(index)
                            .ok_or("descriptor index absent")?
                            .name()
                            .len(),
                    )
                    .ok_or("descriptor names overflow")?;
            }
            (count, value.bytes, names, descriptors.len())
        };
        let bytes = count
            .bytes
            .checked_add(value_bytes)
            .and_then(|n| n.checked_add(names_bytes))
            .and_then(|n| n.checked_add(names_count.checked_mul(std::mem::size_of::<String>())?))
            .ok_or("font descriptor view overflow")?;
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut view = super::DeclarationView {
            css_text: String::new(),
            names: Vec::new(),
            value: String::new(),
            important: false,
        };
        view.css_text
            .try_reserve_exact(count.bytes)
            .map_err(|e| e.to_string())?;
        view.value
            .try_reserve_exact(value_bytes)
            .map_err(|e| e.to_string())?;
        view.names
            .try_reserve_exact(names_count)
            .map_err(|e| e.to_string())?;
        let guard = self.lock.read();
        let descriptors = &original.read_with(&guard).descriptors;

        if self
            .font_revision
            .load(std::sync::atomic::Ordering::Acquire)
            != revision
        {
            return Err("font descriptors changed during admission".into());
        }
        descriptors
            .to_css(&mut style_traits::CssWriter::new(&mut FontBoundedWriter {
                text: &mut view.css_text,
                limit: count.bytes,
            }))
            .map_err(|e| e.to_string())?;
        view.css_text.truncate(view.css_text.trim_end().len());
        if let Some(id) = id {
            descriptors
                .get_with_writer(
                    id,
                    &mut FontBoundedWriter {
                        text: &mut view.value,
                        limit: value_bytes,
                    },
                )
                .map_err(|e| e.to_string())?;
        }
        for index in 0..names_count {
            let name = descriptors
                .at(index)
                .ok_or("descriptor index absent")?
                .name();
            let mut text = String::new();
            text.try_reserve_exact(name.len())
                .map_err(|e| e.to_string())?;
            text.push_str(name);
            view.names.push(text);
        }
        drop(guard);
        admission.checkpoint()?;
        if self
            .font_revision
            .load(std::sync::atomic::Ordering::Acquire)
            != revision
        {
            return Err("font descriptors changed after read".into());
        }
        Ok(NativeDeclarationView { view, allocation })
    }
    fn mutate_font_face(
        &mut self,
        retained: &NativeRule,
        edit: FontFaceDescriptorEdit<'_>,
        admission: &dyn ImportAdmission,
    ) -> Result<super::DeclarationMutation, String> {
        use style::font_face::{DescriptorId, Descriptors};
        use style_traits::ToCss;
        admission.checkpoint()?;
        let original = self.font_face_rule(retained)?;
        let revision = self
            .font_revision
            .load(std::sync::atomic::Ordering::Acquire);
        let next_revision = revision
            .checked_add(1)
            .ok_or("font descriptor revision exhausted")?;
        let location = {
            let guard = self.lock.read();
            let rule = original.read_with(&guard);
            rule.source_location
        };
        // UrlExtraData is an Arc handle; this clones no URL string payload.
        let base = retained
            .owner
            .sheet
            .contents(&self.lock.read())
            .url_data
            .clone();
        let context = style::parser::ParserContext::new(
            Origin::Author,
            &base,
            Some(style::stylesheets::CssRuleType::FontFace),
            style_traits::ParsingMode::DEFAULT,
            self.quirks,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        let mut staged = Descriptors::default();
        let mut result = super::DeclarationMutation {
            previous_value: String::new(),
            css_text: None,
        };
        let replacement = match edit {
            FontFaceDescriptorEdit::Text(text) => {
                let mut input = cssparser::ParserInput::new(text);
                let mut parser = cssparser::Parser::new(&mut input);
                staged = style::font_face::parse_font_face_block(&context, &mut parser, location)
                    .descriptors;
                None
            }
            FontFaceDescriptorEdit::Property {
                name,
                value,
                priority,
            } => {
                let Ok(id) = DescriptorId::from_ident(name) else {
                    return Ok(result);
                };
                if !priority.is_empty() {
                    return Ok(result);
                }
                if !value.is_empty() {
                    let mut input = cssparser::ParserInput::new(value);
                    let mut parser = cssparser::Parser::new(&mut input);
                    if staged.set(id, &context, &mut parser).is_err() {
                        return Ok(result);
                    }
                }
                Some(id)
            }
        };
        let (output_bytes, old_value_bytes, changed) = {
            let guard = self.lock.read();
            let descriptors = &original.read_with(&guard).descriptors;

            if self
                .font_revision
                .load(std::sync::atomic::Ordering::Acquire)
                != revision
            {
                return Err("font descriptors changed during parse".into());
            }
            let mut projected = FontDescriptorCount::default();
            let mut old_value = FontDescriptorCount::default();
            if let Some(id) = replacement {
                font_projected_text(descriptors, &staged, Some(id), &mut projected)?;
                descriptors
                    .get_with_writer(id, &mut old_value)
                    .map_err(|e| e.to_string())?;
                (
                    projected.bytes,
                    old_value.bytes,
                    !descriptors.same_one(id, &staged),
                )
            } else {
                staged
                    .to_css(&mut style_traits::CssWriter::new(&mut projected))
                    .map_err(|e| e.to_string())?;
                (projected.bytes, 0, staged != *descriptors)
            }
        };
        let lease_bytes = {
            let leases = self.leases.lock().map_err(|_| "import ledger poisoned")?;
            leases
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(std::mem::size_of::<LeaseArc<dyn Send + Sync>>()))
                .ok_or("font lease capacity overflow")?
        };
        let lease = admission.reserve(
            output_bytes
                .checked_add(old_value_bytes)
                .and_then(|n| n.checked_add(lease_bytes))
                .ok_or("font output admission overflow")?,
        )?;
        admission.checkpoint()?;
        let mut serialized = String::new();
        serialized
            .try_reserve_exact(output_bytes)
            .map_err(|e| e.to_string())?;
        result
            .previous_value
            .try_reserve_exact(old_value_bytes)
            .map_err(|e| e.to_string())?;
        {
            let guard = self.lock.read();
            let descriptors = &original.read_with(&guard).descriptors;

            if self
                .font_revision
                .load(std::sync::atomic::Ordering::Acquire)
                != revision
            {
                return Err("font descriptors changed during admission".into());
            }
            if let Some(id) = replacement {
                descriptors
                    .get_with_writer(
                        id,
                        &mut FontBoundedWriter {
                            text: &mut result.previous_value,
                            limit: old_value_bytes,
                        },
                    )
                    .map_err(|e| e.to_string())?;
                font_projected_text(
                    descriptors,
                    &staged,
                    Some(id),
                    &mut FontBoundedWriter {
                        text: &mut serialized,
                        limit: output_bytes,
                    },
                )?;
            } else {
                staged
                    .to_css(&mut style_traits::CssWriter::new(&mut FontBoundedWriter {
                        text: &mut serialized,
                        limit: output_bytes,
                    }))
                    .map_err(|e| e.to_string())?;
            }
        }
        if !changed {
            return Ok(result);
        }
        serialized.truncate(serialized.trim_end().len());
        admission.checkpoint()?;
        let mut leases = self.leases.lock().map_err(|_| "import ledger poisoned")?;
        leases.try_reserve_exact(1).map_err(|e| e.to_string())?;
        let mut guard = self.lock.write();
        let current = original.write_with(&mut guard);

        if self
            .font_revision
            .load(std::sync::atomic::Ordering::Acquire)
            != revision
        {
            return Err("font descriptors changed before commit".into());
        }
        if let Some(id) = replacement {
            current.descriptors.swap_one(id, &mut staged);
        } else {
            std::mem::swap(&mut current.descriptors, &mut staged);
        }
        leases.push(lease);
        self.font_revision
            .store(next_revision, std::sync::atomic::Ordering::Release);
        drop(guard);
        drop(leases);
        result.css_text = Some(serialized);
        Ok(result)
    }
}
enum FontFaceDescriptorEdit<'a> {
    Text(&'a str),
    Property {
        name: &'a str,
        value: &'a str,
        priority: &'a str,
    },
}

#[cfg(test)]
mod native_font_face_descriptor_tests {
    use super::*;
    struct Budget;
    impl ImportAdmission for Budget {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Ok(LeaseArc::new(()))
        }
    }
    struct Refused;
    impl ImportAdmission for Refused {
        fn checkpoint(&self) -> Result<(), String> {
            Err("retired".into())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Err("retired".into())
        }
    }
    struct ReserveRefused;
    impl ImportAdmission for ReserveRefused {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Err("allocation denied".into())
        }
    }
    #[test]
    fn descriptor_writer_refuses_growth_before_append() {
        use std::fmt::Write;
        let mut text = String::with_capacity(4);
        text.push_str("old");
        let mut writer = FontBoundedWriter {
            text: &mut text,
            limit: 4,
        };
        assert!(writer.write_str("xx").is_err());
        assert_eq!(text, "old");
    }
    #[test]
    fn typed_font_descriptors_preserve_original_rule_and_relative_source() {
        let budget = Budget;
        let mut graph = ImportGraph::parse(
            "@font-face { font-family: Test; src: url(font.woff2); font-weight: bold; }",
            "https://example.test/fonts/root.css",
            false,
            8,
            SharedRwLock::new(),
            &budget,
        )
        .unwrap();
        let root = graph.root();
        let rule = graph.rule_at(&root, &[], 0, &budget).unwrap();
        let held = graph.rule_at(&root, &[], 0, &budget).unwrap();
        assert_eq!(
            graph
                .rule_declaration(&rule, "font-family", &budget)
                .unwrap()
                .view
                .value,
            "Test"
        );
        assert_eq!(
            graph
                .rule_declaration(&rule, "src", &budget)
                .unwrap()
                .view
                .value,
            "url(\"font.woff2\")"
        );
        let changed = graph
            .set_rule_property(&rule, "font-weight", "400 700", "", &budget)
            .unwrap();
        assert!(changed.css_text.is_some());
        assert!(held.same_rule(&rule));
        assert_eq!(
            graph
                .rule_declaration(&held, "font-weight", &budget)
                .unwrap()
                .view
                .value,
            "400 700"
        );
        assert!(
            graph
                .set_rule_property(&rule, "src", "invalid(", "", &budget)
                .unwrap()
                .css_text
                .is_none()
        );
        assert!(
            graph
                .set_rule_property(&rule, "font-weight", "900", "important", &budget)
                .unwrap()
                .css_text
                .is_none()
        );
        assert_eq!(
            graph
                .rule_declaration(&held, "font-weight", &budget)
                .unwrap()
                .view
                .value,
            "400 700"
        );
        let removed = graph
            .set_rule_property(&rule, "font-weight", "", "", &budget)
            .unwrap();
        assert_eq!(removed.previous_value, "400 700");
        assert_eq!(
            graph
                .rule_declaration(&held, "font-weight", &budget)
                .unwrap()
                .view
                .value,
            ""
        );
        graph
            .set_rule_declaration_text(
                &rule,
                "src: local(\"Other\"); font-family: Other; invalid: value;",
                &budget,
            )
            .unwrap();
        let view = graph.rule_declaration(&held, "src", &budget).unwrap().view;
        assert_eq!(view.value, "local(\"Other\")");
        assert!(view.names.contains(&"src".to_owned()));
        assert!(view.names.contains(&"font-family".to_owned()));
        assert!(!view.names.contains(&"invalid".to_owned()));
        assert!(held.same_rule(&rule));
        assert!(!held.view().unwrap().css_text.contains('\n'));
        let before = held.view().unwrap().css_text;
        assert!(
            graph
                .set_rule_property(&rule, "font-family", "Denied", "", &Refused)
                .is_err()
        );
        assert_eq!(held.view().unwrap().css_text, before);
        assert!(
            graph
                .rule_declaration(&held, "src", &ReserveRefused)
                .is_err()
        );
        assert!(
            graph
                .set_rule_property(&held, "font-family", "Denied", "", &ReserveRefused)
                .is_err()
        );
        assert_eq!(held.view().unwrap().css_text, before);
        graph
            .set_rule_declaration_text(&held, "src: local(Test);", &budget)
            .unwrap();
        graph
            .set_rule_property(&held, "font-family", "Added", "", &budget)
            .unwrap();
        let canonical = graph.rule_declaration(&held, "", &budget).unwrap().view;
        assert_eq!(canonical.names, vec!["font-family", "src"]);
        assert_eq!(canonical.css_text, "font-family: Added; src: local(Test);");
    }
}
