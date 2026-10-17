//! Dynamic CSS IO is owned by the executable, with native revision/realm
//! publication and shared byte custody. No network wait runs on the page actor.
use super::*;
use std::time::Instant;
include!("retained_font_demand_epoch.rs");
include!("retained_font_requests.rs");
include!("implicit_font_backing_finish.rs");
#[cfg(test)]
thread_local! {
    static DYNAMIC_RETAINED_SCHEDULE_FAILURE: Cell<bool> = const { Cell::new(false) };
}
#[cfg(test)]
pub(super) fn fail_next_dynamic_retained_schedule() {
    DYNAMIC_RETAINED_SCHEDULE_FAILURE.with(|flag| flag.set(true));
}
const RESOURCE_ATTRIBUTES: [&str; 11] = [
    "style",
    "background",
    "fill",
    "stroke",
    "filter",
    "clip-path",
    "mask",
    "mask-image",
    "marker-start",
    "marker-mid",
    "marker-end",
];
fn resource_attributes(dom: &SharedDomRuntime, node: NodeId) -> Option<Vec<Option<String>>> {
    let dom = dom.borrow();
    let record = dom.arena().get(node).ok()?;
    Some(
        RESOURCE_ATTRIBUTES
            .iter()
            .map(|name| record.attributes.get_null(name).cloned())
            .collect(),
    )
}
#[derive(Clone)]
struct FontCandidate {
    native_rule: Option<oryn_layout::cssom::native_import_graph::NativeRule>,
    family: String,
    key: String,
    ranges: Vec<(u32, u32)>,
    descriptors: String,
    order: u64,
    retained: Option<RetainedFontDemandEpoch>,
}
fn font_requests(
    host: &mut RawV8Host,
    css: &str,
    base: &str,
) -> Result<Vec<(oryn_layout::cssom::CssResourceReference, FontCandidate)>, HostError> {
    let mut rules = oryn_layout::cssom::read_stylesheet(css, base, false, true)
        .map_err(HostError::Execution)?;
    rules.reverse();
    let mut seen = 0usize;
    let mut requests = Vec::new();
    while let Some(rule) = rules.pop() {
        seen += 1;
        if seen > 8192 {
            return Err(HostError::LimitExceeded {
                resource: "font face rule traversal".into(),
            });
        }
        if rule.rule_type == 4
            && let Some(query) = rule.properties.get("conditionText")
            && !host.native_media_matches(query)?
        {
            continue;
        }
        if rule.rule_type == 12
            && rule
                .properties
                .get("matches")
                .is_some_and(|value| value == "false")
        {
            continue;
        }
        if rule.rule_type == 5
            && let Some(text) = rule.declaration_text
        {
            let family =
                oryn_layout::cssom::read_font_face_declaration(&text, "font-family", base, false)
                    .map_err(HostError::Execution)?
                    .value;
            if let Some(family) = oryn_layout::cssom::font_family_names(&family)
                .into_iter()
                .next()
            {
                use sha2::Digest;
                let key = format!(
                    "{:x}",
                    sha2::Sha256::digest(format!("{base}\n{}", rule.css_text).as_bytes())
                );
                let ranges = oryn_layout::cssom::font_face_unicode_ranges(&text, base)
                    .map_err(HostError::Execution)?;
                for source in oryn_layout::cssom::font_face_sources(&text, base)
                    .map_err(HostError::Execution)?
                {
                    let url = match source {
                        oryn_layout::cssom::FontFaceSource::Url(url) => url,
                        oryn_layout::cssom::FontFaceSource::Local(name) => {
                            if oryn_layout::local_font_bytes(&name).is_none() {
                                continue;
                            }
                            format!("oryn-local-font:{name}")
                        }
                    };
                    requests.push((
                        oryn_layout::cssom::CssResourceReference {
                            url,
                            destination: oryn_layout::cssom::CssResourceDestination::Font,
                        },
                        FontCandidate {
                            native_rule: None,
                            family: family.clone(),
                            key: key.clone(),
                            ranges: ranges.clone(),
                            descriptors: text.clone(),
                            order: seen as u64,
                            retained: None,
                        },
                    ));
                }
            }
        }
        rules.extend(rule.children.into_iter().rev());
    }
    Ok(requests)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontDisplayPhase {
    Block,
    Swap,
    Failure,
    Loaded,
}
#[derive(Clone, Copy)]
struct FontDisplayPeriods {
    block: Duration,
    swap: Option<Duration>,
}
impl FontDisplayPeriods {
    fn parse(descriptors: &str) -> Self {
        let mode = oryn_layout::cssom::read_font_face_declaration(
            descriptors,
            "font-display",
            "https://oryn.invalid/",
            false,
        )
        .map(|view| view.value)
        .unwrap_or_default();
        match mode.as_str() {
            "swap" => Self {
                block: Duration::ZERO,
                swap: None,
            },
            "fallback" => Self {
                block: Duration::from_millis(100),
                swap: Some(Duration::from_secs(3)),
            },
            "optional" => Self {
                block: Duration::from_millis(100),
                swap: Some(Duration::ZERO),
            },
            _ => Self {
                block: Duration::from_secs(3),
                swap: None,
            },
        }
    }
    fn phase(self, elapsed: Duration, loaded: bool) -> FontDisplayPhase {
        if loaded {
            return FontDisplayPhase::Loaded;
        }
        if elapsed < self.block {
            return FontDisplayPhase::Block;
        }
        if self
            .swap
            .is_some_and(|swap| elapsed >= self.block.saturating_add(swap))
        {
            FontDisplayPhase::Failure
        } else {
            FontDisplayPhase::Swap
        }
    }
}
pub(super) struct CssFontTimeline {
    implicit_tokens: Option<oryn_js::v8_bindings::font_loading_transport::ImplicitFontTokens>,
    implicit_started: bool,
    implicit_terminal: Option<bool>,
    identity: ParserDocumentIdentity,
    dom: SharedDomRuntime,
    node: NodeId,
    revision: u64,
    font: FontCandidate,
    started: Instant,
    periods: FontDisplayPeriods,
    loaded: bool,
    failed: bool,
    published: Option<FontDisplayPhase>,
    _node: ParserScriptLease,
    _bytes: ScriptSourceReservation,
}
impl CssFontTimeline {
    fn valid(&self, host: &RawV8Host) -> bool {
        self.font
            .retained
            .as_ref()
            .is_none_or(RetainedFontDemandEpoch::current)
            && host.automation_realm_identity(self.identity.realm)
                == Some((
                    self.identity.document,
                    self.identity.document_generation,
                    self.identity.stream_epoch,
                    true,
                ))
            && self.dom.borrow().node_document(self.node).ok().flatten()
                == Some(self.identity.document)
            && self.dom.borrow().is_connected(self.node).unwrap_or(false)
            && self
                .dom
                .borrow()
                .arena()
                .stylesheet_revision(self.node)
                .ok()
                == Some(self.revision)
    }
    fn phase(&self, now: Instant) -> FontDisplayPhase {
        if self.failed {
            return FontDisplayPhase::Failure;
        }
        self.periods
            .phase(now.saturating_duration_since(self.started), self.loaded)
    }
    fn delay(&self, now: Instant) -> Option<Duration> {
        let phase = self.phase(now);
        if self.published != Some(phase) {
            return Some(Duration::ZERO);
        }
        let deadline = match phase {
            FontDisplayPhase::Block => self.periods.block,
            FontDisplayPhase::Swap => self.periods.block.saturating_add(self.periods.swap?),
            _ => return None,
        };
        Some(
            self.started
                .checked_add(deadline)?
                .saturating_duration_since(now),
        )
    }
}
pub(super) struct CssDemandBatch {
    retry: Option<Instant>,
    evaluated: Option<oryn_dom::MutationCursor>,
    evaluated_font: Option<RetainedFontDemandEpoch>,
    realm: RealmId,
    batch: CssDependencyBatch,
    attempted: BTreeSet<(String, Option<String>)>,
    identity: ParserDocumentIdentity,
    dom: SharedDomRuntime,
    _node: ParserScriptLease,
}
impl CssDemandBatch {
    pub(super) fn realm(&self) -> RealmId { self.realm }
    pub(super) fn evaluated(&self) -> bool { self.evaluated.is_some() }
    pub(super) fn evaluation_snapshot(&self) -> (NodeId, u64, Option<RetainedFontDemandEpoch>) {
        (self.batch.node, self.batch.revision, self.evaluated_font.clone())
    }
}
struct CssDemand {
    families: BTreeMap<String, BTreeSet<u32>>,
}
impl CssDemand {
    fn from_layout(layout: &oryn_js::v8_bindings::BindingLayout) -> Self {
        if !layout.font_demands.is_empty() {
            return Self {
                families: layout.font_demands.clone(),
            };
        }
        let mut families = BTreeMap::<String, BTreeSet<u32>>::new();
        for run in &layout.resolved_text {
            let mut add = |owner, text: &str| {
                if let Some(value) = layout
                    .styles
                    .get(&owner)
                    .and_then(|style| style.get("font-family"))
                {
                    for family in oryn_layout::cssom::font_family_names(value) {
                        families
                            .entry(family.to_ascii_lowercase())
                            .or_default()
                            .extend(oryn_layout::cssom::font_text_codepoints(text));
                    }
                }
            };
            if run.spans.is_empty() {
                add(run.layout_node, &run.text);
            } else {
                for span in &run.spans {
                    if let Some(text) = run.text.get(span.rendered_start..span.rendered_end) {
                        add(span.owner, text);
                    }
                }
            }
        }
        Self { families }
    }
    fn matches(&self, candidate: &FontCandidate) -> bool {
        self.families
            .get(&candidate.family.to_ascii_lowercase())
            .is_some_and(|characters| {
                candidate
                    .ranges
                    .iter()
                    .any(|(start, end)| characters.range(*start..=*end).next().is_some())
            })
    }
}
// Shared existing implicit/public font resource limits.
pub(super) const CSS_BINARY_RESPONSE_BYTES: usize = 24 * 1024 * 1024;
pub(super) const NATIVE_FONT_DECODE_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct CssBinary {
    pub url: String,
    pub bytes: Vec<u8>,
}
struct StylesheetResult {
    fatal: Option<HostError>,
    binary: Option<CssBinary>,
    result: Result<StylesheetSource, String>,
    _bytes: Option<ScriptSourceReservation>,
}
pub(super) struct StylesheetPreparation {
    prepared_implicit_backing:Option<Arc<ImplicitFontBacking>>,
    #[cfg(unix)]
    retained: Option<ParserResourceScheduler>,
    retry: Option<Instant>,
    _render_blocker: Option<oryn_dom::RenderBlocker>,
    published: Option<Option<bool>>,
    identity: ParserDocumentIdentity,
    dom: SharedDomRuntime,
    node: NodeId,
    revision: u64,
    install: bool,
    font_family: Option<String>,
    font_group: Option<String>,
    retained_font: Option<RetainedFontDemandEpoch>,
    font_descriptors: String,
    font_order: u64,
    requested_url: Option<String>,
    resource_attributes: Option<Vec<Option<String>>>,
    receiver: std::sync::mpsc::Receiver<StylesheetResult>,
    ready: Option<StylesheetResult>,
    _dependency_bytes: Option<std::sync::Arc<ScriptSourceReservation>>,
    _node: ParserScriptLease,
    _lifetime: std::sync::Arc<()>,
    _cancel: Option<standalone_stylesheet::Cancel>,
}
impl StylesheetPreparation {
    fn live(&self, host: &RawV8Host) -> bool {
        self.retained_font
            .as_ref()
            .is_none_or(RetainedFontDemandEpoch::current)
            && host.automation_realm_identity(self.identity.realm)
                == Some((
                    self.identity.document,
                    self.identity.document_generation,
                    self.identity.stream_epoch,
                    true,
                ))
            && !host.realm_document_load_stopped(self.identity.realm)
            && self.dom.borrow().node_document(self.node).ok().flatten()
                == Some(self.identity.document)
            && self.dom.borrow().is_connected(self.node).unwrap_or(false)
            && self
                .dom
                .borrow()
                .arena()
                .stylesheet_revision(self.node)
                .ok()
                == Some(self.revision)
            && self.resource_attributes.as_ref().is_none_or(|expected| {
                resource_attributes(&self.dom, self.node).as_ref() == Some(expected)
            })
    }
    fn ready(&mut self) -> bool {
        #[cfg(unix)]
        if let Some(scheduler) = &mut self.retained {
            return scheduler.has_ready_completion();
        }
        if self.ready.is_none() {
            match self.receiver.try_recv() {
                Ok(result) => self.ready = Some(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.ready = Some(StylesheetResult {
                        binary: None,
                        result: Err("stylesheet producer disconnected".into()),
                        fatal: (!self.install&&self.font_family.is_some()).then(||HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Infrastructure("stylesheet producer disconnected".into()))),
                        _bytes: None,
                    })
                }
            }
        }
        self.ready.is_some()
    }
}
impl RustExecutableDocument {
    fn schedule_css_dependency_batch(
        &mut self,
        realm: RealmId,
        batch: CssDependencyBatch,
    ) -> Result<(), HostError> {
        self.admit_css_dependency_batch(realm, &mut Some(batch))
    }
    fn admit_css_dependency_batch(
        &mut self,
        realm: RealmId,
        pending: &mut Option<CssDependencyBatch>,
    ) -> Result<(), HostError> {
        let batch = pending.as_ref().expect("pending CSS batch");
        if self.css_dependency_sources.iter().any(|entry| {
            entry.realm == realm
                && entry.batch.node == batch.node
                && entry.batch.revision == batch.revision
                && entry.batch.epoch == batch.epoch
                && entry.batch.dependencies == batch.dependencies
        }) {
            pending.take();
            return Ok(());
        }
        #[cfg(test)]
        css_publication_failure(4)?;
        let replaces = self
            .css_dependency_sources
            .iter()
            .any(|entry| entry.realm == realm && entry.batch.node == batch.node);
        if !replaces && self.css_dependency_sources.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "CSS demand source owners".into(),
            });
        }
        self.css_dependency_sources
            .try_reserve(1)
            .map_err(|_| HostError::LimitExceeded {
                resource: "CSS demand owner allocation".into(),
            })?;
        self.agent.host.activate_realm(realm)?;
        let dom = self
            .agent
            .host
            .active_dom()
            .ok_or_else(|| HostError::Execution("CSS demand document absent".into()))?;
        let identity = ParserFetchContext::capture(&self.agent.host, &dom, batch.node)?.identity();
        let node = ParserScriptLease::new(&dom, batch.node)?;
        self.css_dependency_sources
            .retain(|entry| !(entry.realm == realm && entry.batch.node == batch.node));
        let batch = pending.take().expect("accepted CSS batch");
        self.css_dependency_sources.push_back(CssDemandBatch {
            retry: None,
            evaluated: None,
            evaluated_font: None,
            realm,
            batch,
            attempted: BTreeSet::new(),
            identity,
            dom,
            _node: node,
        });
        Ok(())
    }
    fn dispatch_css_dependency_batch(
        &mut self,
        realm: RealmId,
        batch: &CssDependencyBatch,
        attempted: &mut BTreeSet<(String, Option<String>)>,
    ) -> Result<Option<RetainedFontDemandEpoch>, HostError> {
        #[cfg(test)]
        css_publication_failure(3)?;
        self.agent.host.activate_realm(realm)?;
        let Some(dom) = self.agent.host.active_dom() else {
            return Ok(None);
        };
        if self.agent.host.document_stream_epoch(realm) != Some(batch.epoch)
            || dom.borrow().arena().stylesheet_revision(batch.node).ok() != Some(batch.revision)
            || !dom.borrow().is_connected(batch.node).unwrap_or(false)
        {
            return Ok(None);
        }
        let demand = CssDemand::from_layout(&self.agent.host.native_css_resource_layout(realm)?);
        let font_allocation = self.agent.host.native_retained_admission_for_realm(realm)?;
        let _font_scope = if font_allocation.is_some() {
            Some(self.agent.host.retained_allocation_scope(realm)?)
        } else { None };
        let font_projection = retained_font_requests(&mut self.agent.host, realm, batch, &dom)?;
        let mut requests = Vec::new();
        let mut bytes = 0usize;
        let mut selected_fonts = BTreeSet::new();
        let mut relevant_fonts = BTreeSet::new();
        let mut unattempted_fonts=BTreeSet::new();
        if font_allocation.is_some() {
            self.agent.host.native_retained_admission_for_realm(realm)?;
        }
        let evaluated_font = font_projection.as_ref().and_then(|projection| projection.epoch.clone());
        let discovered = match font_projection {
            Some(projection) => projection,
            None => legacy_font_requests(&mut self.agent.host, batch)?,
        };
        for (base, initiator, policy, reference, candidate) in discovered.entries {
            let family = Some(candidate);
            let absolute = Url::parse(&base)
                .ok()
                .and_then(|base| base.join(&reference.url).ok())
                .map(|url| url.to_string())
                .unwrap_or_else(|| reference.url.clone());
            {
                if family.is_some()
                    && !self
                        .agent
                        .host
                        .css_font_owner_applicable(realm, batch.node)?
                {
                    continue;
                }
                if family
                    .as_ref()
                    .is_some_and(|family| !demand.matches(family))
                {
                    continue;
                }
                if let Some(family) = &family {
                    relevant_fonts.insert(family.key.clone());
                    if !attempted.contains(&(absolute.clone(),Some(family.key.clone()))){
                        unattempted_fonts.insert(family.key.clone());
                    }
                }
                if family.as_ref().is_some_and(|candidate| {
                    candidate.native_rule.is_none()&&self.css_font_timelines.iter().any(|timeline| {
                        timeline.identity.realm == realm
                            && timeline.node == batch.node
                            && timeline.revision == batch.revision
                            && timeline.font.key == candidate.key
                            && timeline.phase(Instant::now()) == FontDisplayPhase::Failure
                    })
                }) {
                    continue;
                }
                if attempted.contains(&(
                    absolute.clone(),
                    family.as_ref().map(|candidate| candidate.key.clone()),
                )) {
                    continue;
                }
                if let Some(candidate) = &family {
                    if selected_fonts.contains(&candidate.key)
                        || self.dynamic_stylesheets.iter().any(|job| {
                            job.identity.realm == realm
                                && job.node == batch.node
                                && job.font_group.as_deref() == Some(candidate.key.as_str())
                                && job.live(&self.agent.host)
                        })
                        || self.css_binary_resources.iter().any(|entry| {
                            entry.identity.realm == realm
                                && entry.node == batch.node
                                && entry.font_group.as_deref() == Some(candidate.key.as_str())
                                && entry.live(&self.agent.host)
                        })
                    {
                        continue;
                    }
                    selected_fonts.insert(candidate.key.clone());
                }
                if requests.len() >= 256 {
                    return Err(HostError::LimitExceeded {
                        resource: "CSS dependency requests".into(),
                    });
                }
                bytes = bytes
                    .saturating_add(base.len())
                    .saturating_add(initiator.len())
                    .saturating_add(policy.len())
                    .saturating_add(reference.url.len())
                    .saturating_add(family.as_ref().map_or(0, |candidate| {
                        candidate.family.len()
                            + candidate.key.len()
                            + candidate.descriptors.len()
                            + candidate.ranges.len().saturating_mul(8)
                    }));
                requests.push((
                    base.clone(),
                    initiator.clone(),
                    policy.clone(),
                    reference.clone(),
                    family,
                ));
            }
        }
        for timeline in &mut self.css_font_timelines {
            if timeline.identity.realm == realm
                && timeline.node == batch.node
                && timeline.revision == batch.revision
                && !timeline.loaded
                && relevant_fonts.contains(&timeline.font.key)
                && !selected_fonts.contains(&timeline.font.key)
                && !self.dynamic_stylesheets.iter().any(|job| {
                    job.identity.realm == realm
                        && job.node == batch.node
                        && job.font_group.as_deref() == Some(timeline.font.key.as_str())
                        && job.live(&self.agent.host)
                })
            {
                timeline.failed = true;
                if !unattempted_fonts.contains(&timeline.font.key){timeline.implicit_terminal=Some(false);}
            }
        }
        self.poll_font_display()?;
        if requests.is_empty() {
            return Ok(evaluated_font);
        }
        if self
            .dynamic_stylesheets
            .len()
            .saturating_add(requests.len())
            > 1024
        {
            return Err(HostError::LimitExceeded {
                resource: "CSS dependency jobs".into(),
            });
        }
        self.dynamic_stylesheets
            .try_reserve(requests.len())
            .map_err(|_| HostError::LimitExceeded {
                resource: "CSS dependency job allocation".into(),
            })?;
        let retained = std::sync::Arc::new(
            ScriptSourceReservation::reserve(&self.dynamic_scripts.source_bytes, bytes)
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "CSS dependency request bytes".into(),
                })?,
        );
        let fetch = ParserFetchContext::capture(&self.agent.host, &dom, batch.node)?;
        // One job per selected face; failed decoding permits the next source.
        // Image demand is owned exclusively by the native CSS image manager.
        for (base, initiator, policy, reference, font_family) in requests {
            let absolute = Url::parse(&base)
                .ok()
                .and_then(|base| base.join(&reference.url).ok())
                .map(|url| url.to_string())
                .unwrap_or_else(|| reference.url.clone());
            let accepted_key = (
                absolute,
                font_family.as_ref().map(|candidate| candidate.key.clone()),
            );
            if let Some(candidate) = &font_family {
                self.begin_font_display(fetch.identity(), &dom, batch, candidate)?;
            }
            let (sender, receiver) = std::sync::mpsc::channel();
            let lifetime = std::sync::Arc::new(());
            let mut job = StylesheetPreparation {
                prepared_implicit_backing:None,
                #[cfg(unix)]
                retained: None,
                retry: None,
                published: None,
                _render_blocker: None,
                identity: fetch.identity(),
                dom: dom.clone(),
                node: batch.node,
                revision: batch.revision,
                install: false,
                font_family: font_family
                    .as_ref()
                    .map(|candidate| candidate.family.clone()),
                font_group: font_family.as_ref().map(|candidate| candidate.key.clone()),
                retained_font: font_family
                    .as_ref()
                    .and_then(|candidate| candidate.retained.clone()),
                font_descriptors: font_family
                    .as_ref()
                    .map(|candidate| candidate.descriptors.clone())
                    .unwrap_or_default(),
                font_order: font_family.as_ref().map_or(0, |candidate| candidate.order),
                requested_url: Url::parse(&base)
                    .ok()
                    .and_then(|base| base.join(&reference.url).ok())
                    .map(|url| url.to_string()),
                resource_attributes: resource_attributes(&dom, batch.node),
                receiver,
                ready: None,
                _dependency_bytes: Some(retained.clone()),
                _node: ParserScriptLease::new(&dom, batch.node)?,
                _lifetime: lifetime.clone(),
                _cancel: None,
            };
            let is_font = font_family.is_some();
            let failed = sender.clone();
            let weak = std::sync::Arc::downgrade(&lifetime);
            #[cfg(unix)]
            let context = self
                .context
                .as_ref()
                .map(|(_, page_id, opener)| {
                    self.agent
                        .host
                        .resource_client_for_realm(fetch.realm, *page_id)
                        .map(|client| (client, *page_id, *opener))
                })
                .transpose()?;
            #[cfg(unix)]
            let ownership = context
                .as_ref()
                .map(|(client, _, _)| client.track_resource_job())
                .transpose()
                .map_err(|error| HostError::Execution(error.to_string()))?;
            #[cfg(unix)]
            let loader = if context.is_none() {
                let (loader, cancel) = standalone_stylesheet::Loader::capture(
                    &mut self.agent.host,
                    &self.resources,
                    &fetch,
                    self.dynamic_scripts.source_bytes.clone(),
                )?;
                job._cancel = Some(cancel);
                Some(loader)
            } else {
                None
            };
            #[cfg(unix)]
            let native =
                std::sync::Arc::new(crate::context_service::ResourceCancellation::default());
            #[cfg(unix)]
            if context.is_some() {
                job._cancel = Some(standalone_stylesheet::Cancel::native(native.clone()));
            }
            #[cfg(not(unix))]
            let loader = {
                let (loader, cancel) = standalone_stylesheet::Loader::capture(
                    &mut self.agent.host,
                    &self.resources,
                    &fetch,
                    self.dynamic_scripts.source_bytes.clone(),
                )?;
                job._cancel = Some(cancel);
                Some(loader)
            };
            #[cfg(unix)]
            let creator = fetch.resource_initiator.clone();
            let request_bytes = retained.clone();
            let body_budget = self.dynamic_scripts.source_bytes.clone();
            if let Some(candidate)=&font_family{
                let timeline=self.css_font_timelines.iter_mut().find(|timeline|
                    timeline.identity.realm==fetch.realm&&timeline.node==batch.node
                    &&timeline.revision==batch.revision&&timeline.font.key==candidate.key)
                    .ok_or_else(||HostError::Execution("implicit font timeline absent".into()))?;
                if !timeline.implicit_started{
                    if let (Some(rule),Some(epoch))=(&candidate.native_rule,&candidate.retained){
                        let identity=epoch.publication.graph.borrow().connected_font_context(rule)
                            .map_err(HostError::Execution)?.identity();
                        timeline.implicit_tokens=Some(self.agent.host.publish_implicit_font_transition(fetch.realm,identity)?);
                    }
                    timeline.implicit_started=true;
                }
            }
            let started = resource_io_executor::submit(move || {
                #[cfg(unix)]
                let _ownership = ownership;
                let _request_bytes = request_bytes;
                let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                    if weak.upgrade().is_none(){return Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Cancelled));}
                    if let Some(name)=reference.url.strip_prefix("oryn-local-font:"){
                        let bytes=oryn_layout::local_font_bytes(name).ok_or_else(||HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch("local font no longer available".into())))?;
                        return Ok(CssBinary{url:reference.url.clone(),bytes:bytes.to_vec()});
                    }
                    if let Some(loader)=loader{return loader.load_dependency(&base,&initiator,&policy,reference);}
                    #[cfg(unix)] {
                    let(client,page_id,_)=context.expect("captured CSS endpoint");let creator=creator.ok_or_else(||HostError::Execution("CSS dependency native initiator absent".into()))?;
                    match native.run(&client,page_id,creator.creator.clone(),css_subresource_request(page_id,&creator,&base,&initiator,&policy,reference)).map_err(|error|HostError::Execution(error.to_string()))?{
                        crate::context_service::ContextServiceResponse::PageBinaryResource{url,body_base64,..}=>{
                            if body_base64.len()>CSS_BINARY_RESPONSE_BYTES{return Err(HostError::LimitExceeded{resource:"CSS binary response".into()});}
                            let bytes=base64::Engine::decode(&base64::engine::general_purpose::STANDARD,body_base64).map_err(|error|HostError::Execution(error.to_string()))?;
                            Ok(CssBinary{url,bytes})
                        },
                        _=>Err(HostError::Execution("invalid CSS dependency response".into())),
                    }
                    }
                    #[cfg(not(unix))] {Err(HostError::Execution("CSS local loader missing".into()))}
                })).map_err(|_|HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Infrastructure("CSS dependency producer panicked".into())))
                    .and_then(|result:Result<CssBinary,HostError>|result);
                let result = result.and_then(|binary| {
                    if is_font {
                        oryn_layout::decode_web_font(&binary.bytes, NATIVE_FONT_DECODE_BYTES)
                            .map_err(|error|HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(error)))?;
                    }
                    Ok(binary)
                });
                let response = match result {
                    Ok(binary) => {
                        match ScriptSourceReservation::reserve(&body_budget, binary.bytes.len()) {
                            Some(bytes) => StylesheetResult {
                                fatal: None,
                                binary: Some(binary),
                                result: Ok((String::new(), String::new(), Vec::new())),
                                _bytes: Some(bytes),
                            },
                            None => StylesheetResult {
                                fatal:Some(HostError::LimitExceeded{resource:"CSS binary retained byte capacity".into()}),
                                binary: None,
                                result: Err("CSS binary retained byte capacity".into()),
                                _bytes: None,
                            },
                        }
                    }
                    Err(error) => {
                        let source=matches!(&error,HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(_)|crate::script_load_failure::ScriptLoadFailure::Integrity(_)));
                        StylesheetResult{binary:None,result:Err(error.to_string()),fatal:(!source).then_some(error),_bytes:None}
                    },
                };
                let _ = sender.send(response);
            });
            if let Err(error) = started {
                let _ = failed.send(StylesheetResult {
                    fatal:Some(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Infrastructure(error.clone()))),
                    binary: None,
                    result: Err(error),
                    _bytes: None,
                });
            }
            self.dynamic_stylesheets.push_back(job);
            attempted.insert(accepted_key);
        }
        Ok(evaluated_font)
    }
    pub(super) fn schedule_document_pi_stylesheets(
        &mut self,
        realm: RealmId,
    ) -> Result<(), HostError> {
        let Some((document, _, _, true)) = self.agent.host.automation_realm_identity(realm) else {
            return Ok(());
        };
        let Some(dom) = self.agent.host.automation_dom(realm) else {
            return Ok(());
        };
        let nodes = {
            let dom = dom.borrow();
            dom.arena()
                .get(document)
                .map_err(|error| HostError::Execution(error.to_string()))?
                .children
                .iter()
                .copied()
                .filter(|node| dom.arena().xml_stylesheet(*node).ok().flatten().is_some())
                .collect::<Vec<_>>()
        };
        for node in nodes {
            self.schedule_dynamic_stylesheet(realm, node)?;
        }
        Ok(())
    }
    pub(super) fn dynamic_stylesheets_block_scripts(
        &self,
        realm: RealmId,
    ) -> Result<bool, HostError> {
        for job in &self.dynamic_stylesheets {
            if job.identity.realm == realm
                && job.install
                && job.live(&self.agent.host)
                && self
                    .agent
                    .host
                    .cached_stylesheet_blocks_scripts_in_realm(realm, job.node)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn dynamic_stylesheets_pending(&self, realm: Option<RealmId>) -> bool {
        self.css_dependency_sources.iter().any(|entry| {
            entry.evaluated.is_none() && realm.is_none_or(|realm| realm == entry.realm)
        }) || self.dynamic_stylesheets.iter().any(|job| {
            realm.is_none_or(|realm| realm == job.identity.realm) && job.live(&self.agent.host)
        })
    }
    pub(super) fn dynamic_stylesheets_runnable(&mut self) -> bool {
        if self
            .font_display_delay()
            .is_some_and(|delay| delay.is_zero())
        {
            return true;
        }
        self.css_dependency_sources.iter().any(|entry| {
            !self.agent.host.realm_is_parked(entry.realm)
                && entry.retry.is_none_or(|retry| retry <= Instant::now())
                && (entry.evaluated != Some(entry.dom.borrow().mutation_cursor())
                    || !retained_font_evaluation_current(&self.agent.host, entry))
        }) || self.dynamic_stylesheets.iter_mut().any(|job| {
            !job.live(&self.agent.host)
                || (!self.agent.host.realm_is_parked(job.identity.realm)
                    && job.retry.is_none_or(|retry| retry <= Instant::now())
                    && job.ready())
        })
    }
    #[cfg(not(unix))]
    pub(super) fn portable_stylesheet_pending(
        &self,
        realm: RealmId,
        node: NodeId,
        revision: u64,
    ) -> bool {
        self.dynamic_stylesheets.iter().any(|job| {
            job.identity.realm == realm
                && job.node == node
                && job.revision == revision
                && job.live(&self.agent.host)
        })
    }
    pub(super) fn schedule_dynamic_stylesheet(
        &mut self,
        realm: RealmId,
        node: NodeId,
    ) -> Result<(), HostError> {
        self.schedule_stylesheet_with_parser(realm, node, false)
    }
    pub(super) fn schedule_stylesheet_with_parser(
        &mut self,
        realm: RealmId,
        node: NodeId,
        parser_inserted: bool,
    ) -> Result<(), HostError> {
        self.agent.host.activate_realm(realm)?;
        let dom = self
            .agent
            .host
            .active_dom()
            .ok_or_else(|| HostError::Execution("stylesheet DOM unavailable".into()))?;
        if !dom.borrow().is_connected(node).unwrap_or(false) {
            return Ok(());
        }
        self.agent.host.reconcile_inline_style_policy()?;
        if dom.borrow().arena().get(node).is_ok_and(|node| {
            !node.stylesheet_policy_allowed
                && matches!(&node.kind,NodeKind::Element{name} if name.local.as_ref()=="style")
        }) {
            return Ok(());
        }
        let revision = dom
            .borrow()
            .arena()
            .stylesheet_revision(node)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        if self.dynamic_stylesheets.iter().any(|job| {
            job.identity.realm == realm
                && job.node == node
                && job.revision == revision
                && job.live(&self.agent.host)
        }) {
            return Ok(());
        }
        self.dynamic_stylesheets
            .retain(|job| !(job.identity.realm == realm && job.node == node));
        if self.dynamic_stylesheets.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "dynamic stylesheet preparations".into(),
            });
        }
        let mut fetch = ParserFetchContext::capture(&self.agent.host, &dom, node)?;
        fetch.render_blocking = dom
            .borrow()
            .potentially_render_blocking(node, parser_inserted);
        let identity = fetch.identity();
        let (linked, inline, install, href, css, encoding) = {
            let dom = dom.borrow();
            let record = dom
                .arena()
                .get(node)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let pi = dom
                .arena()
                .xml_stylesheet(node)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let inline = matches!(&record.kind,NodeKind::Element{name} if matches!(name.ns.as_ref(),"http://www.w3.org/1999/xhtml"|"http://www.w3.org/2000/svg")&&name.local.as_ref()=="style");
            let linked =
                pi.is_some() || dom.is_element(node, "http://www.w3.org/1999/xhtml", "link");
            let href = pi.map(|sheet| sheet.href).unwrap_or_else(|| {
                record
                    .attributes
                    .get_null("href")
                    .cloned()
                    .unwrap_or_default()
            });
            let install = dom
                .arena()
                .is_css_stylesheet_owner(node)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let css = if inline {
                dom.arena()
                    .stylesheet_text(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?
            } else {
                RESOURCE_ATTRIBUTES
                    .into_iter()
                    .filter(|name| *name != "style" || record.style_attribute_policy_allowed)
                    .filter_map(|name| {
                        record.attributes.get_null(name).map(|value| {
                            if name == "background" {
                                format!("background-image:url({value})")
                            } else {
                                value.clone()
                            }
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(";")
            };
            let encoding = dom
                .arena()
                .get(identity.document)
                .ok()
                .and_then(|node| node.document_metadata.as_ref())
                .map(|metadata| metadata.encoding.clone())
                .unwrap_or_else(|| "UTF-8".into());
            (linked, inline, install, href, css, encoding)
        };
        if !install {
            let mut dependencies = Vec::new();
            collect_css_dependencies(
                &mut dependencies,
                &fetch.base_url,
                &fetch.initiator_url,
                &fetch.referrer_policy,
                &css,
            )?;
            let size = dependencies
                .iter()
                .map(|(base, initiator, policy, css)| {
                    base.len() + initiator.len() + policy.len() + css.len()
                })
                .sum();
            let bytes = ScriptSourceReservation::reserve(&self.dynamic_scripts.source_bytes, size)
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "inline CSS resource metadata".into(),
                })?;
            return self.schedule_css_dependency_batch(
                realm,
                CssDependencyBatch {
                    node,
                    revision,
                    epoch: identity.stream_epoch,
                    dependencies,
                    _allocation: None,
                    _bytes: bytes,
                },
            );
        }
        let pure = inline
            && !oryn_layout::cssom::stylesheet_requires_resources(&css, &fetch.base_url)
                .map_err(HostError::Execution)?;
        #[cfg(unix)]
        let retained = self.agent.host.native_retained_admission_for_realm(realm)?.is_some();
        let budget = self.dynamic_scripts.source_bytes.clone();
        let reservation =
            ScriptSourceReservation::reserve(&budget, css.len().saturating_add(href.len()))
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "queued stylesheet request bytes".into(),
                })?;
        let lifetime = std::sync::Arc::new(());
        let (sender, receiver) = std::sync::mpsc::channel();
        let render_blocker = dom
            .borrow_mut()
            .admit_render_resource(node, parser_inserted)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let mut job = StylesheetPreparation {
            prepared_implicit_backing:None,
            #[cfg(unix)]
            retained: None,
            retry: None,
            published: None,
            _render_blocker: render_blocker,
            identity,
            dom: dom.clone(),
            node,
            revision,
            install,
            font_family: None,
            font_group: None,
            retained_font: None,
            font_descriptors: String::new(),
            font_order: 0,
            requested_url: None,
            resource_attributes: (!install)
                .then(|| resource_attributes(&dom, node))
                .flatten(),
            receiver,
            ready: None,
            _dependency_bytes: None,
            _node: ParserScriptLease::new(&dom, node)?,
            _lifetime: lifetime.clone(),
            _cancel: None,
        };
        #[cfg(unix)]
        if retained {
            // The retained scheduler admits the same owner render blocker;
            // release the legacy preparation's token before creating it.
            job._render_blocker = None;
            #[cfg(test)]
            if DYNAMIC_RETAINED_SCHEDULE_FAILURE.with(|flag| flag.replace(false)) {
                return Err(HostError::Execution("injected dynamic native schedule preflight".into()));
            }
            let mut scheduler = ParserResourceScheduler::with_page_limits(&self.dynamic_scripts);
            let scheduled = scheduler.schedule_stylesheet_auto(AutomaticStylesheetScheduleInput {
                context: self.context.as_ref(), node, revision,
                href: if linked { Some(href) } else { None }, css, encoding, fetch,
                host: &mut self.agent.host, resources: &self.resources,
            });
            // The original binding queue head is not committed on Err;
            // its owner/source generation remains available for retry.
            scheduled?;
            job.retained = Some(scheduler);
            self.dynamic_stylesheets.push_back(job);
            return Ok(());
        }
        if pure {
            job.ready = Some(StylesheetResult {
                fatal: None,
                binary: None,
                result: Ok((fetch.initiator_url, css, Vec::new())),
                _bytes: Some(reservation),
            });
            self.dynamic_stylesheets.push_back(job);
            return Ok(());
        }
        #[cfg(unix)]
        if let Some((_, page_id, _)) = self.context.clone() {
            let client = self
                .agent
                .host
                .resource_client_for_realm(fetch.realm, page_id)?;
            let resource_initiator = fetch.stylesheet_initiator()?;
            let ownership = client
                .track_resource_job()
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let failed = sender.clone();
            let lifetime = std::sync::Arc::downgrade(&lifetime);
            let native =
                std::sync::Arc::new(crate::context_service::ResourceCancellation::default());
            let cancel = native.flag.clone();
            job._cancel = Some(standalone_stylesheet::Cancel::native(native.clone()));
            let started = resource_io_executor::submit(move || {
                let _ownership = ownership;
                let mut expansion =
                    StylesheetExpansionBudget::new(budget.clone(), cancel).with_native(native);
                let result = if lifetime.upgrade().is_some() {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if linked {
                            let requested = fetch.resolve(&href)?;
                            load_parser_stylesheet(crate::executable::ParserStylesheetInput {
                                client: &client,
                                page_id,
                                initiator: &resource_initiator,
                                requested_url: &requested,
                                initiator_url: Some(fetch.initiator_url.clone()),
                                referrer_policy: Some(fetch.referrer_policy.clone()),
                                environment_encoding: &encoding,
                                depth: 0,
                                budget: &mut expansion,
                            })
                        } else if inline {
                            load_inline_css_resources(crate::executable::InlineCssResourceInput {
                                client: &client,
                                page_id,
                                initiator: &resource_initiator,
                                document_url: &fetch.initiator_url,
                                base_url: &fetch.base_url,
                                referrer_policy: &fetch.referrer_policy,
                                css: &css,
                                environment_encoding: &encoding,
                                budget: &mut expansion,
                            })
                        } else {
                            Err(HostError::Execution(
                                "non-sheet source bypassed dependency scheduler".into(),
                            ))
                        }
                    }))
                    .map_err(|_| "stylesheet producer panicked".to_owned())
                    .and_then(
                        |result: Result<StylesheetSource, HostError>| {
                            result.map_err(|error| error.to_string())
                        },
                    )
                } else {
                    Err("stylesheet owner retired".into())
                };
                drop(expansion);
                drop(reservation);
                if lifetime.upgrade().is_none() {
                    return;
                }
                let bytes = result.as_ref().map_or(0, stylesheet_source_bytes);
                let reservation = ScriptSourceReservation::reserve(&budget, bytes);
                let result = if reservation.is_some() {
                    result
                } else {
                    Err("queued stylesheet response bytes exceeded".into())
                };
                let _ = sender.send(StylesheetResult {
                    binary: None,
                    result,
                    _bytes: reservation,
                    fatal: None,
                });
            });
            if let Err(error) = started {
                let _ = failed.send(StylesheetResult {
                    binary: None,
                    result: Err(error),
                    _bytes: None,
                    fatal: None,
                });
            }
            self.dynamic_stylesheets.push_back(job);
            return Ok(());
        }
        {
            let (loader, cancel) = standalone_stylesheet::Loader::capture(
                &mut self.agent.host,
                &self.resources,
                &fetch,
                budget.clone(),
            )?;
            job._cancel = Some(cancel);
            let failed = sender.clone();
            let lifetime = std::sync::Arc::downgrade(&lifetime);
            let started = resource_io_executor::submit(move || {
                let result = if lifetime.upgrade().is_some() {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        loader.load(linked, install, &fetch.base_url, &href, &css, &encoding)
                    }))
                    .map_err(|_| "standalone stylesheet producer panicked".to_owned())
                    .and_then(|result| result.map_err(|error| error.to_string()))
                } else {
                    Err("stylesheet owner retired".into())
                };
                drop(reservation);
                if lifetime.upgrade().is_none() {
                    return;
                }
                let bytes = result.as_ref().map_or(0, stylesheet_source_bytes);
                let reservation = ScriptSourceReservation::reserve(&budget, bytes);
                let result = if reservation.is_some() {
                    result
                } else {
                    Err("queued standalone stylesheet response bytes exceeded".into())
                };
                let _ = sender.send(StylesheetResult {
                    binary: None,
                    result,
                    _bytes: reservation,
                    fatal: None,
                });
            });
            if let Err(error) = started {
                let _ = failed.send(StylesheetResult {
                    binary: None,
                    result: Err(error),
                    _bytes: None,
                    fatal: None,
                });
            }
        }

        self.dynamic_stylesheets.push_back(job);
        Ok(())
    }
    pub(super) fn poll_dynamic_stylesheets(&mut self) -> Result<(), HostError> {
        let previous = self.agent.host.active_realm();
        let result = (|| {
            self.poll_font_display()?;
            let mut transfer_error = None;
            for (realm, batch) in self.agent.host.take_css_dependencies() {
                let mut pending = Some(batch);
                if let Err(error) = self.admit_css_dependency_batch(realm, &mut pending) {
                    if let Some(batch) = pending {
                        self.agent.host.restore_css_dependency(realm, batch);
                    }
                    if transfer_error.is_none() {
                        transfer_error = Some(error);
                    }
                }
            }
            if let Some(error) = transfer_error {
                return Err(error);
            }
            for _ in 0..self.css_dependency_sources.len().min(64) {
                let mut source = self
                    .css_dependency_sources
                    .pop_front()
                    .expect("bounded CSS demand queue");
                let live = self.agent.host.automation_realm_identity(source.realm)
                    == Some((
                        source.identity.document,
                        source.identity.document_generation,
                        source.identity.stream_epoch,
                        true,
                    ))
                    && source
                        .dom
                        .borrow()
                        .node_document(source.batch.node)
                        .ok()
                        .flatten()
                        == Some(source.identity.document)
                    && !self.agent.host.realm_document_load_stopped(source.realm)
                    && source
                        .dom
                        .borrow()
                        .is_connected(source.batch.node)
                        .unwrap_or(false)
                    && source
                        .dom
                        .borrow()
                        .arena()
                        .stylesheet_revision(source.batch.node)
                        .ok()
                        == Some(source.batch.revision);
                if !live {
                    continue;
                }
                if self.agent.host.realm_is_parked(source.realm)
                    || source.retry.is_some_and(|retry| retry > Instant::now())
                {
                    self.css_dependency_sources.push_back(source);
                    continue;
                }
                match self.dispatch_css_dependency_batch(
                    source.realm,
                    &source.batch,
                    &mut source.attempted,
                ) {
                    Ok(evaluated_font) => {
                        source.retry = None;
                        source.evaluated = Some(source.dom.borrow().mutation_cursor());
                        source.evaluated_font = evaluated_font;
                    }
                    Err(error) => {
                        source.evaluated = None;
                        source.retry = Some(Instant::now() + Duration::from_millis(25));
                        self.css_dependency_sources.push_front(source);
                        return Err(error);
                    }
                }
                self.css_dependency_sources.push_back(source);
            }
            let turns = self.dynamic_stylesheets.len().min(64);
            for _ in 0..turns {
                let mut job = self
                    .dynamic_stylesheets
                    .pop_front()
                    .expect("bounded stylesheet queue");
                if !job.live(&self.agent.host) {
                    continue;
                }
                if self.agent.host.realm_is_parked(job.identity.realm)
                    || job.retry.is_some_and(|retry| retry > Instant::now())
                {
                    self.dynamic_stylesheets.push_back(job);
                    continue;
                }
                if !job.ready() {
                    self.dynamic_stylesheets.push_back(job);
                    continue;
                }
                if let Some(error)=job.ready.as_mut().and_then(|result|result.fatal.take()){
                    for source in &mut self.css_dependency_sources{
                        if source.realm==job.identity.realm&&source.batch.node==job.node&&source.batch.revision==job.revision{
                            if let Some(url)=&job.requested_url{source.attempted.remove(&(url.clone(),job.font_group.clone()));}
                            source.evaluated=None;
                        }
                    }
                    return Err(error);
                }
                #[cfg(unix)]
                if let Some(scheduler) = &mut job.retained {
                    let outcome = (|| {
                        self.agent.host.activate_realm(job.identity.realm)?;
                        for _ in 0..MAX_SCRIPT_COMPLETIONS_PER_SLICE {
                            let Some(completion) = scheduler.try_next_with_host(&mut self.agent.host) else { break; };
                            let ParserResourceCompletion::RetainedStylesheet(ready) = completion else {
                                scheduler.retry_completion(completion);
                                return Err(HostError::Execution("unexpected dynamic retained completion".into()));
                            };
                            scheduler.apply_retained_ready_job(Some(ready), &job.dom, &mut self.agent.host)?;
                        }
                        scheduler.poll_retained_ready_jobs(&job.dom, &mut self.agent.host)?;
                        scheduler.poll_native_pending_imports(&mut self.agent.host)
                    })();
                    if let Err(error) = outcome {
                        job.retry = Some(Instant::now() + Duration::from_millis(25));
                        self.dynamic_stylesheets.push_front(job);
                        return Err(error);
                    }
                    if scheduler.has_stylesheet_owner_work() {
                        self.dynamic_stylesheets.push_back(job);
                    }
                    continue;
                }
                if job.install {
                    let publication = (|| {
                        self.agent.host.activate_realm(job.identity.realm)?;
                        if job.published.is_none() {
                            let result = &job.ready.as_ref().expect("ready stylesheet").result;
                            job.published = Some(publish_parser_stylesheet_completion(
                                crate::executable::StylesheetPublicationInput {
                                    identity: job.identity,
                                    node: job.node,
                                    revision: job.revision,
                                    result,
                                    live_dom: &job.dom,
                                    host: &mut self.agent.host,
                                    resources: &mut self.resources,
                                    diagnostics: &mut self.diagnostics,
                                },
                            )?);
                        }
                        if let Some(success) = job.published.flatten() {
                            self.agent.host.queue_stylesheet_event(
                                job.node,
                                job.revision,
                                success,
                            )?;
                        }
                        Ok::<_, HostError>(())
                    })();
                    if let Err(error) = publication {
                        job.retry = Some(Instant::now() + Duration::from_millis(25));
                        self.dynamic_stylesheets.push_front(job);
                        return Err(error);
                    }
                    // Release only after native sheet publication and admission
                    // of its retained load/error task; retry cannot rerun CSS IO.
                    continue;
                }
                let publication = (|| {
                    self.agent.host.activate_realm(job.identity.realm)?;
                    self.publish_css_binary_retained(&mut job)?;
                    if job.ready.as_ref().expect("ready stylesheet").result.is_ok(){
                        for timeline in &mut self.css_font_timelines{
                            if timeline.identity.realm==job.identity.realm&&timeline.node==job.node
                                &&timeline.revision==job.revision&&Some(&timeline.font.key)==job.font_group.as_ref(){
                                timeline.implicit_terminal=Some(true);
                            }
                        }
                        self.poll_font_display()?;
                    }
                    Ok::<_,HostError>(())
                })();
                if let Err(error) = publication {
                    job.retry = Some(Instant::now() + Duration::from_millis(25));
                    self.dynamic_stylesheets.push_front(job);
                    return Err(error);
                }
                let result = job.ready.take().expect("ready stylesheet");
                if let Err(error) = result.result {
                    self.diagnostics.push(css_resource_failure_diagnostic(
                        job.font_family.as_deref(),
                        job.requested_url.as_deref(),
                        error,
                    ));
                    for source in &mut self.css_dependency_sources {
                        if source.realm == job.identity.realm
                            && source.batch.node == job.node
                            && source.batch.revision == job.revision
                        {
                            source.evaluated = None;
                        }
                    }
                }
            }
            Ok(())
        })();
        let restored = self.agent.host.activate_realm(previous);
        result.and(restored)
    }
}

fn css_resource_failure_diagnostic(
    font_family: Option<&str>,
    url: Option<&str>,
    error: String,
) -> oryn_common::v3::CapabilityDiagnostic {
    let mut diagnostic =
        script_diagnostic_at(HostError::Execution(error), url.unwrap_or("CSS resource"));
    if let Some(family) = font_family {
        diagnostic.capability = "font.load".into();
        diagnostic.detail = Some(format!(
            "font family {family}: {}",
            diagnostic.detail.unwrap_or_default()
        ));
    }
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_inline_declaration_sheet_keeps_native_publication_resource_free() {
        let mut page = RustExecutableDocument::load(
            r#"<!doctype html><style id=rules></style><script>
            document.querySelector('style').innerText =
                `input { border: 1px solid }`.repeat(2 << 16);
            </script><input>"#,
        )
        .unwrap();
        assert_eq!(
            page.evaluate("String(document.querySelector('style').textContent.length)")
                .unwrap(),
            ("input { border: 1px solid }".len() * (2 << 16)).to_string()
        );
        assert_eq!(
            page.evaluate("getComputedStyle(document.querySelector('input')).borderTopWidth")
                .unwrap(),
            "1px"
        );
    }

    #[test]
    fn font_failure_diagnostic_preserves_resource_and_nonfont_classification() {
        let font = css_resource_failure_diagnostic(
            Some("Missing"),
            Some("https://fonts.test/missing.ttf"),
            "HTTP 404".into(),
        );
        assert_eq!(font.capability, "font.load");
        let detail = font.detail.unwrap();
        assert!(detail.contains("Missing"));
        assert!(detail.contains("https://fonts.test/missing.ttf"));
        assert!(detail.contains("HTTP 404"));
        let image = css_resource_failure_diagnostic(
            None,
            Some("https://fonts.test/image.png"),
            "HTTP 404".into(),
        );
        assert_eq!(image.capability, "javascript.execution");
        assert!(
            image
                .detail
                .unwrap()
                .contains("https://fonts.test/image.png")
        );
    }
    #[test]
    fn font_display_periods_have_consumable_block_swap_and_failure_deadlines() {
        let fallback = FontDisplayPeriods::parse("font-display:fallback");
        assert_eq!(
            fallback.phase(Duration::from_millis(99), false),
            FontDisplayPhase::Block
        );
        assert_eq!(
            fallback.phase(Duration::from_millis(100), false),
            FontDisplayPhase::Swap
        );
        assert_eq!(
            fallback.phase(Duration::from_millis(3100), false),
            FontDisplayPhase::Failure
        );
        let optional = FontDisplayPeriods::parse("font-display:optional");
        assert_eq!(
            optional.phase(Duration::from_millis(100), false),
            FontDisplayPhase::Failure
        );
        assert_eq!(
            optional.phase(Duration::from_secs(4), true),
            FontDisplayPhase::Loaded
        );
        assert_eq!(
            FontDisplayPeriods::parse("font-display:swap").phase(Duration::ZERO, false),
            FontDisplayPhase::Swap
        );
    }
    #[test]
    fn optional_deadline_unblocks_both_layout_inputs_and_rejects_late_publication() {
        let mut page =
            RustExecutableDocument::load("<style id=s>p{font-family:Demo}</style><p>AB</p>")
                .unwrap();
        let node = page.agent.dom.borrow().find_by_id("s").unwrap();
        let realm = page.top_realm;
        let dom = page.agent.dom.clone();
        let identity = ParserFetchContext::capture(&page.agent.host, &dom, node)
            .unwrap()
            .identity();
        let revision = dom.borrow().arena().stylesheet_revision(node).unwrap();
        let batch = CssDependencyBatch {
            node,
            revision,
            epoch: identity.stream_epoch,
            dependencies: Vec::new(),
            _allocation: None,
            _bytes: ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, 0)
                .unwrap(),
        };
        let candidate = FontCandidate {
            native_rule: None,
            family: "Demo".into(),
            key: "display-test".into(),
            ranges: vec![(0, 0x10ffff)],
            descriptors: "font-display:optional".into(),
            order: 0,
            retained: None,
        };
        page.begin_font_display(identity, &dom, &batch, &candidate)
            .unwrap();
        assert_eq!(
            page.css_font_timelines[0].published,
            Some(FontDisplayPhase::Block)
        );
        assert!(page.next_timer_delay().unwrap() <= Duration::from_millis(100));
        page.css_font_timelines[0].started = Instant::now() - Duration::from_millis(101);
        assert_eq!(page.font_display_delay(), Some(Duration::ZERO));
        page.poll_font_display().unwrap();
        assert_eq!(
            page.css_font_timelines[0].published,
            Some(FontDisplayPhase::Failure)
        );
        assert_eq!(page.font_display_delay(), None);
        page.schedule_dynamic_stylesheet(realm, node).unwrap();
        let mut job = page.dynamic_stylesheets.pop_back().unwrap();
        job.font_family = Some(candidate.family);
        job.font_group = Some(candidate.key);
        job.font_descriptors = candidate.descriptors;
        let bytes = oryn_layout::DEFAULT_FONT_BYTES.to_vec();
        let custody =
            ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, bytes.len())
                .unwrap();
        page.publish_css_binary(
            &mut job,
            CssBinary {
                url: "https://example.test/late.ttf".into(),
                bytes,
            },
            Some(custody),
        )
        .unwrap();
        assert!(page.css_binary_resources.is_empty());
        assert!(!page.css_font_timelines[0].loaded);
    }
    #[test]
    fn font_sources_filter_before_fetch_and_preserve_fallback_and_descriptors() {
        let mut page = RustExecutableDocument::load("<body>Font demand</body>").unwrap();
        let requests=font_requests(&mut page.agent.host,"@font-face{font-family:Demo;font-weight:700;unicode-range:U+0041;src:local('Nonexistent Font'),url(skip.svg) format(svg),url(first.woff2) format(woff2),url(last.ttf) format(truetype)}","https://example.test/style.css").unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].0.url.ends_with("first.woff2"));
        assert!(requests[1].0.url.ends_with("last.ttf"));
        assert_eq!(requests[0].1.key, requests[1].1.key);
        assert_eq!(requests[0].1.ranges, vec![(0x41, 0x41)]);
        assert!(requests[0].1.descriptors.contains("700"));
    }
    #[test]
    fn final_pseudo_font_demand_is_authoritative_over_ordinary_style() {
        let mut layout = oryn_js::v8_bindings::BindingLayout::default();
        layout.styles.insert(
            1,
            std::sync::Arc::new(BTreeMap::from([("font-family".into(), "Ordinary".into())])),
        );
        layout
            .font_demands
            .insert("dropcap".into(), BTreeSet::from(['S' as u32]));
        let demand = CssDemand::from_layout(&layout);
        assert_eq!(demand.families, layout.font_demands);
        assert!(!demand.families.contains_key("ordinary"));
    }
    #[test]
    fn font_demand_uses_each_rendered_spans_family_and_unicode_range() {
        let mut layout = oryn_js::v8_bindings::BindingLayout::default();
        layout.styles.insert(
            1,
            std::sync::Arc::new(BTreeMap::from([("font-family".into(), "Latin".into())])),
        );
        layout.styles.insert(
            2,
            std::sync::Arc::new(BTreeMap::from([("font-family".into(), "Emoji".into())])),
        );
        layout.resolved_text.push(oryn_common::v4::ResolvedTextRun {
            layout_node: 1,
            text: "A😀".into(),
            aliases: Vec::new(),
            spans: vec![
                oryn_common::v4::ResolvedTextSpan {
                    owner: 1,
                    source_node: None,
                    rendered_start: 0,
                    rendered_end: 1,
                    source_start: 0,
                    source_end: 1,
                    transformed: false,
                    atomic: false,
                },
                oryn_common::v4::ResolvedTextSpan {
                    owner: 2,
                    source_node: None,
                    rendered_start: 1,
                    rendered_end: 5,
                    source_start: 0,
                    source_end: 2,
                    transformed: false,
                    atomic: false,
                },
            ],
        });
        let demand = CssDemand::from_layout(&layout);
        let candidate = |family: &str, start, end| FontCandidate {
            native_rule: None,
            family: family.into(),
            key: family.into(),
            ranges: vec![(start, end)],
            descriptors: String::new(),
            order: 0,
            retained: None,
        };
        assert!(demand.matches(&candidate("Latin", 0, 127)));
        assert!(!demand.matches(&candidate("Latin", 0x1f600, 0x1f64f)));
        assert!(demand.matches(&candidate("Emoji", 0x1f600, 0x1f64f)));
        assert!(!demand.matches(&candidate("Emoji", 0, 127)));
        assert!(!demand.matches(&candidate("Unused", 0, 0x10ffff)));
    }
    #[test]
    fn dynamic_stylesheet_ready_custody_and_replacement_use_native_revision() {
        let mut page =
            RustExecutableDocument::load("<style id=s>body{color:red}</style><body></body>")
                .unwrap();
        let node = page.agent.dom.borrow().find_by_id("s").unwrap();
        let realm = page.top_realm;
        let budget = page.dynamic_scripts.source_bytes.clone();
        let unrelated = budget.load(std::sync::atomic::Ordering::Acquire);
        page.schedule_dynamic_stylesheet(realm, node).unwrap();
        assert!(page.dynamic_stylesheets_pending(Some(realm)));
        assert!(page.dynamic_stylesheets_runnable());
        let job = page.dynamic_stylesheets.back_mut().unwrap();
        #[cfg(unix)]
        let reserved = if let Some(scheduler) = &mut job.retained {
            // Native ready envelopes, not legacy StylesheetResult, own the
            // exact original source reservations through publication/retry.
            assert!(scheduler.has_ready_completion());
            let envelope = scheduler.ready_completions.iter().find_map(|(completion, _)| {
                if let ParserResourceCompletion::RetainedStylesheet(ready) = completion {
                    ready.result.as_ref().ok()
                } else { None }
            }).expect("native original ready source");
            assert!(envelope.tree.as_ref().is_some_and(|tree|
                tree.response.source.as_ref() == "body{color:red}"));
            envelope.reservations.iter().map(|reservation| reservation.bytes).sum::<usize>()
        } else {
            job.ready.as_ref().unwrap()._bytes.as_ref().unwrap().bytes
        };
        #[cfg(not(unix))]
        let reserved = job.ready.as_ref().unwrap()._bytes.as_ref().unwrap().bytes;
        assert!(reserved > 0);
        assert_eq!(
            budget.load(std::sync::atomic::Ordering::Acquire),
            unrelated + reserved
        );
        page.agent
            .host
            .execute("document.getElementById('s').textContent='body{color:blue}'")
            .unwrap();
        page.poll_dynamic_stylesheets().unwrap();
        assert!(!page.dynamic_stylesheets_pending(None));
        assert_eq!(
            page.agent
                .dom
                .borrow()
                .arena()
                .stylesheet_text(node)
                .unwrap(),
            "body{color:blue}"
        );
        assert_eq!(
            budget.load(std::sync::atomic::Ordering::Acquire),
            unrelated,
            "replacement releases only its own reservation"
        );
    }
    #[cfg(unix)]
    #[test]
    fn css_image_demand_uses_native_image_owner_without_competing_binary_job() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><div id=owner></div>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let node = page.agent.dom.borrow().find_by_id("owner").unwrap();
        page.agent
            .dom
            .borrow_mut()
            .set_attribute(
                node,
                "style",
                "background-image:url(first.png);width:10px;height:10px",
            )
            .unwrap();
        page.schedule_dynamic_stylesheet(page.top_realm, node)
            .unwrap();
        let mut source = page.css_dependency_sources.pop_back().unwrap();
        page.dispatch_css_dependency_batch(source.realm, &source.batch, &mut source.attempted)
            .unwrap();
        assert!(page.dynamic_stylesheets.is_empty());
        let layout = page
            .agent
            .host
            .native_css_resource_layout(page.top_realm)
            .unwrap();
        assert!(
            layout
                .css_image_requests
                .iter()
                .any(|url| url.ends_with("first.png"))
        );
        let revision_before = page.agent.dom.borrow().mutation_cursor().0;
        let key = (u64::from(node.generation) << 32) | u64::from(node.slot);
        let computed_before = layout
            .styles
            .get(&key)
            .and_then(|style| style.get("background-image"))
            .cloned();
        let demands_before = layout.css_image_requests.clone();
        page.agent
            .host
            .execute("document.getElementById('owner').style.backgroundImage='url(second.png)'")
            .unwrap();
        // Keep the producer, invalidation and consumer evidence together: an
        // expando-only CSSOM write, stale computed style and stale demand set
        // require different repairs. These reads do not flush extra JS work.
        let (revision_after, inline_after) = {
            let dom = page.agent.dom.borrow();
            (
                dom.mutation_cursor().0,
                dom.arena()
                    .get(node)
                    .unwrap()
                    .attributes
                    .get_null("style")
                    .cloned(),
            )
        };
        let layout = page
            .agent
            .host
            .native_css_resource_layout(page.top_realm)
            .unwrap();
        let computed_after = layout
            .styles
            .get(&key)
            .and_then(|style| style.get("background-image"));
        let evidence = format!(
            "revision {revision_before}->{revision_after}; inline={inline_after:?}; computed {computed_before:?}->{computed_after:?}; demand {demands_before:?}->{:?}",
            layout.css_image_requests
        );
        assert!(
            !layout
                .css_image_requests
                .iter()
                .any(|url| url.ends_with("first.png")),
            "{evidence}"
        );
        assert!(
            layout
                .css_image_requests
                .iter()
                .any(|url| url.ends_with("second.png")),
            "{evidence}"
        );
    }
    #[test]
    fn binary_font_receipt_reaches_layout_and_stale_revision_cannot_publish() {
        let mut page = RustExecutableDocument::load(
            "<style id=s>p{font-family:Fixture}</style><p>Measured</p>",
        )
        .unwrap();
        let node = page.agent.dom.borrow().find_by_id("s").unwrap();
        page.schedule_dynamic_stylesheet(page.top_realm, node)
            .unwrap();
        let mut job = page.dynamic_stylesheets.pop_back().unwrap();
        job.font_family = Some("Fixture".into());
        let bytes = oryn_layout::DEFAULT_FONT_BYTES.to_vec();
        let permit =
            ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, bytes.len())
                .unwrap();
        page.publish_css_binary(
            &mut job,
            CssBinary {
                url: "https://example.test/font.ttf".into(),
                bytes,
            },
            Some(permit),
        )
        .unwrap();
        let (arena, document) = page.agent.dom.borrow().snapshot();
        let parsed = ParsedDocument {
            dom: arena,
            document,
            errors: Vec::new(),
            quirks_mode: html5ever::interface::QuirksMode::NoQuirks,
            events: Vec::new(),
        };
        let mut input = crate::runtime::layout_input(
            &parsed,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            &[],
        );
        page.populate_css_fonts(&mut input).unwrap();
        assert_eq!(input.web_fonts.len(), 1);
        let output = oryn_layout::compute(&input, &oryn_layout::LayoutLimits::default()).unwrap();
        assert_eq!(output.loaded_fonts.len(), 1);
        page.agent
            .host
            .execute("document.getElementById('s').textContent='p{font-family:serif}'")
            .unwrap();
        let bytes = vec![1, 2, 3];
        let permit =
            ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, bytes.len())
                .unwrap();
        page.publish_css_binary(
            &mut job,
            CssBinary {
                url: "https://example.test/stale.png".into(),
                bytes,
            },
            Some(permit),
        )
        .unwrap();
        assert!(page.css_binary_resources.is_empty());
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn xml_pi_completion_reaches_cssom_media_and_production_layout() {
        struct XmlStyles;
        impl crate::network::NetworkBroker for XmlStyles {
            fn send(&self, request: crate::network::NetworkRequest) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                let (mime, body) = if request.url.ends_with("/theme.css") {
                    ("text/css", "root{color:red}")
                } else if request.url.ends_with("/other.css") {
                    ("text/css", "root{color:blue}")
                } else {
                    ("application/xml", "<?xml-stylesheet href='theme.css' media='screen' title='Main'?><root/>")
                };
                Ok(crate::network::NetworkResponse {
                    response_policy: None, timing: None, request_id: request.request_id,
                    status: 200, headers: vec![("content-type".into(), mime.into())],
                    body: body.as_bytes().to_vec(),
                })
            }
        }
        let context = crate::Browser::in_process_probe().new_context_with_broker(
            crate::ContextOptions::default(), crate::network::SharedNetworkBroker::new(XmlStyles));
        let page = context.new_page();
        page.goto("https://example.test/document.xml").await.unwrap();
        assert_eq!(page.evaluate("(()=>{const pi=document.firstChild;return [pi.sheet!==null,document.styleSheets[0]===pi.sheet,pi.sheet.ownerNode===pi,pi.sheet.media.mediaText,pi.sheet.href,getComputedStyle(document.documentElement).color].join('|')})()").await.unwrap(), "true|true|true|screen|https://example.test/theme.css|rgb(255, 0, 0)");
        assert_eq!(page.evaluate("(()=>{const pi=document.firstChild,old=pi.sheet;pi.data=\"href='other.css'\";return [pi.sheet===null,old.ownerNode===null].join('|')})()").await.unwrap(), "true|true");
    }
}

pub(super) struct CachedCssBinary {
    identity: ParserDocumentIdentity,
    dom: SharedDomRuntime,
    node: NodeId,
    revision: u64,
    family: Option<String>,
    font_group: Option<String>,
    retained_font: Option<RetainedFontDemandEpoch>,
    font_descriptors: String,
    font_order: u64,
    resource_attributes: Option<Vec<Option<String>>>,
    binary: CssBinary,
    _bytes: ScriptSourceReservation,
    _metadata: ScriptSourceReservation,
}
impl CachedCssBinary {
    fn live(&self, host: &RawV8Host) -> bool {
        self.retained_font.as_ref().is_none_or(RetainedFontDemandEpoch::current)
            && host.automation_realm_identity(self.identity.realm)
            == Some((
                self.identity.document,
                self.identity.document_generation,
                self.identity.stream_epoch,
                true,
            ))
            && self.dom.borrow().node_document(self.node).ok().flatten()
                == Some(self.identity.document)
            && self.dom.borrow().is_connected(self.node).unwrap_or(false)
            && self
                .dom
                .borrow()
                .arena()
                .stylesheet_revision(self.node)
                .ok()
                == Some(self.revision)
            && self
                .dom
                .borrow()
                .arena()
                .get(self.node)
                .is_ok_and(|record| record.stylesheet_policy_allowed)
            && self.resource_attributes.as_ref().is_none_or(|expected| {
                resource_attributes(&self.dom, self.node).as_ref() == Some(expected)
            })
    }
}
impl RustExecutableDocument {
    fn publish_css_binary_retained(
        &mut self,
        job: &mut StylesheetPreparation,
    ) -> Result<(), HostError> {
        self.css_binary_resources
            .retain(|entry| entry.live(&self.agent.host));
        if !job.live(&self.agent.host) {
            return Ok(());
        }
        let visual_failed=self.css_font_timelines.iter().any(|timeline| {
            timeline.identity.realm == job.identity.realm
                && timeline.node == job.node
                && timeline.revision == job.revision
                && Some(&timeline.font.key) == job.font_group.as_ref()
                && timeline.phase(Instant::now()) == FontDisplayPhase::Failure
        });
        if job.published.is_some() {
            job
                .dom
                .borrow_mut()
                .notify_stylesheet_resource_changed(job.node)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            return self.finish_implicit_font_backing(job);
        }
        let Some(binary) = job.ready.as_ref().and_then(|result| result.binary.as_ref()) else {
            return Ok(());
        };
        let original=self.css_font_timelines.iter().find(|timeline|
            timeline.identity.realm==job.identity.realm&&timeline.node==job.node
            &&timeline.revision==job.revision&&Some(&timeline.font.key)==job.font_group.as_ref())
            .and_then(|timeline|timeline.font.native_rule.clone());
        if job.prepared_implicit_backing.is_none() && let (Some(rule),Some(epoch))=(original,job.retained_font.clone()){
            job.prepared_implicit_backing=Some(self.prepare_implicit_font_backing(job.identity.realm,rule,epoch,binary)?);
        }
        if job.prepared_implicit_backing.is_some(){
            let provider=self.agent.host.native_layout_provider_in_realm(job.identity.realm).ok_or(HostError::CachedAuthorityExpired)?;
            let shared=self.agent.host.native_retained_admission_for_realm(job.identity.realm)?.ok_or(HostError::CachedAuthorityExpired)?;
            let _scope=self.agent.host.retained_allocation_scope(job.identity.realm)?;
            let store=provider.native_backend().and_then(|backend|backend.downcast_ref::<LiveLayoutProvider>())
                .ok_or(HostError::CachedAuthorityExpired)?.implicit_font_store(shared.as_ref()).map_err(HostError::Execution)?;
            prepare_implicit_font_store(&store,shared.as_ref())?;
        }
        if job
            .ready
            .as_ref()
            .and_then(|result| result._bytes.as_ref())
            .is_none()
        {
            return Err(HostError::Execution(
                "CSS binary response has no byte custody".into(),
            ));
        }
        let replaces = self.css_binary_resources.iter().any(|entry| {
            entry.identity.realm == job.identity.realm
                && entry.node == job.node
                && entry.binary.url == binary.url
                && entry.font_group == job.font_group
        });
        if !replaces && self.css_binary_resources.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "CSS binary resources".into(),
            });
        }
        self.css_binary_resources
            .try_reserve(1)
            .map_err(|_| HostError::LimitExceeded {
                resource: "CSS binary resource allocation".into(),
            })?;
        let metadata = ScriptSourceReservation::reserve(
            &self.dynamic_scripts.source_bytes,
            job.font_descriptors
                .len()
                .saturating_add(job.font_family.as_ref().map_or(0, String::len))
                .saturating_add(job.font_group.as_ref().map_or(0, String::len)),
        )
        .ok_or_else(|| HostError::LimitExceeded {
            resource: "cached font descriptor bytes".into(),
        })?;
        #[cfg(test)]
        css_publication_failure(1)?;
        if !visual_failed && let Some(family) = &job.font_family {
            use sha2::Digest;
            let custody = ScriptSourceReservation::reserve(
                &self.dynamic_scripts.source_bytes,
                binary
                    .bytes
                    .len()
                    .saturating_add(job.font_descriptors.len()),
            )
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "native layout font bytes".into(),
            })?;
            self.agent.host.install_native_css_font(
                job.identity.realm,
                job.identity.document,
                job.node,
                job.revision,
                oryn_js::v8_bindings::BindingFontResource {
                    pending: false,
                    blocked: false,
                    order: job.font_order,
                    descriptors: job.font_descriptors.clone(),
                    family: family.clone(),
                    source_url: binary.url.clone(),
                    sha256: format!("{:x}", sha2::Sha256::digest(&binary.bytes)),
                    bytes: binary.bytes.clone(),
                },
                std::sync::Arc::new(custody),
                job.retained_font.clone(),
            )?;
        }
        self.css_binary_resources.retain(|entry| {
            !(entry.identity.realm == job.identity.realm
                && entry.node == job.node
                && entry.binary.url == binary.url
                && entry.font_group == job.font_group)
        });
        let result = job.ready.as_mut().expect("retained binary result");
        let binary = result.binary.take().expect("retained binary");
        let bytes = result._bytes.take().expect("retained binary byte custody");
        self.css_binary_resources.push(CachedCssBinary {
            identity: job.identity,
            dom: job.dom.clone(),
            node: job.node,
            revision: job.revision,
            family: job.font_family.clone(),
            font_group: job.font_group.clone(),
            retained_font: job.retained_font.clone(),
            font_descriptors: job.font_descriptors.clone(),
            font_order: job.font_order,
            resource_attributes: job.resource_attributes.clone(),
            binary,
            _bytes: bytes,
            _metadata: metadata,
        });
        for timeline in &mut self.css_font_timelines {
            if timeline.identity.realm == job.identity.realm
                && timeline.node == job.node
                && timeline.revision == job.revision
                && Some(&timeline.font.key) == job.font_group.as_ref()
            {
                if !visual_failed {
                    timeline.loaded = true;
                    timeline.published = Some(FontDisplayPhase::Loaded);
                }
            }
        }
        job.published = Some(None);
        #[cfg(test)]
        css_publication_failure(2)?;
        job.dom
            .borrow_mut()
            .notify_stylesheet_resource_changed(job.node)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        self.finish_implicit_font_backing(job)?;
        Ok(())
    }
    #[cfg(test)]
    fn publish_css_binary(
        &mut self,
        job: &mut StylesheetPreparation,
        binary: CssBinary,
        bytes: Option<ScriptSourceReservation>,
    ) -> Result<(), HostError> {
        job.published = None;
        job.ready = Some(StylesheetResult {
            binary: Some(binary),
            result: Ok((String::new(), String::new(), Vec::new())),
            _bytes: bytes,
            fatal: None,
        });
        self.publish_css_binary_retained(job)
    }
    pub(crate) fn populate_css_fonts(
        &self,
        input: &mut oryn_layout::LayoutInput,
    ) -> Result<(), HostError> {
        let document = self
            .automation_scope
            .as_ref()
            .map(|scope| scope.document_root)
            .unwrap_or(self.document.document);
        let mut fonts = Vec::new();
        let mut order = BTreeMap::new();
        for entry in self
            .css_binary_resources
            .iter()
            .filter(|entry| entry.identity.document == document && entry.live(&self.agent.host))
        {
            let Some(family) = &entry.family else {
                continue;
            };
            if !self
                .agent
                .host
                .css_font_owner_applicable(entry.identity.realm, entry.node)?
            {
                continue;
            }
            if order.is_empty() {
                order = entry
                    .dom
                    .borrow()
                    .arena()
                    .descendants_inclusive(document)
                    .map_err(|error| HostError::Execution(error.to_string()))?
                    .into_iter()
                    .enumerate()
                    .map(|(position, node)| (node, position))
                    .collect();
            }
            use sha2::Digest;
            fonts.push((
                entry.node,
                oryn_layout::WebFont {
                    pending: false,
                    blocked: false,
                    order: entry.font_order,
                    descriptors: entry.font_descriptors.clone(),
                    family: family.clone(),
                    source_url: entry.binary.url.clone(),
                    sha256: format!("{:x}", sha2::Sha256::digest(&entry.binary.bytes)),
                    bytes: entry.binary.bytes.clone(),
                },
            ));
        }
        for timeline in &self.css_font_timelines {
            if timeline.loaded
                || timeline.identity.document != document
                || !timeline.valid(&self.agent.host)
                || !self
                    .agent
                    .host
                    .css_font_owner_applicable(timeline.identity.realm, timeline.node)?
            {
                continue;
            }
            if order.is_empty() {
                order = timeline
                    .dom
                    .borrow()
                    .arena()
                    .descendants_inclusive(document)
                    .map_err(|error| HostError::Execution(error.to_string()))?
                    .into_iter()
                    .enumerate()
                    .map(|(position, node)| (node, position))
                    .collect();
            }
            fonts.push((
                timeline.node,
                oryn_layout::WebFont {
                    pending: true,
                    blocked: timeline.published == Some(FontDisplayPhase::Block),
                    order: timeline.font.order,
                    descriptors: timeline.font.descriptors.clone(),
                    family: timeline.font.family.clone(),
                    source_url: format!("oryn-font-face:{}", timeline.font.key),
                    sha256: String::new(),
                    bytes: Vec::new(),
                },
            ));
        }
        fonts.sort_by_key(|(node, font)| {
            (order.get(node).copied().unwrap_or(usize::MAX), font.order)
        });
        input
            .web_fonts
            .extend(fonts.into_iter().map(|(_, font)| font));
        Ok(())
    }
}

impl RustExecutableDocument {
    fn begin_font_display(
        &mut self,
        identity: ParserDocumentIdentity,
        dom: &SharedDomRuntime,
        batch: &CssDependencyBatch,
        font: &FontCandidate,
    ) -> Result<(), HostError> {
        self.css_font_timelines
            .retain(|timeline| timeline.valid(&self.agent.host)||timeline.implicit_tokens.is_some());
        if self.css_font_timelines.iter().any(|timeline| {
            timeline.valid(&self.agent.host)&&timeline.identity.realm == identity.realm
                && timeline.node == batch.node
                && timeline.revision == batch.revision
                && timeline.font.key == font.key
        }) {
            return Ok(());
        }
        if self.css_font_timelines.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "font display timelines".into(),
            });
        }
        let bytes = ScriptSourceReservation::reserve(
            &self.dynamic_scripts.source_bytes,
            font.descriptors
                .len()
                .saturating_add(font.family.len())
                .saturating_add(font.key.len())
                .saturating_add(font.ranges.len().saturating_mul(8)),
        )
        .ok_or_else(|| HostError::LimitExceeded {
            resource: "font display metadata".into(),
        })?;
        self.css_font_timelines.push(CssFontTimeline {
            implicit_tokens: None,
            implicit_started: false,
            implicit_terminal: None,
            identity,
            dom: dom.clone(),
            node: batch.node,
            revision: batch.revision,
            font: font.clone(),
            started: Instant::now(),
            periods: FontDisplayPeriods::parse(&font.descriptors),
            loaded: false,
            failed: false,
            published: None,
            _node: ParserScriptLease::new(dom, batch.node)?,
            _bytes: bytes,
        });
        self.poll_font_display()
    }
    pub(super) fn cancel_pending_font_display(&mut self) -> Result<(), HostError> {
        for timeline in &mut self.css_font_timelines {
            if !timeline.loaded {
                timeline.failed = true;
                timeline.implicit_terminal=Some(false);
            }
        }
        self.poll_font_display()
    }
    pub(super) fn stylesheet_retry_delay(&self) -> Option<Duration> {
        let now = Instant::now();
        self.css_dependency_sources
            .iter()
            .filter(|source| !self.agent.host.realm_is_parked(source.realm))
            .filter_map(|source| source.retry)
            .chain(
                self.dynamic_stylesheets
                    .iter()
                    .filter(|job| !self.agent.host.realm_is_parked(job.identity.realm))
                    .filter_map(|job| job.retry),
            )
            .map(|retry| retry.saturating_duration_since(now))
            .min()
    }
    pub(super) fn font_display_delay(&self) -> Option<Duration> {
        let now = Instant::now();
        self.css_font_timelines
            .iter()
            .filter(|timeline| {
                timeline.valid(&self.agent.host)
                    && !self.agent.host.realm_is_parked(timeline.identity.realm)
            })
            .filter_map(|timeline| timeline.delay(now))
            .min()
    }
    fn poll_font_display(&mut self) -> Result<(), HostError> {
        for timeline in &mut self.css_font_timelines {
            if !timeline.valid(&self.agent.host)&&let Some(tokens)=&timeline.implicit_tokens{
                self.agent.host.complete_implicit_font_tokens(timeline.identity.realm,&tokens.tokens,false)?;
                timeline.implicit_tokens=None;
            }
        }
        self.css_font_timelines
            .retain(|timeline| timeline.valid(&self.agent.host));
        let now = Instant::now();
        for timeline in &mut self.css_font_timelines {
            if self.agent.host.realm_is_parked(timeline.identity.realm) {
                continue;
            }
            let terminal=timeline.implicit_terminal.or(timeline.loaded.then_some(true));
            if let Some(success)=terminal && let Some(tokens)=&timeline.implicit_tokens{
                self.agent.host.complete_implicit_font_tokens(timeline.identity.realm,&tokens.tokens,success)?;
                timeline.implicit_tokens=None;
            }
            let phase = timeline.phase(now);
            if timeline.published == Some(phase) {
                continue;
            }
            if phase != FontDisplayPhase::Loaded {
                let font = oryn_js::v8_bindings::BindingFontResource {
                    pending: true,
                    blocked: phase == FontDisplayPhase::Block,
                    order: timeline.font.order,
                    descriptors: timeline.font.descriptors.clone(),
                    family: timeline.font.family.clone(),
                    source_url: format!("oryn-font-face:{}", timeline.font.key),
                    sha256: String::new(),
                    bytes: Vec::new(),
                };
                let custody = ScriptSourceReservation::reserve(
                    &self.dynamic_scripts.source_bytes,
                    font.descriptors.len().saturating_add(font.family.len()),
                )
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "font display layout metadata".into(),
                })?;
                self.agent.host.install_native_css_font(
                    timeline.identity.realm,
                    timeline.identity.document,
                    timeline.node,
                    timeline.revision,
                    font,
                    std::sync::Arc::new(custody),
                    timeline.font.retained.clone(),
                )?;
            }
            timeline
                .dom
                .borrow_mut()
                .notify_stylesheet_resource_changed(timeline.node)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            timeline.published = Some(phase);
        }
        Ok(())
    }
}

#[cfg(test)]
thread_local! {static CSS_PUBLICATION_FAILURE:std::cell::Cell<u8>=const{std::cell::Cell::new(0)};}
#[cfg(test)]
fn css_publication_failure(stage: u8) -> Result<(), HostError> {
    CSS_PUBLICATION_FAILURE.with(|slot| {
        if slot.get() == stage {
            slot.set(0);
            Err(HostError::Execution(
                "injected CSS publication failure".into(),
            ))
        } else {
            Ok(())
        }
    })
}
#[cfg(test)]
mod retained_css_publication_tests {
    use super::*;
    #[test]
    fn binary_retains_bytes_before_commit_and_never_reinstalls_after_commit() {
        let mut page =
            RustExecutableDocument::load("<style id=s>p{color:green}</style><p>Text</p>").unwrap();
        let node = page.agent.dom.borrow().find_by_id("s").unwrap();
        page.schedule_dynamic_stylesheet(page.top_realm, node)
            .unwrap();
        let mut job = page.dynamic_stylesheets.pop_back().unwrap();
        job.install = false;
        let binary = CssBinary {
            url: "https://example.test/resource.bin".into(),
            bytes: vec![1, 2, 3],
        };
        let bytes =
            ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, 3).unwrap();
        job.ready = Some(StylesheetResult {
            binary: Some(binary),
            result: Ok((String::new(), String::new(), Vec::new())),
            _bytes: Some(bytes),
            fatal: None,
        });
        CSS_PUBLICATION_FAILURE.with(|slot| slot.set(1));
        assert!(page.publish_css_binary_retained(&mut job).is_err());
        assert!(job.ready.as_ref().unwrap().binary.is_some());
        assert!(job.ready.as_ref().unwrap()._bytes.is_some());
        assert!(page.css_binary_resources.is_empty());
        CSS_PUBLICATION_FAILURE.with(|slot| slot.set(2));
        assert!(page.publish_css_binary_retained(&mut job).is_err());
        assert!(job.published.is_some());
        assert!(job.ready.as_ref().unwrap().binary.is_none());
        assert_eq!(page.css_binary_resources.len(), 1);
        page.publish_css_binary_retained(&mut job).unwrap();
        assert_eq!(page.css_binary_resources.len(), 1);
        assert_eq!(page.css_binary_resources[0].binary.bytes, vec![1, 2, 3]);
    }
    #[test]
    fn native_batch_transfer_restores_original_body_when_owner_admission_fails() {
        let mut page =
            RustExecutableDocument::load("<style id=s>p{color:green}</style><p>Text</p>").unwrap();
        let node = page.agent.dom.borrow().find_by_id("s").unwrap();
        let revision = page
            .agent
            .dom
            .borrow()
            .arena()
            .stylesheet_revision(node)
            .unwrap();
        page.agent
            .host
            .queue_css_dependencies(node, revision, Vec::new())
            .unwrap();
        CSS_PUBLICATION_FAILURE.with(|slot| slot.set(4));
        assert!(page.poll_dynamic_stylesheets().is_err());
        assert!(page.css_dependency_sources.is_empty());
        assert!(page.agent.host.take_css_dependencies().is_empty());
        // No native execution occurs during this wait; the retained admission has a
        // real deadline and does not depend on an unrelated DOM mutation to resume.
        std::thread::sleep(Duration::from_millis(30));
        page.poll_dynamic_stylesheets().unwrap();
        assert_eq!(page.css_dependency_sources.len(), 1);
        assert_eq!(page.css_dependency_sources[0].batch.node, node);
        assert_eq!(page.css_dependency_sources[0].batch.revision, revision);
    }
    #[test]
    fn dependency_admission_failure_retains_source_with_bounded_retry() {
        let mut page =
            RustExecutableDocument::load("<style id=s>p{color:green}</style><p>Text</p>").unwrap();
        let realm = page.top_realm;
        let node = page.agent.dom.borrow().find_by_id("s").unwrap();
        let revision = page
            .agent
            .dom
            .borrow()
            .arena()
            .stylesheet_revision(node)
            .unwrap();
        let epoch = page.agent.host.document_stream_epoch(realm).unwrap();
        let bytes =
            ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, 0).unwrap();
        page.schedule_css_dependency_batch(
            realm,
            CssDependencyBatch {
                node,
                revision,
                epoch,
                dependencies: Vec::new(),
                _allocation: None,
                _bytes: bytes,
            },
        )
        .unwrap();
        CSS_PUBLICATION_FAILURE.with(|slot| slot.set(3));
        assert!(page.poll_dynamic_stylesheets().is_err());
        assert_eq!(page.css_dependency_sources.len(), 1);
        assert!(page.css_dependency_sources[0].evaluated.is_none());
        assert!(
            page.stylesheet_retry_delay()
                .is_some_and(|delay| delay > Duration::ZERO)
        );
        page.css_dependency_sources[0].retry = Some(Instant::now());
        page.poll_dynamic_stylesheets().unwrap();
        assert!(page.css_dependency_sources[0].evaluated.is_some());
        assert!(page.css_dependency_sources[0].retry.is_none());
    }
}
