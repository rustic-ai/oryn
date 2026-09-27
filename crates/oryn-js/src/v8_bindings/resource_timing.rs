//! Native, document-owned resource timeline and immutable entry snapshots.
use super::*;
#[derive(Clone)]
pub(super) struct Entry {
    pub navigation: Option<crate::navigation_timing::NavigationTiming>,
    pub render_blocking: bool,
    pub kind: &'static str,
    pub name: String,
    pub initiator: String,
    pub start: f64,
    pub end: f64,
    pub bytes: u64,
    pub status: u16,
    pub details: Option<crate::platform::ResourceTimingMetadata>,
}
impl Entry {
    pub(super) fn name_matches(&self, name: &str) -> bool {
        self.navigation
            .as_ref()
            .map_or_else(|| self.name == name, |record| record.name_matches(name))
    }
}
struct Span {
    entry: Entry,
    visible: bool,
}
enum Operation {
    Start(Box<Span>),
    Read(u64),
    Abort(u64),
}
pub(super) struct State {
    navigation: Option<Entry>,
    entries: Vec<Entry>,
    paint: Vec<Entry>,
    limit: usize,
    operations: BTreeMap<HostOperationId, Operation>,
    streams: BTreeMap<u64, Span>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            navigation: None,
            entries: Vec::new(),
            paint: Vec::new(),
            limit: 250,
            operations: BTreeMap::new(),
            streams: BTreeMap::new(),
        }
    }
}
#[cfg(test)]
pub(super) fn external(
    state: &V8BindingState,
    name: String,
    initiator: String,
    start: f64,
    end: f64,
    details: Option<crate::platform::ResourceTimingMetadata>,
) {
    external_with_status(state, name, initiator, start, end, details, false)
}
pub(super) fn external_with_status(
    state: &V8BindingState,
    name: String,
    initiator: String,
    start: f64,
    end: f64,
    details: Option<crate::platform::ResourceTimingMetadata>,
    render_blocking: bool,
) {
    let details = details.filter(|details| details.allow_details);
    let bytes = details
        .as_ref()
        .and_then(|details| details.decoded_body_size)
        .unwrap_or(0);
    let status = details
        .as_ref()
        .map_or(0, |details| details.response_status);
    let mut timing = state.resource_timing.borrow_mut();
    let entry = Entry {
        navigation: None,
        render_blocking,
        kind: "resource",
        name,
        initiator,
        start: (start * 10.).floor() / 10.,
        end: (end * 10.).floor() / 10.,
        bytes,
        status,
        details,
    };
    performance_observer::publish(state, &entry);
    if timing.entries.len() < timing.limit {
        timing.entries.push(entry);
    }
}
fn push(state: &V8BindingState, mut span: Span) {
    span.entry.end = state.performance_now();
    if !span.visible {
        span.entry.bytes = 0;
        span.entry.status = 0;
        span.entry.details = None;
    }
    let mut timing = state.resource_timing.borrow_mut();
    performance_observer::publish(state, &span.entry);
    if timing.entries.len() < timing.limit {
        timing.entries.push(span.entry);
    }
}
pub(super) fn admit(
    state: &V8BindingState,
    id: HostOperationId,
    request: &PlatformHostRequest,
    pending: &PendingPlatformOperation,
) {
    let xslt = matches!(request, PlatformHostRequest::XsltStart { .. });
    let operation = match request {
        PlatformHostRequest::PrefetchStart { request }
        | PlatformHostRequest::ImagePreloadStart { request, .. }
        | PlatformHostRequest::HttpStart { request }
        | PlatformHostRequest::XsltStart { request }
        | PlatformHostRequest::ImageStart { request, .. }
        | PlatformHostRequest::MediaStart { request, .. } => {
            let initiator = if xslt {
                "other"
            } else {
                match pending {
                    PendingPlatformOperation::Preload(_) => "link",
                    PendingPlatformOperation::MediaResource(p) => p.initiator(),
                    PendingPlatformOperation::TextTrack(p) => p.initiator(),
                    PendingPlatformOperation::CssImage(_) => "css",
                    PendingPlatformOperation::ImageStart { .. } => "img",
                    PendingPlatformOperation::FetchStart { .. } => "fetch",
                    _ => "xmlhttprequest",
                }
            };
            Operation::Start(Box::new(Span {
                entry: Entry {
                    navigation: None,
                    render_blocking: false,
                    kind: "resource",
                    name: request.url.clone(),
                    initiator: initiator.into(),
                    start: state.performance_now(),
                    end: 0.,
                    bytes: 0,
                    status: 0,
                    details: None,
                },
                visible: false,
            }))
        }
        PlatformHostRequest::HttpRead { stream_id, .. } => Operation::Read(*stream_id),
        PlatformHostRequest::HttpAbort { stream_id } => Operation::Abort(*stream_id),
        _ => return,
    };
    state
        .resource_timing
        .borrow_mut()
        .operations
        .insert(id, operation);
}
pub(super) fn complete(
    state: &V8BindingState,
    id: HostOperationId,
    result: &Result<PlatformHostResult, String>,
) {
    let operation = state.resource_timing.borrow_mut().operations.remove(&id);
    let Some(operation) = operation else { return };
    match (operation, result) {
        (
            Operation::Start(span),
            Ok(PlatformHostResult::HttpStarted {
                stream_id,
                url,
                status,
                headers,
                internal_response,
                timing,
                ..
            }),
        ) => {
            let mut span = *span;
            let own = state
                .request_initiator_url()
                .ok()
                .and_then(|v| Url::parse(&v).ok());
            let target = Url::parse(url).ok();
            let origin = own.as_ref().map(|v| v.origin().ascii_serialization());
            span.visible = timing
                .as_ref()
                .map(|timing| timing.allow_details)
                .unwrap_or_else(|| {
                    internal_response.is_none()
                        && (own
                            .zip(target)
                            .is_some_and(|(a, b)| a.origin() == b.origin())
                            || headers
                                .iter()
                                .filter(|(n, _)| n.eq_ignore_ascii_case("timing-allow-origin"))
                                .any(|(_, v)| {
                                    v.split(',').any(|v| {
                                        v.trim() == "*" || origin.as_deref() == Some(v.trim())
                                    })
                                }))
                });
            span.entry.details = timing.as_deref().cloned();
            span.entry.status = timing
                .as_ref()
                .map_or(*status, |timing| timing.response_status);
            state
                .resource_timing
                .borrow_mut()
                .streams
                .insert(*stream_id, span);
        }
        (
            Operation::Read(stream),
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size,
                stream_id,
                bytes,
                done,
            }),
        ) if stream == *stream_id => {
            let mut span = state.resource_timing.borrow_mut().streams.remove(&stream);
            if let Some(mut span) = span.take() {
                span.entry.bytes = span.entry.bytes.saturating_add(bytes.len() as u64);
                if *done {
                    if let Some(details) = &mut span.entry.details {
                        details.encoded_body_size = encoded_body_size.or(details.encoded_body_size);
                    }
                    push(state, span)
                } else {
                    state
                        .resource_timing
                        .borrow_mut()
                        .streams
                        .insert(stream, span);
                }
            }
        }
        // A foreign receipt cannot terminate or publish the requested stream.
        (Operation::Read(_), Ok(PlatformHostResult::HttpChunk { .. })) => {}
        (Operation::Start(span), _) => push(state, *span),
        (Operation::Read(stream) | Operation::Abort(stream), _) => {
            let span = state.resource_timing.borrow_mut().streams.remove(&stream);
            if let Some(span) = span {
                push(state, span)
            }
        }
    }
}
/// Called only after a native paint scene has been accepted for this exact
/// Document. Forced style/layout queries are not paint notifications.
pub(super) fn paint(state: &V8BindingState, contentful: bool) {
    if !state.document_rendering_allowed() {
        return;
    }
    let time = state.performance_now();
    let mut timing = state.resource_timing.borrow_mut();
    for name in [
        Some("first-paint"),
        contentful.then_some("first-contentful-paint"),
    ]
    .into_iter()
    .flatten()
    {
        if timing.paint.iter().any(|entry| entry.name == name) {
            continue;
        }
        let entry = Entry {
            navigation: None,
            render_blocking: false,
            kind: "paint",
            name: name.into(),
            initiator: String::new(),
            start: time,
            end: time,
            bytes: 0,
            status: 0,
            details: None,
        };
        performance_observer::publish(state, &entry);
        timing.paint.push(entry);
    }
}

pub(super) fn buffered(state: &V8BindingState) -> Vec<Entry> {
    let timing = state.resource_timing.borrow();
    timing
        .navigation
        .iter()
        .filter(|entry| {
            entry
                .navigation
                .as_ref()
                .is_some_and(|record| record.completed())
        })
        .chain(timing.entries.iter())
        .chain(timing.paint.iter())
        .cloned()
        .collect()
}
pub(super) fn navigation(
    state: &V8BindingState,
) -> Option<crate::navigation_timing::NavigationTiming> {
    state
        .resource_timing
        .borrow()
        .navigation
        .as_ref()
        .and_then(|entry| entry.navigation.clone())
}
pub(super) fn begin_navigation(
    state: &V8BindingState,
    record: crate::navigation_timing::NavigationTiming,
) {
    if state.binding_global != BindingGlobal::Window {
        return;
    }
    state.resource_timing.borrow_mut().navigation = Some(Entry {
        navigation: Some(record),
        render_blocking: false,
        kind: "navigation",
        name: String::new(),
        initiator: "navigation".into(),
        start: 0.,
        end: 0.,
        bytes: 0,
        status: 0,
        details: None,
    });
}
pub(super) fn navigation_stage(state: &V8BindingState, stage: crate::navigation_timing::Stage) {
    let entry = state.resource_timing.borrow().navigation.clone();
    let Some(entry) = entry else { return };
    let Some(record) = entry.navigation.as_ref() else {
        return;
    };
    if record.mark(stage, Instant::now()) && stage == crate::navigation_timing::Stage::LoadEnd {
        performance_observer::publish(state, &entry);
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let id = register_platform_object(state, PlatformObjectRecord::Performance);
    let object = new_platform_object(scope, state, "Performance", id)?;
    let name = v8::String::new(scope, "performance")
        .ok_or(V8BindingError::Allocation("performance name"))?;
    context
        .global(scope)
        .create_data_property(scope, name.into(), object.into());
    Ok(())
}
fn server_metrics<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    metrics: &[crate::server_timing::Metric],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let mut values = Vec::with_capacity(metrics.len());
    for metric in metrics {
        let id = register_platform_object(
            state,
            PlatformObjectRecord::PerformanceServerTiming(metric.clone()),
        );
        values.push(new_platform_object(scope, state, "PerformanceServerTiming", id)?.into());
    }
    let array = v8::Array::new_with_elements(scope, &values);
    if array.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return Err(V8BindingError::Allocation("server timing array"));
    }
    Ok(array.into())
}
/// Project authenticated transport timestamps onto the original request clock.
/// A replay without the original producer anchor cannot acquire a worker stamp.
fn resource_worker_start(entry: &Entry) -> f64 {
    let Some(details) = entry
        .details
        .as_ref()
        .filter(|details| details.allow_details)
    else {
        return 0.;
    };
    let Some(origin) = details.producer_clock_micros else {
        return 0.;
    };
    let Some(worker) = details
        .transport
        .as_ref()
        .and_then(|transport| transport.worker_start_clock_micros)
    else {
        return 0.;
    };
    let Some(elapsed) = worker.checked_sub(origin) else {
        return 0.;
    };
    if elapsed > details.response_start_micros {
        return 0.;
    }
    ((entry.start + elapsed as f64 / 1000.) * 10.).floor() / 10.
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    // serverTiming builds new native wrappers; release the registry before
    // projecting any attribute, including toJSON's internal getter calls.
    let record = {
        let record = platform_object(state, id)?;
        match &*record {
            PlatformObjectRecord::Performance => PlatformObjectRecord::Performance,
            PlatformObjectRecord::PerformanceServerTiming(metric) => {
                PlatformObjectRecord::PerformanceServerTiming(metric.clone())
            }
            PlatformObjectRecord::PerformanceEntry(entry) => {
                PlatformObjectRecord::PerformanceEntry(entry.clone())
            }
            _ => return Err(V8BindingError::Allocation("performance receiver")),
        }
    };
    match &record {
        PlatformObjectRecord::PerformanceServerTiming(metric) => match member {
            "name" => v8_string(scope, &metric.name),
            "description" => v8_string(scope, &metric.description),
            "duration" => Ok(v8::Number::new(scope, metric.duration).into()),
            _ => Err(V8BindingError::Allocation("server timing attribute")),
        },
        PlatformObjectRecord::Performance if member == "timeOrigin" => {
            Ok(v8::Number::new(scope, state.time_origin_ms.get()).into())
        }
        PlatformObjectRecord::PerformanceEntry(entry) if entry.navigation.is_some() => {
            let snapshot = entry.navigation.as_ref().unwrap().snapshot();
            match member {
                "serverTiming" => server_metrics(scope, state, &snapshot.server_timing),
                "name" => v8_string(scope, &snapshot.name),
                "entryType" => v8_string(scope, "navigation"),
                "type" => v8_string(scope, snapshot.kind),
                "initiatorType" => v8_string(scope, "navigation"),
                "nextHopProtocol" => v8_string(scope, &snapshot.protocol),
                "contentType" => v8_string(scope, &snapshot.content_type),
                "deliveryType" => v8_string(scope, snapshot.delivery_type),
                "renderBlockingStatus" => v8_string(scope, "non-blocking"),
                _ => Ok(
                    v8::Number::new(scope, snapshot.values.get(member).copied().unwrap_or(0.))
                        .into(),
                ),
            }
        }
        PlatformObjectRecord::PerformanceEntry(entry) => match member {
            "serverTiming" => server_metrics(
                scope,
                state,
                entry
                    .details
                    .as_ref()
                    .filter(|details| details.allow_details)
                    .map_or(&[], |details| details.server_timing.as_slice()),
            ),
            "name" => v8_string(scope, &entry.name),
            "entryType" => v8_string(scope, entry.kind),
            "initiatorType" => v8_string(scope, &entry.initiator),
            "renderBlockingStatus" => v8_string(
                scope,
                if entry.render_blocking {
                    "blocking"
                } else {
                    "non-blocking"
                },
            ),
            "contentType" => v8_string(
                scope,
                entry
                    .details
                    .as_ref()
                    .map_or("", |timing| timing.content_type.as_str()),
            ),
            "nextHopProtocol" => v8_string(
                scope,
                entry
                    .details
                    .as_ref()
                    .and_then(|timing| timing.transport.as_ref())
                    .map_or("", |timing| timing.next_hop_protocol.as_str()),
            ),
            "deliveryType" => v8_string(
                scope,
                entry
                    .details
                    .as_ref()
                    .and_then(|details| details.transport.as_ref())
                    .map_or("", |transport| transport.delivery.delivery_type()),
            ),
            _ => Ok(v8::Number::new(
                scope,
                match member {
                    "workerStart" => resource_worker_start(entry),
                    "redirectStart" | "redirectEnd" => entry
                        .details
                        .as_ref()
                        .filter(|details| details.allow_details)
                        .and_then(|details| details.navigation_phases.as_ref())
                        .and_then(|phases| {
                            if member == "redirectStart" {
                                phases.redirect_start_micros
                            } else {
                                phases.redirect_end_micros
                            }
                        })
                        .map_or(0., |micros| {
                            ((entry.start + micros as f64 / 1000.) * 10.).floor() / 10.
                        }),
                    "startTime" => entry.start,
                    "fetchStart" => {
                        entry.start
                            + entry
                                .details
                                .as_ref()
                                .and_then(|details| details.navigation_phases.as_ref())
                                .map_or(0, |phases| phases.fetch_start_micros)
                                as f64
                                / 1000.
                    }
                    "responseEnd" => entry.end,
                    "duration" => (entry.end - entry.start).max(0.),
                    "domainLookupStart"
                    | "domainLookupEnd"
                    | "connectStart"
                    | "connectEnd"
                    | "secureConnectionStart"
                    | "requestStart" => entry.details.as_ref().map_or(0., |details| {
                        let transport = details.transport.as_ref();
                        let phase = transport.and_then(|timing| match member {
                            "domainLookupStart" => timing.dns_start,
                            "domainLookupEnd" => timing.dns_end,
                            "connectStart" => timing.connect_start,
                            "connectEnd" => timing.secure_end.or(timing.connect_end),
                            "secureConnectionStart" => timing.secure_start,
                            "requestStart" => Some(timing.request_start),
                            _ => None,
                        });
                        if member == "secureConnectionStart" && phase.is_none() {
                            0.
                        } else {
                            entry.start + phase.unwrap_or(0) as f64 / 1000.
                        }
                    }),
                    "responseStart" => entry.details.as_ref().map_or(0., |timing| {
                        entry.start + timing.response_start_micros as f64 / 1000.
                    }),
                    "encodedBodySize" => entry
                        .details
                        .as_ref()
                        .and_then(|timing| timing.encoded_body_size)
                        .unwrap_or(0) as f64,
                    "transferSize" => entry.details.as_ref().map_or(0, |details| {
                        details.transport.as_ref().map_or(0, |transport| {
                            transport
                                .delivery
                                .transfer_size(details.encoded_body_size.unwrap_or(0))
                        })
                    }) as f64,
                    "decodedBodySize" => entry.bytes as f64,
                    "responseStatus" => f64::from(entry.status),
                    _ => 0.,
                },
            )
            .into()),
        },
        _ => Err(V8BindingError::Allocation("performance receiver")),
    }
}
pub(super) fn entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    entry: Entry,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let interface = if entry.kind == "navigation" {
        "PerformanceNavigationTiming"
    } else if entry.kind == "paint" {
        "PerformancePaintTiming"
    } else {
        "PerformanceResourceTiming"
    };
    let id = register_platform_object(state, PlatformObjectRecord::PerformanceEntry(entry));
    new_platform_object(scope, state, interface, id)
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let is_performance = matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::Performance
    );
    if member == "toJSON" {
        let output = v8::Object::new(scope);
        let mut fields = if is_performance {
            vec!["timeOrigin"]
        } else if matches!(
            &*platform_object(state, id)?,
            PlatformObjectRecord::PerformanceServerTiming(_)
        ) {
            vec!["name", "duration", "description"]
        } else if matches!(&*platform_object(state,id)?,PlatformObjectRecord::PerformanceEntry(entry)if entry.kind=="paint")
        {
            vec!["name", "entryType", "startTime", "duration"]
        } else {
            vec![
                "name",
                "entryType",
                "startTime",
                "duration",
                "initiatorType",
                "nextHopProtocol",
                "deliveryType",
                "workerStart",
                "redirectStart",
                "redirectEnd",
                "fetchStart",
                "domainLookupStart",
                "domainLookupEnd",
                "connectStart",
                "connectEnd",
                "secureConnectionStart",
                "requestStart",
                "responseStart",
                "responseEnd",
                "contentType",
                "decodedBodySize",
                "encodedBodySize",
                "transferSize",
                "responseStatus",
                "renderBlockingStatus",
                "serverTiming",
            ]
        };
        if matches!(&*platform_object(state,id)?,PlatformObjectRecord::PerformanceEntry(entry)if entry.kind=="navigation")
        {
            fields.extend([
                "type",
                "redirectCount",
                "unloadEventStart",
                "unloadEventEnd",
                "domInteractive",
                "domContentLoadedEventStart",
                "domContentLoadedEventEnd",
                "domComplete",
                "loadEventStart",
                "loadEventEnd",
                "activationStart",
                "criticalCHRestart",
            ])
        };
        for field in fields {
            let value = get(scope, state, object, field)?;
            let key = v8::String::new(scope, field)
                .ok_or(V8BindingError::Allocation("performance JSON"))?;
            output.create_data_property(scope, key.into(), value);
        }
        return Ok(Some(output.into()));
    }
    if !is_performance {
        return Err(V8BindingError::Allocation("Performance receiver"));
    }
    match member {
        "now" => Ok(Some(v8::Number::new(scope, state.performance_now()).into())),
        "clearResourceTimings" => {
            state.resource_timing.borrow_mut().entries.clear();
            Ok(None)
        }
        "setResourceTimingBufferSize" => {
            let size = args
                .get(0)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            state.resource_timing.borrow_mut().limit = (size as usize).min(4096);
            Ok(None)
        }
        "getEntries" | "getEntriesByType" | "getEntriesByName" => {
            let first = if member == "getEntries" {
                None
            } else {
                Some(
                    args.get(0)
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .to_rust_string_lossy(scope),
                )
            };
            let kind = if member == "getEntriesByName" && !args.get(1).is_undefined() {
                Some(
                    args.get(1)
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .to_rust_string_lossy(scope),
                )
            } else {
                None
            };
            let mut entries = {
                let timing = state.resource_timing.borrow();
                timing
                    .navigation
                    .iter()
                    .chain(timing.entries.iter())
                    .chain(timing.paint.iter())
                    .filter(|entry| match member {
                        "getEntriesByType" => first.as_deref() == Some(entry.kind),
                        "getEntriesByName" => {
                            first
                                .as_deref()
                                .is_some_and(|name| entry.name_matches(name))
                                && kind.as_deref().is_none_or(|k| k == entry.kind)
                        }
                        _ => true,
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            entries.sort_by(|a, b| a.start.total_cmp(&b.start));
            let mut values = Vec::with_capacity(entries.len());
            for entry in entries {
                values.push(entry_object(scope, state, entry)?.into())
            }
            Ok(Some(v8::Array::new_with_elements(scope, &values).into()))
        }
        _ => Err(V8BindingError::Allocation("Performance operation")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resource_worker_timing_requires_current_response_clock_and_permission() {
        let mut entry = Entry {
            navigation: None,
            render_blocking: false,
            kind: "resource",
            name: "https://worker.test/data".into(),
            initiator: "fetch".into(),
            start: 10.,
            end: 20.,
            bytes: 3,
            status: 200,
            details: Some(crate::platform::ResourceTimingMetadata {
                server_timing: Vec::new(),
                response_end_micros: Some(10_000),
                navigation_phases: None,
                producer_clock_micros: Some(1_000_000),
                decoded_body_size: Some(3),
                origin: "https://worker.test".into(),
                hops: Vec::new(),
                transport: Some(oryn_common::transport_timing::TransportTiming {
                    worker_start_clock_micros: Some(1_002_345),
                    ..Default::default()
                }),
                allow_details: true,
                response_status: 200,
                response_start_micros: 5_000,
                encoded_body_size: Some(3),
                content_type: String::new(),
            }),
        };
        assert_eq!(resource_worker_start(&entry), 12.3);
        entry.details.as_mut().unwrap().allow_details = false;
        assert_eq!(resource_worker_start(&entry), 0.);
        entry.details.as_mut().unwrap().allow_details = true;
        entry.details.as_mut().unwrap().producer_clock_micros = None;
        assert_eq!(
            resource_worker_start(&entry),
            0.,
            "cache/service reuse cannot replay a previous request's clock"
        );
        entry.details.as_mut().unwrap().producer_clock_micros = Some(1_003_000);
        assert_eq!(
            resource_worker_start(&entry),
            0.,
            "obsolete worker timestamp"
        );
        entry.details.as_mut().unwrap().producer_clock_micros = Some(1_000_000);
        entry
            .details
            .as_mut()
            .unwrap()
            .transport
            .as_mut()
            .unwrap()
            .worker_start_clock_micros = Some(1_006_000);
        assert_eq!(
            resource_worker_start(&entry),
            0.,
            "worker selection cannot follow response headers"
        );
    }
    #[test]
    fn resource_json_includes_inherited_worker_and_redirect_timings() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        let details = crate::platform::ResourceTimingMetadata {
            server_timing: Vec::new(),
            response_end_micros: Some(10_000),
            navigation_phases: Some(crate::platform::NavigationTimingPhases {
                fetch_start_micros: 4_000,
                redirect_start_micros: Some(500),
                redirect_end_micros: Some(3_500),
            }),
            producer_clock_micros: Some(1_000_000),
            decoded_body_size: Some(3),
            origin: "https://worker.test".into(),
            hops: Vec::new(),
            transport: Some(oryn_common::transport_timing::TransportTiming {
                worker_start_clock_micros: Some(1_002_000),
                ..Default::default()
            }),
            allow_details: true,
            response_status: 200,
            response_start_micros: 5_000,
            encoded_body_size: Some(3),
            content_type: String::new(),
        };
        external_with_status(
            &state,
            "https://worker.test/data".into(),
            "fetch".into(),
            10.,
            20.,
            Some(details.clone()),
            false,
        );
        external_with_status(
            &state,
            "https://worker.test/hidden".into(),
            "fetch".into(),
            10.,
            20.,
            Some(crate::platform::ResourceTimingMetadata {
                allow_details: false,
                ..details
            }),
            false,
        );
        let text=v8::String::new(scope,r#"(()=>{const [entry,hidden]=performance.getEntriesByType('resource');const json=entry.toJSON();if(entry.workerStart!==12||entry.fetchStart!==14||entry.redirectStart!==10.5||entry.redirectEnd!==13.5||json.workerStart!==12||json.redirectStart!==10.5||json.redirectEnd!==13.5)throw Error('resource clock fields');if(hidden.workerStart!==0||hidden.redirectStart!==0||hidden.redirectEnd!==0||hidden.toJSON().workerStart!==0)throw Error('timing privacy');return true})()"#).unwrap();
        assert!(
            v8::Script::compile(scope, text, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn navigation_entry_uses_original_clock_and_updates_retained_wrappers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        let start = Instant::now() - Duration::from_millis(25);
        let timing = crate::navigation_timing::NavigationTiming::new(
            start,
            crate::navigation_timing::Kind::Reload,
        );
        state
            .initialize_navigation_timing(
                timing.clone(),
                "https://navigation.test/first".into(),
                false,
            )
            .unwrap();
        let run = |scope: &mut v8::PinScope, source: &str| {
            let source = v8::String::new(scope, source).unwrap();
            assert!(
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        };
        run(
            scope,
            r#"globalThis.savedNavigation=performance.getEntriesByType('navigation')[0];globalThis.savedOrigin=performance.timeOrigin;(()=>{if(!(savedNavigation instanceof PerformanceNavigationTiming)||!(savedNavigation instanceof PerformanceResourceTiming)||savedNavigation.type!=='reload'||savedNavigation.startTime!==0||savedNavigation.loadEventEnd!==0||performance.now()<20)throw Error('pre-author native timing');return true})()"#,
        );
        assert!(
            buffered(&state).is_empty(),
            "navigation entry is readable before observer publication"
        );
        state.record_navigation_timing_stage(crate::navigation_timing::Stage::LoadEnd);
        assert_eq!(buffered(&state).len(), 1);
        run(
            scope,
            r#"(()=>{performance.clearResourceTimings();const current=performance.getEntriesByType('navigation');if(current.length!==1||savedNavigation.loadEventEnd<=0||savedNavigation.duration!==savedNavigation.loadEventEnd||savedOrigin!==performance.timeOrigin||current[0].name!==savedNavigation.name)throw Error('retained navigation entry');const json=savedNavigation.toJSON();if(json.type!=='reload'||json.loadEventEnd!==savedNavigation.loadEventEnd||json.name!=='https://navigation.test/first')throw Error('navigation JSON');return true})()"#,
        );
    }

    use super::*;
    #[test]
    fn server_timing_resource_access_is_masked_without_native_permission() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        for allowed in [false, true] {
            let details = crate::platform::ResourceTimingMetadata {
                server_timing: crate::server_timing::parse(&[(
                    "Server-Timing".into(),
                    "private;desc=secret".into(),
                )]),
                response_end_micros: None,
                navigation_phases: None,
                producer_clock_micros: None,
                decoded_body_size: None,
                origin: "https://foreign.test".into(),
                hops: Vec::new(),
                transport: None,
                allow_details: allowed,
                response_status: 200,
                response_start_micros: 0,
                encoded_body_size: None,
                content_type: String::new(),
            };
            external_with_status(
                &state,
                format!("https://foreign.test/{allowed}"),
                "fetch".into(),
                0.,
                1.,
                Some(details),
                false,
            );
        }
        let text=v8::String::new(scope,"(()=>{const hidden=performance.getEntriesByName('https://foreign.test/false')[0].serverTiming;const exposed=performance.getEntriesByName('https://foreign.test/true')[0].serverTiming;if(hidden.length!==0||!Object.isFrozen(hidden)||exposed.length!==1||exposed[0].description!=='secret'||!(exposed[0] instanceof PerformanceServerTiming))throw Error('server timing privacy');const json=performance.getEntriesByName('https://foreign.test/true')[0].toJSON();if(json.serverTiming.length!==1||json.serverTiming[0].toJSON().description!=='secret'||performance.getEntriesByName('https://foreign.test/false')[0].toJSON().serverTiming.length!==0)throw Error('server timing JSON snapshot privacy');return true})()").unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn native_resource_queries_snapshot_filter_order_and_clear() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        push(
            &state,
            Span {
                entry: Entry {
                    navigation: None,
                    render_blocking: false,
                    kind: "resource",
                    name: "https://foreign.test/opaque".into(),
                    initiator: "link".into(),
                    start: 0.,
                    end: 0.,
                    bytes: 1234,
                    status: 206,
                    details: None,
                },
                visible: false,
            },
        );
        push(
            &state,
            Span {
                entry: Entry {
                    navigation: None,
                    render_blocking: false,
                    kind: "resource",
                    name: "https://example.test/image".into(),
                    initiator: "img".into(),
                    start: 0.,
                    end: 0.,
                    bytes: 17,
                    status: 200,
                    details: None,
                },
                visible: true,
            },
        );
        let source=v8::String::new(scope,r#"(()=>{const entries=performance.getEntriesByType('resource');if(entries.length!==2||!(entries[0] instanceof PerformanceResourceTiming)||entries[0].entryType!=='resource'||entries[0].decodedBodySize!==0||entries[0].responseStatus!==0||entries[1].decodedBodySize!==17)throw Error('native timing');if(performance.getEntriesByName(entries[0].name,'mark').length||performance.getEntriesByName(entries[0].name,'resource').length!==1)throw Error('filter');const json=entries[0].toJSON();if(json.name!==entries[0].name||json.duration<0||performance.now()<entries[0].responseEnd)throw Error('clock');performance.clearResourceTimings();if(performance.getEntries().length||entries[0].name!==json.name)throw Error('immutable snapshot');return true})()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn opaque_terminal_outcomes_have_the_same_public_timing_shape() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        for status in [200, 206, 416] {
            push(
                &state,
                Span {
                    entry: Entry {
                        navigation: None,
                        render_blocking: false,
                        kind: "resource",
                        name: format!("https://foreign.test/{status}"),
                        initiator: "link".into(),
                        start: 0.,
                        end: 0.,
                        bytes: 91,
                        status,
                        details: None,
                    },
                    visible: false,
                },
            )
        }
        let entries = &state.resource_timing.borrow().entries;
        assert_eq!(entries.len(), 3);
        assert!(
            entries
                .iter()
                .all(|e| e.bytes == 0 && e.status == 0 && e.initiator == "link")
        );
    }
    #[test]
    fn foreign_platform_chunk_does_not_publish_original_stream() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        state.resource_timing.borrow_mut().streams.insert(
            7,
            Span {
                entry: Entry {
                    navigation: None,
                    render_blocking: false,
                    kind: "resource",
                    name: "https://example.test/data".into(),
                    initiator: "fetch".into(),
                    start: 0.,
                    end: 0.,
                    bytes: 0,
                    status: 200,
                    details: None,
                },
                visible: true,
            },
        );
        state
            .resource_timing
            .borrow_mut()
            .operations
            .insert(HostOperationId(1), Operation::Read(7));
        complete(
            &state,
            HostOperationId(1),
            &Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 8,
                bytes: vec![9, 9],
                done: true,
            }),
        );
        assert!(state.resource_timing.borrow().entries.is_empty());
        assert!(state.resource_timing.borrow().streams.contains_key(&7));
        state
            .resource_timing
            .borrow_mut()
            .operations
            .insert(HostOperationId(2), Operation::Read(7));
        complete(
            &state,
            HostOperationId(2),
            &Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: Some(1),
                stream_id: 7,
                bytes: vec![1],
                done: true,
            }),
        );
        let timing = state.resource_timing.borrow();
        assert_eq!(timing.entries.len(), 1);
        assert_eq!(timing.entries[0].bytes, 1);
        assert!(timing.streams.is_empty());
    }
    #[test]
    fn native_tao_provenance_can_expose_opaque_timing_without_exposing_response_headers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        for allowed in [false, true] {
            let id = HostOperationId(if allowed { 2 } else { 1 });
            state.resource_timing.borrow_mut().operations.insert(
                id,
                Operation::Start(Box::new(Span {
                    entry: Entry {
                        navigation: None,
                        render_blocking: false,
                        kind: "resource",
                        name: "https://foreign.test/image".into(),
                        initiator: "img".into(),
                        start: 0.,
                        end: 0.,
                        bytes: 0,
                        status: 0,
                        details: None,
                    },
                    visible: false,
                })),
            );
            complete(
                &state,
                id,
                &Ok(PlatformHostResult::HttpStarted {
                    timing: Some(Box::new(crate::platform::ResourceTimingMetadata {
                        server_timing: Vec::new(),
                        response_end_micros: None,
                        navigation_phases: None,
                        producer_clock_micros: None,
                        decoded_body_size: None,
                        origin: String::new(),
                        hops: Vec::new(),
                        transport: None,
                        allow_details: allowed,
                        response_status: 200,
                        response_start_micros: 1200,
                        encoded_body_size: Some(3),
                        content_type: "image/png".into(),
                    })),
                    internal_response: Some(crate::platform::InternalResponseMetadata {
                        status: 200,
                        headers: Vec::new(),
                        url: "https://foreign.test/image".into(),
                        redirected: false,
                        range_requested: false,
                    }),
                    response_type: Some(crate::platform::FetchResponseType::Opaque),
                    upload: None,
                    cors_filtered: true,
                    redirected: false,
                    stream_id: id.0,
                    status: 0,
                    url: String::new(),
                    headers: Vec::new(),
                }),
            );
            state
                .resource_timing
                .borrow_mut()
                .operations
                .insert(id, Operation::Read(id.0));
            complete(
                &state,
                id,
                &Ok(PlatformHostResult::HttpChunk {
                    encoded_body_size: None,
                    stream_id: id.0,
                    bytes: vec![1, 2, 3],
                    done: true,
                }),
            );
        }
        let timing = state.resource_timing.borrow();
        assert_eq!(timing.entries[0].bytes, 0);
        assert!(timing.entries[0].details.is_none());
        assert_eq!(timing.entries[1].bytes, 3);
        assert_eq!(timing.entries[1].status, 200);
        assert_eq!(
            timing.entries[1]
                .details
                .as_ref()
                .unwrap()
                .encoded_body_size,
            Some(3)
        );
    }
}

#[cfg(test)]
mod paint_tests {
    use super::*;
    #[test]
    fn native_paint_entries_are_immutable_once_only_and_survive_resource_clear() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>painted</body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        paint(&state, false);
        paint(&state, true);
        paint(&state, true);
        let source=v8::String::new(scope,r#"(()=>{let p=performance.getEntriesByType('paint');if(p.length!==2||!(p[0] instanceof PerformancePaintTiming)||!(p[0] instanceof PerformanceEntry)||p[0].name!=='first-paint'||p[1].name!=='first-contentful-paint'||p.some(x=>x.duration!==0))throw Error('paint records');let json=p[0].toJSON();if(Object.keys(json).sort().join()!=='duration,entryType,name,startTime')throw Error('paint JSON');performance.clearResourceTimings();if(performance.getEntriesByType('paint').length!==2||performance.getEntriesByName('first-paint','resource').length)throw Error('timeline filtering');return true})()"#).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn blocking_status_is_snapshot_data_even_without_timing_allow_origin() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(0),
            1,
        )
        .unwrap();
        external_with_status(
            &state,
            "https://foreign.test/s.js".into(),
            "script".into(),
            1.,
            2.,
            None,
            true,
        );
        external_with_status(
            &state,
            "https://foreign.test/import.js".into(),
            "script".into(),
            2.,
            3.,
            None,
            false,
        );
        let source=v8::String::new(scope,"const entries=performance.getEntriesByType('resource');entries[0].renderBlockingStatus==='blocking'&&entries[0].toJSON().renderBlockingStatus==='blocking'&&entries[0].responseStatus===0&&entries[1].renderBlockingStatus==='non-blocking'").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }

    #[test]
    fn dynamic_script_request_status_is_captured_before_attribute_mutation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<head></head><body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            RealmId(0),
            1,
        )
        .unwrap();
        state.commit_location("https://example.test/").unwrap();
        let source=v8::String::new(scope,"let s=document.createElement('script');s.src='/blocked.js';s.blocking='render';document.head.append(s);s.blocking='';let bodyScript=document.createElement('script');bodyScript.src='/body.js';bodyScript.blocking='render';document.body.append(bodyScript)").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let queued = state.take_pending_dynamic_scripts();
        assert_eq!(queued.len(), 2);
        assert!(queued[0].render_blocking);
        assert!(!queued[1].render_blocking);
        assert!(
            queued[0].render_blocker.is_none(),
            "body creation closed actual rendering admission"
        );
    }
}

#[cfg(test)]
mod navigation_entry_custody_tests {
    use super::*;
    #[test]
    fn navigation_entry_clones_share_canonical_name_and_match_filters() {
        let record = crate::navigation_timing::NavigationTiming::new(
            Instant::now(),
            crate::navigation_timing::Kind::Navigate,
        );
        let url = format!("https://example.test/{}", "x".repeat(65536));
        record.set_name(url.clone()).unwrap();
        let entry = Entry {
            navigation: Some(record),
            render_blocking: false,
            kind: "navigation",
            name: String::new(),
            initiator: "navigation".into(),
            start: 0.,
            end: 0.,
            bytes: 0,
            status: 0,
            details: None,
        };
        let retained = entry.clone();
        assert_eq!(retained.name.capacity(), 0);
        assert!(retained.name_matches(&url));
        assert!(!retained.name_matches(""));
        assert_eq!(retained.navigation.unwrap().snapshot().name, url);
    }
}
