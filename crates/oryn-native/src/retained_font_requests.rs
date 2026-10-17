// Included in dynamic_stylesheet_preparation. All callbacks run after dropping
// the live graph/registry borrow; the original typed snapshot owns its handles.
struct RetainedFontRequests {
    epoch: Option<RetainedFontDemandEpoch>,
    entries: Vec<(
        String,
        String,
        String,
        oryn_layout::cssom::CssResourceReference,
        FontCandidate,
    )>,
    _snapshot: Option<oryn_layout::cssom::native_import_graph::NativeFontDemandSnapshot>,
    _allocation: Arc<dyn Send + Sync>,
}
fn retained_font_requests(
    host: &mut RawV8Host,
    realm: RealmId,
    batch: &CssDependencyBatch,
    dom: &SharedDomRuntime,
) -> Result<Option<RetainedFontRequests>, HostError> {
    use oryn_layout::cssom::native_import_graph::{NativeFontSource, NativeImport};
    let identity = ParserFetchContext::capture(host, dom, batch.node)?.identity();
    if identity.realm != realm {
        return Err(HostError::CachedAuthorityExpired);
    }
    let Some(epoch) = retained_font_epoch(host, identity, dom, batch.node, batch.revision)? else {
        return Ok(None);
    };
    let _admission = host
        .native_retained_admission_for_realm(realm)?
        .ok_or_else(|| HostError::Execution("font descriptor admission unavailable".into()))?;
    let _scope = host.retained_allocation_scope(realm)?;
    let admission = epoch.publication.admission.clone();
    let count = epoch
        .publication
        .graph
        .try_borrow()
        .map_err(|_| HostError::Execution("font capture graph borrow conflict".into()))?
        .completion_context_count();
    let capture_charge = admission
        .reserve(
            count
                .checked_mul(std::mem::size_of::<NativeImport>())
                .ok_or_else(|| HostError::Execution("font capture capacity overflow".into()))?,
        )
        .map_err(HostError::Execution)?;
    let mut imports = Vec::new();
    imports
        .try_reserve_exact(count)
        .map_err(|_| HostError::Execution("font capture allocation".into()))?;
    let context = epoch
        .publication
        .graph
        .try_borrow()
        .map_err(|_| HostError::Execution("font capture graph borrow conflict".into()))?
        .completion_context(imports)
        .map_err(HostError::Execution)?;
    let snapshot = context
        .font_demands(admission.as_ref())
        .map_err(HostError::Execution)?;
    if snapshot.font_revision != epoch.font_revision || !epoch.current() {
        return Err(HostError::CachedAuthorityExpired);
    }
    let mut bytes = 0usize;
    let mut request_count = 0usize;
    {
        let responses =
            epoch.publication.responses.try_borrow().map_err(|_| {
                HostError::Execution("font response ownership borrow conflict".into())
            })?;
        for face in &snapshot.faces {
            let policy = responses
                .entries
                .iter()
                .find(|response| {
                    response
                        .sheet
                        .as_ref()
                        .is_some_and(|sheet| sheet.same_sheet(&face.sheet))
                })
                .ok_or_else(|| {
                    HostError::Execution("font face response ownership missing".into())
                })?;
            request_count = request_count
                .checked_add(face.sources.len())
                .ok_or_else(|| HostError::Execution("font source count overflow".into()))?;
            for source in &face.sources {
                let source_bytes = match source {
                    NativeFontSource::Url { url, .. } => url.len(),
                    NativeFontSource::Local { name } => name
                        .len()
                        .checked_add("oryn-local-font:".len())
                        .ok_or_else(|| HostError::Execution("local font URL overflow".into()))?,
                };
                bytes = bytes
                    .checked_add(face.base_url.len())
                    .and_then(|n| n.checked_add(policy.final_url.len()))
                    .and_then(|n| n.checked_add(policy.referrer_policy.len()))
                    .and_then(|n| n.checked_add(face.family.len()))
                    .and_then(|n| n.checked_add(face.descriptors.view.css_text.len()))
                    .and_then(|n| n.checked_add(face.unicode_ranges.len().max(1).checked_mul(8)?))
                    .and_then(|n| n.checked_add(source_bytes))
                    .and_then(|n| n.checked_add(128)) // bounded hexadecimal identities + decimal revisions
                    .ok_or_else(|| HostError::Execution("font request metadata overflow".into()))?;
            }
        }
    }
    let charge = admission
        .reserve(
            bytes
                .checked_add(
                    request_count
                        .checked_mul(std::mem::size_of::<(
                            String,
                            String,
                            String,
                            oryn_layout::cssom::CssResourceReference,
                            FontCandidate,
                        )>())
                        .ok_or_else(|| {
                            HostError::Execution("font requests capacity overflow".into())
                        })?,
                )
                .ok_or_else(|| HostError::Execution("font request allocation overflow".into()))?,
        )
        .map_err(HostError::Execution)?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(request_count)
        .map_err(|_| HostError::Execution("font requests allocation".into()))?;
    for (order, face) in snapshot.faces.iter().enumerate() {
        let mut applicable = true;
        for condition in &face.conditions {
            let condition = condition
                .font_condition(admission.as_ref())
                .map_err(HostError::Execution)?;
            if !condition.supports_matches {
                applicable = false;
                break;
            }
            if let Some(query) = &condition.media_text {
                if !host.native_media_matches(query)? {
                    applicable = false;
                    break;
                }
            }
        }
        for import in &face.imports {
            let condition = import
                .font_condition(admission.as_ref())
                .map_err(HostError::Execution)?;
            if !host.native_media_matches(&condition.media_text)? || !condition.supports_matches {
                applicable = false;
                break;
            }
        }
        if !applicable || face.family.is_empty() {
            continue;
        }
        let (initiator, policy) = {
            let responses = epoch.publication.responses.try_borrow().map_err(|_| {
                HostError::Execution("font response ownership borrow conflict".into())
            })?;
            let response = responses
                .entries
                .iter()
                .find(|response| {
                    response
                        .sheet
                        .as_ref()
                        .is_some_and(|sheet| sheet.same_sheet(&face.sheet))
                })
                .ok_or_else(|| {
                    HostError::Execution("font face response ownership changed".into())
                })?;
            (response.final_url.clone(), response.referrer_policy.clone())
        };
        let key = format!(
            "native:{:x}:{:x}:{}:{}",
            Rc::as_ptr(&epoch.publication) as usize,
            face.rule
                .native_font_identity()
                .ok_or_else(|| HostError::Execution("font rule identity lost".into()))?,
            epoch.mutation_revision,
            epoch.font_revision
        );
        for source in &face.sources {
            let url = match source {
                NativeFontSource::Url { url, format, tech } => {
                    if !native_font_source_supported(format, *tech) {
                        continue;
                    }
                    url.clone()
                }
                NativeFontSource::Local { name } => {
                    if oryn_layout::local_font_bytes(name).is_none() {
                        continue;
                    }
                    format!("oryn-local-font:{name}")
                }
            };
            entries.push((
                face.base_url.clone(),
                initiator.clone(),
                policy.clone(),
                oryn_layout::cssom::CssResourceReference {
                    url,
                    destination: oryn_layout::cssom::CssResourceDestination::Font,
                },
                FontCandidate {
                    native_rule: Some(face.rule.clone()),
                    family: face.family.clone(),
                    key: key.clone(),
                    ranges: if face.unicode_ranges.is_empty() {
                        vec![(0, 0x10ffff)]
                    } else {
                        face.unicode_ranges.clone()
                    },
                    descriptors: face.descriptors.view.css_text.clone(),
                    order: order as u64,
                    retained: Some(epoch.clone()),
                },
            ));
        }
    }
    admission.checkpoint().map_err(HostError::Execution)?;
    if !epoch.current() {
        return Err(HostError::CachedAuthorityExpired);
    }
    drop(capture_charge);
    Ok(Some(RetainedFontRequests {
        epoch: Some(epoch),
        entries,
        _snapshot: Some(snapshot),
        _allocation: charge,
    }))
}
fn native_font_source_supported(
    format: &Option<oryn_layout::cssom::native_import_graph::NativeFontSourceFormat>,
    tech: u16,
) -> bool {
    // Uses the same native format/tech filter as the existing font source path.
    oryn_layout::cssom::native_import_graph::native_font_source_supported(format.as_ref(), tech)
}

fn legacy_font_requests(
    host: &mut RawV8Host,
    batch: &CssDependencyBatch,
) -> Result<RetainedFontRequests, HostError> {
    let mut entries = Vec::new();
    for (dependency_index, (base, initiator, policy, css)) in batch.dependencies.iter().enumerate()
    {
        for (reference, mut candidate) in font_requests(host, css, base)? {
            candidate.order = candidate
                .order
                .checked_add(
                    (dependency_index as u64)
                        .checked_mul(8192)
                        .ok_or_else(|| HostError::Execution("font source order overflow".into()))?,
                )
                .ok_or_else(|| HostError::Execution("font source order overflow".into()))?;
            entries.push((
                base.clone(),
                initiator.clone(),
                policy.clone(),
                reference,
                candidate,
            ));
        }
    }
    Ok(RetainedFontRequests {
        epoch: None,
        entries,
        _snapshot: None,
        _allocation: Arc::new(()),
    })
}
