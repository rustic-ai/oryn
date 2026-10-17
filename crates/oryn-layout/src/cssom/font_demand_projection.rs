// Original typed font demand projection. Included in native_import_graph.
pub type NativeFontSourceFormat = style::font_face::FontFaceSourceFormat;
pub fn native_font_source_supported(format: Option<&NativeFontSourceFormat>, tech: u16) -> bool {
    use style::font_face::{
        FontFaceSourceFormat as F, FontFaceSourceFormatKeyword as K, FontFaceSourceTechFlags as T,
    };
    let format_ok = match format {
        None => true,
        Some(F::Keyword(k)) => matches!(k, K::Opentype | K::Truetype | K::Woff | K::Woff2),
        Some(F::String(s)) => ["opentype", "truetype", "woff", "woff2"]
            .iter()
            .any(|name| s.eq_ignore_ascii_case(name)),
    };
    format_ok && tech & !(T::FEATURES_OPENTYPE | T::VARIATIONS).bits() == 0
}
impl NativeRule {
    /// Original retained FontFace Arc only; strong demand ownership prevents ABA.
    pub fn native_font_identity(&self) -> Option<usize> {
        match &self.rule {
            CssRule::FontFace(rule) => Some((&**rule as *const _) as usize),
            _ => None,
        }
    }
}
pub struct NativeFontRuleCondition {
    pub media_text: Option<String>,
    pub supports_matches: bool,
    _allocation: LeaseArc<dyn Send + Sync>,
}
impl NativeRule {
    pub fn font_condition(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeFontRuleCondition, String> {
        use style_traits::ToCss;
        let media = match &self.rule {
            CssRule::Media(rule) => Some(rule.media_queries.clone()),
            _ => None,
        };
        let bytes = if let Some(media) = &media {
            let guard = self.owner.shared_lock().read();
            let mut count = FontDescriptorCount::default();
            media
                .read_with(&guard)
                .to_css(&mut style_traits::CssWriter::new(&mut count))
                .map_err(|e| e.to_string())?;
            count.bytes
        } else {
            0
        };
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut text = String::new();
        text.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
        let guard = self.owner.shared_lock().read();
        if let Some(media) = &media {
            media
                .read_with(&guard)
                .to_css(&mut style_traits::CssWriter::new(&mut FontBoundedWriter {
                    text: &mut text,
                    limit: bytes,
                }))
                .map_err(|e| e.to_string())?;
        }
        let supports_matches = match &self.rule {
            CssRule::Supports(rule) => rule.enabled,
            _ => true,
        };
        Ok(NativeFontRuleCondition {
            media_text: media.map(|_| text),
            supports_matches,
            _allocation: allocation,
        })
    }
}
pub struct NativeFontImportCondition {
    pub media_text: String,
    pub supports_text: Option<String>,
    pub supports_matches: bool,
    _allocation: LeaseArc<dyn Send + Sync>,
}
impl NativeImport {
    pub fn font_condition(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeFontImportCondition, String> {
        use style_traits::ToCss;
        let (media_bytes, support_bytes) = {
            let guard = self.parent.shared_lock().read();
            let mut media = FontDescriptorCount::default();
            self.media
                .read_with(&guard)
                .to_css(&mut style_traits::CssWriter::new(&mut media))
                .map_err(|e| e.to_string())?;
            let mut supports = FontDescriptorCount::default();
            if let Some(s) = &self.rule.read_with(&guard).supports {
                s.condition
                    .to_css(&mut style_traits::CssWriter::new(&mut supports))
                    .map_err(|e| e.to_string())?;
            }
            (media.bytes, supports.bytes)
        };
        let allocation = admission.reserve(
            media_bytes
                .checked_add(support_bytes)
                .ok_or("font import condition overflow")?,
        )?;
        admission.checkpoint()?;
        let mut media_text = String::new();
        media_text
            .try_reserve_exact(media_bytes)
            .map_err(|e| e.to_string())?;
        let mut supports = String::new();
        supports
            .try_reserve_exact(support_bytes)
            .map_err(|e| e.to_string())?;
        let guard = self.parent.shared_lock().read();
        self.media
            .read_with(&guard)
            .to_css(&mut style_traits::CssWriter::new(&mut FontBoundedWriter {
                text: &mut media_text,
                limit: media_bytes,
            }))
            .map_err(|e| e.to_string())?;
        let import = self.rule.read_with(&guard);
        if let Some(s) = &import.supports {
            s.condition
                .to_css(&mut style_traits::CssWriter::new(&mut FontBoundedWriter {
                    text: &mut supports,
                    limit: support_bytes,
                }))
                .map_err(|e| e.to_string())?;
        }
        Ok(NativeFontImportCondition {
            media_text,
            supports_text: import.supports.as_ref().map(|_| supports),
            supports_matches: import.supports.as_ref().is_none_or(|s| s.enabled),
            _allocation: allocation,
        })
    }
}
pub enum NativeFontSource {
    Url {
        url: String,
        format: Option<style::font_face::FontFaceSourceFormat>,
        tech: u16,
    },
    Local {
        name: String,
    },
}
pub struct NativeFontDemand {
    pub rule: NativeRule,
    pub sheet: NativeSheet,
    pub base_url: String,
    pub family: String,
    pub descriptors: NativeDeclarationView,
    pub sources: Vec<NativeFontSource>,
    pub unicode_ranges: Vec<(u32, u32)>,
    pub conditions: Vec<NativeRule>,
    pub imports: Vec<NativeImport>,
    _allocation: LeaseArc<dyn Send + Sync>,
}
pub struct NativeFontDemandSnapshot {
    pub font_revision: u64,
    pub faces: Vec<NativeFontDemand>,
    _allocations: Vec<LeaseArc<dyn Send + Sync>>,
}
struct FontCaptureAdmission(LeaseArc<dyn Send + Sync>);
impl ImportAdmission for FontCaptureAdmission {
    fn checkpoint(&self) -> Result<(), String> {
        Ok(())
    }
    fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
        Ok(self.0.clone())
    }
}
impl ImportGraph {
    pub fn font_revision(&self) -> u64 {
        self.font_revision
            .load(std::sync::atomic::Ordering::Acquire)
    }
    fn project_font_demand(
        &self,
        rule: NativeRule,
        imports: Vec<NativeImport>,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeFontDemand, String> {
        let original = self.font_face_rule(&rule)?;
        let revision = self.font_revision();
        let descriptors = self.font_face_declaration(&rule, "", admission)?;
        let (bytes, source_count, range_count) = {
            let guard = self.lock.read();
            let d = &original.read_with(&guard).descriptors;
            let mut bytes = d.font_family.as_ref().map_or(0, |f| f.name.len());
            for source in d.src.as_ref().map_or(&[][..], |s| &s.0[..]) {
                bytes = bytes
                    .checked_add(match source {
                        style::font_face::Source::Url(u) => u
                            .url
                            .url()
                            .map_or(0, |url| url.as_str().len())
                            .checked_add(match &u.format_hint {
                                Some(style::font_face::FontFaceSourceFormat::String(s)) => s.len(),
                                _ => 0,
                            })
                            .ok_or("font format bytes overflow")?,
                        style::font_face::Source::Local(n) => n.name.len(),
                    })
                    .ok_or("font source bytes overflow")?;
            }
            bytes = bytes
                .checked_add(rule.owner.sheet.contents(&guard).url_data.as_str().len())
                .ok_or("font base overflow")?;
            (
                bytes,
                d.src.as_ref().map_or(0, |s| s.0.len()),
                d.unicode_range.as_ref().map_or(0, Vec::len),
            )
        };
        let bytes = bytes
            .checked_add(
                source_count
                    .checked_mul(std::mem::size_of::<NativeFontSource>())
                    .ok_or("font source count overflow")?,
            )
            .and_then(|n| {
                n.checked_add(range_count.checked_mul(std::mem::size_of::<(u32, u32)>())?)
            })
            .and_then(|n| {
                n.checked_add(
                    rule.ancestor_len
                        .checked_mul(std::mem::size_of::<NativeRule>())?,
                )
            })
            .ok_or("font demand metadata overflow")?;
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(source_count)
            .map_err(|e| e.to_string())?;
        let mut unicode_ranges = Vec::new();
        unicode_ranges
            .try_reserve_exact(range_count)
            .map_err(|e| e.to_string())?;
        let mut conditions = Vec::new();
        conditions
            .try_reserve_exact(rule.ancestor_len)
            .map_err(|e| e.to_string())?;
        let guard = self.lock.read();
        if self.font_revision() != revision {
            return Err("font demand changed during admission".into());
        }
        let d = &original.read_with(&guard).descriptors;
        let copy = |s: &str| -> Result<String, String> {
            let mut out = String::new();
            out.try_reserve_exact(s.len()).map_err(|e| e.to_string())?;
            out.push_str(s);
            Ok(out)
        };
        let family = copy(d.font_family.as_ref().map_or("", |f| f.name.as_ref()))?;
        let base_url = copy(rule.owner.sheet.contents(&guard).url_data.as_str())?;
        for source in d.src.as_ref().map_or(&[][..], |s| &s.0[..]) {
            sources.push(match source {
                style::font_face::Source::Url(u) => {
                    let Some(resolved) = u.url.url() else {
                        continue;
                    };
                    let format = match &u.format_hint {
                        None => None,
                        Some(style::font_face::FontFaceSourceFormat::Keyword(k)) => {
                            Some(style::font_face::FontFaceSourceFormat::Keyword(*k))
                        }
                        Some(style::font_face::FontFaceSourceFormat::String(s)) => {
                            Some(style::font_face::FontFaceSourceFormat::String(copy(s)?))
                        }
                    };
                    NativeFontSource::Url {
                        url: copy(resolved.as_str())?,
                        format,
                        tech: u.tech_flags.bits(),
                    }
                }
                style::font_face::Source::Local(n) => NativeFontSource::Local {
                    name: copy(n.name.as_ref())?,
                },
            });
        }
        if let Some(ranges) = &d.unicode_range {
            for r in ranges {
                unicode_ranges.push((r.start, r.end));
            }
        }
        drop(guard);
        let mut parent = rule.parent_rule();
        while let Some(p) = parent {
            parent = p.parent_rule();
            conditions.push(p);
        }
        conditions.reverse();
        Ok(NativeFontDemand {
            sheet: rule.owner.clone(),
            rule,
            base_url,
            family,
            descriptors,
            sources,
            unicode_ranges,
            conditions,
            imports,
            _allocation: allocation,
        })
    }
}
impl ImportCompletionContext {
    pub fn font_demands(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeFontDemandSnapshot, String> {
        let graph = &self.snapshot;
        let revision = graph.font_revision();
        let mut out = NativeFontDemandSnapshot {
            font_revision: revision,
            faces: Vec::new(),
            _allocations: Vec::new(),
        };
        // The caller owns this snapshot, not a borrowed live registry. No
        // admission callback runs under the style lock; acquisition adapters
        // retain only already admitted ancestor metadata.
        let mut stack: Vec<(
            NativeSheet,
            Option<NativeRule>,
            usize,
            Vec<NativeImport>,
            Vec<NativeRule>,
        )> = Vec::new();
        let charge = admission.reserve(
            std::mem::size_of_val(&stack)
                + std::mem::size_of::<(
                    NativeSheet,
                    Option<NativeRule>,
                    usize,
                    Vec<NativeImport>,
                    Vec<NativeRule>,
                )>()
                + std::mem::size_of::<LeaseArc<dyn Send + Sync>>(),
        )?;
        stack.try_reserve_exact(1).map_err(|e| e.to_string())?;
        stack.push((graph.root(), None, 0, Vec::new(), Vec::new()));
        out._allocations
            .try_reserve_exact(1)
            .map_err(|e| e.to_string())?;
        out._allocations.push(charge);
        let mut seen = 0usize;
        while let Some((sheet, parent, index, imports, inherited)) = stack.pop() {
            admission.checkpoint()?;
            let count = match &parent {
                Some(p) => graph.group_rule_count(p).map_err(|e| format!("{e:?}"))?,
                None => graph.rule_count(&sheet)?,
            };
            if index >= count {
                continue;
            }
            seen = seen.checked_add(1).ok_or("font traversal overflow")?;
            // Same existing native font_requests traversal bound.
            if seen > 8192 {
                return Err("font face rule traversal limit exceeded".into());
            }
            let ancestors = match &parent {
                None => 0,
                Some(p) => p
                    .ancestor_len
                    .checked_add(1)
                    .ok_or("font ancestors overflow")?,
            };
            let bytes = ancestors
                .checked_mul(std::mem::size_of::<RuleAncestor>())
                .ok_or("font ancestor overflow")?;
            let charge = admission.reserve(bytes)?;
            admission.checkpoint()?;
            let capture = FontCaptureAdmission(charge);
            let rule = match &parent {
                Some(p) => graph.group_rule_at(p, index, &capture),
                None => graph.rule_at(&sheet, &[], index, &capture),
            }
            .map_err(|e| format!("{e:?}"))?;
            let import_capacity = imports
                .len()
                .checked_add(1)
                .ok_or("font imports overflow")?;
            let condition_capacity = inherited
                .len()
                .checked_add(ancestors)
                .ok_or("font condition count overflow")?;
            let grow = admission.reserve(
                2usize
                    .checked_mul(std::mem::size_of::<(
                        NativeSheet,
                        Option<NativeRule>,
                        usize,
                        Vec<NativeImport>,
                        Vec<NativeRule>,
                    )>())
                    .and_then(|n| n.checked_add(std::mem::size_of::<NativeFontDemand>()))
                    .and_then(|n| {
                        n.checked_add(2 * std::mem::size_of::<LeaseArc<dyn Send + Sync>>())
                    })
                    .and_then(|n| {
                        n.checked_add(
                            import_capacity.checked_mul(std::mem::size_of::<NativeImport>())?,
                        )
                    })
                    .and_then(|n| {
                        n.checked_add(
                            condition_capacity
                                .checked_mul(2 * std::mem::size_of::<NativeRule>())?,
                        )
                    })
                    .ok_or("font walk overflow")?,
            )?;
            stack.try_reserve_exact(2).map_err(|e| e.to_string())?;
            out.faces.try_reserve_exact(1).map_err(|e| e.to_string())?;
            out._allocations
                .try_reserve_exact(1)
                .map_err(|e| e.to_string())?;
            out._allocations.push(grow);
            let mut next_imports = Vec::new();
            next_imports
                .try_reserve_exact(import_capacity)
                .map_err(|e| e.to_string())?;
            next_imports.extend(imports.iter().cloned());
            let mut next_conditions = Vec::new();
            next_conditions
                .try_reserve_exact(condition_capacity)
                .map_err(|e| e.to_string())?;
            next_conditions.extend(inherited.iter().cloned());
            let next = index.checked_add(1).ok_or("font list index overflow")?;
            stack.push((sheet.clone(), parent, next, imports, inherited));
            if matches!(&rule.rule, CssRule::FontFace(_)) {
                let mut face = graph.project_font_demand(rule, next_imports, admission)?;
                next_conditions.extend(face.conditions.drain(..));
                face.conditions = next_conditions;
                out.faces.push(face);
            } else if let Some(import) = graph.import_for_rule(&rule) {
                if import.state == ImportState::Complete {
                    if let Some(child) = &import.child {
                        if next_imports.len() >= graph.max_depth {
                            return Err("font import traversal depth exceeded".into());
                        }
                        let child = child.clone();
                        let inherited_len = next_conditions.len();
                        let mut parent = rule.parent_rule();
                        while let Some(p) = parent {
                            parent = p.parent_rule();
                            next_conditions.push(p);
                        }
                        next_conditions[inherited_len..].reverse();
                        next_imports.push(import);
                        stack.push((child, None, 0, next_imports, next_conditions));
                    }
                }
            } else if graph.group_rule_count(&rule).is_ok() {
                stack.push((sheet, Some(rule), 0, next_imports, next_conditions));
            }
        }
        if graph.font_revision() != revision {
            return Err("font demand snapshot changed".into());
        }
        Ok(out)
    }
}
#[cfg(test)]
mod native_font_demand_controls {
    use super::*;
    struct Admission;
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Ok(LeaseArc::new(()))
        }
    }
    fn snapshot(graph: &ImportGraph) -> ImportCompletionContext {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(graph.completion_context_count())
            .unwrap();
        graph.completion_context(entries).unwrap()
    }
    #[test]
    fn font_demands_preserve_native_identity_and_descriptor_revision() {
        let admission = Admission;
        let mut graph=ImportGraph::parse("@supports(color:red){@font-face{font-family:Demand;src:local(Demand),url(face.woff2) format(woff2);unicode-range:U+41-5A}}", "https://example.test/css/root.css",false,8,SharedRwLock::new(),&admission).unwrap();
        let first = snapshot(&graph).font_demands(&admission).unwrap();
        assert_eq!(first.faces.len(), 1);
        assert_eq!(first.faces[0].family, "Demand");
        assert_eq!(first.faces[0].unicode_ranges, vec![(0x41, 0x5A)]);
        assert_eq!(first.faces[0].conditions.len(), 1);
        assert_eq!(first.faces[0].base_url, "https://example.test/css/root.css");
        assert!(
            matches!(&first.faces[0].sources[0],NativeFontSource::Local{name} if name=="Demand")
        );
        assert!(
            matches!(&first.faces[0].sources[1],NativeFontSource::Url{url,..} if url=="https://example.test/css/face.woff2")
        );
        graph
            .set_rule_property(
                &first.faces[0].rule,
                "font-family",
                "Revised",
                "",
                &admission,
            )
            .unwrap();
        let second = snapshot(&graph).font_demands(&admission).unwrap();
        assert!(second.font_revision > first.font_revision);
        assert!(first.faces[0].rule.same_rule(&second.faces[0].rule));
        assert_eq!(second.faces[0].family, "Revised");
        assert_eq!(first.faces[0].family, "Demand");
    }
    #[test]
    fn font_demand_admission_failure_preserves_graph() {
        struct Refuse;
        impl ImportAdmission for Refuse {
            fn checkpoint(&self) -> Result<(), String> {
                Ok(())
            }
            fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
                Err("injected font projection failure".into())
            }
        }
        let graph = ImportGraph::parse(
            "@font-face{font-family:Demand;src:url(a.woff2)}",
            "https://example.test/",
            false,
            8,
            SharedRwLock::new(),
            &Admission,
        )
        .unwrap();
        let revision = graph.font_revision();
        assert!(snapshot(&graph).font_demands(&Refuse).is_err());
        assert_eq!(graph.font_revision(), revision);
        assert_eq!(
            snapshot(&graph).font_demands(&Admission).unwrap().faces[0].family,
            "Demand"
        );
    }
    #[test]
    fn font_demands_keep_import_conditions_and_original_child_base() {
        let admission = Admission;
        let mut graph = ImportGraph::parse(
            "@import 'nested/child.css' screen;",
            "https://example.test/root.css",
            false,
            8,
            SharedRwLock::new(),
            &admission,
        )
        .unwrap();
        let import = graph.occurrence(OccurrenceId(1)).unwrap();
        graph
            .complete(
                import.id,
                "@supports(color:red){@font-face{font-family:Child;src:url(face.woff2)}}",
                "https://example.test/nested/child.css",
                &admission,
            )
            .unwrap();
        let demands = snapshot(&graph).font_demands(&admission).unwrap();
        assert_eq!(demands.faces.len(), 1);
        let face = &demands.faces[0];
        assert_eq!(face.imports.len(), 1);
        assert!(face.imports[0].same_rule(&import));
        assert_eq!(face.conditions.len(), 1);
        assert_eq!(face.base_url, "https://example.test/nested/child.css");
        assert!(
            matches!(&face.sources[0],NativeFontSource::Url{url,..} if url=="https://example.test/nested/face.woff2")
        );
        assert!(
            face.sheet
                .same_sheet(&graph.occurrence(import.id).unwrap().child.unwrap())
        );
    }
}
