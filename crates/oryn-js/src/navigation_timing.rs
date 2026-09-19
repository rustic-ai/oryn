//! Native navigation timing custody. Clones share measured phases, not a new
//! clock; cached documents and prerender activation retain the same record.
use crate::platform::ResourceTimingMetadata;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Navigate,
    Reload,
    BackForward,
}
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Navigate => "navigate",
            Self::Reload => "reload",
            Self::BackForward => "back_forward",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    DomInteractive,
    DomContentLoadedStart,
    DomContentLoadedEnd,
    DomComplete,
    LoadStart,
    LoadEnd,
    Activation,
}
#[derive(Clone, Debug)]
pub struct NavigationTiming {
    inner: Arc<Mutex<Record>>,
}
#[derive(Clone, Debug)]
struct Redirect {
    url: String,
    allow: Vec<String>,
    start: Option<Instant>,
    end: Option<Instant>,
}
#[derive(Clone, Debug)]
struct Record {
    budget: Option<Arc<dyn MetadataBudget>>,
    lease: Option<Arc<dyn MetadataLease>>,
    start: Instant,
    wall: f64,
    kind: Kind,
    url: String,
    fetch: Option<Instant>,
    received: Option<Instant>,
    ended: Option<Instant>,
    bytes: u64,
    encoded_bytes: Option<u64>,
    status: u16,
    content_type: String,
    identity_encoding: bool,
    server_timing: Vec<crate::server_timing::Metric>,
    details: Option<ResourceTimingMetadata>,
    redirects: Vec<Redirect>,
    no_referrer: bool,
    opaque: bool,
    stages: BTreeMap<Stage, Instant>,
    worker_start: Option<Instant>,
    unload: Option<(String, Instant, Instant)>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub server_timing: Vec<crate::server_timing::Metric>,
    pub name: String,
    pub kind: &'static str,
    pub protocol: String,
    pub delivery_type: &'static str,
    pub content_type: String,
    pub values: BTreeMap<&'static str, f64>,
}
fn coarse(value: f64) -> f64 {
    (value.max(0.) * 10.).floor() / 10.
}
fn origin(url: &str) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin();
    match origin {
        url::Origin::Tuple(..) => Some(origin.ascii_serialization()),
        _ => None,
    }
}
/// The native document owner supplies its existing shared admission budget.
pub trait MetadataLease: std::fmt::Debug + Send + Sync {}
pub trait MetadataBudget: std::fmt::Debug + Send + Sync {
    fn identity(&self) -> usize;
    fn reserve(&self, bytes: usize) -> Option<Arc<dyn MetadataLease>>;
}
impl Record {
    fn retained_bytes(&self) -> Result<usize, String> {
        let mut n = std::mem::size_of::<Self>() + 4096; // Arc/Mutex and all seven stage nodes.
        fn add(n: &mut usize, v: usize) -> Result<(), String> {
            *n = n
                .checked_add(v)
                .ok_or("navigation timing metadata overflow")?;
            Ok(())
        }
        fn strings(n: &mut usize, v: &Vec<String>) -> Result<(), String> {
            add(
                n,
                v.capacity()
                    .checked_mul(std::mem::size_of::<String>())
                    .ok_or("navigation timing metadata overflow")?,
            )?;
            for x in v {
                add(n, x.capacity())?;
            }
            Ok(())
        }
        fn metrics(n: &mut usize, v: &Vec<crate::server_timing::Metric>) -> Result<(), String> {
            add(
                n,
                v.capacity()
                    .checked_mul(std::mem::size_of::<crate::server_timing::Metric>())
                    .ok_or("navigation timing metadata overflow")?,
            )?;
            for m in v {
                add(n, m.name.capacity())?;
                add(n, m.description.capacity())?;
            }
            Ok(())
        }
        for v in [&self.url, &self.content_type] {
            add(&mut n, v.capacity())?;
        }
        metrics(&mut n, &self.server_timing)?;
        add(
            &mut n,
            self.redirects
                .capacity()
                .checked_mul(std::mem::size_of::<Redirect>())
                .ok_or("navigation timing metadata overflow")?,
        )?;
        for r in &self.redirects {
            add(&mut n, r.url.capacity())?;
            strings(&mut n, &r.allow)?;
        }
        if let Some((origin, _, _)) = &self.unload {
            add(&mut n, origin.capacity())?;
        }
        if let Some(d) = &self.details {
            add(&mut n, d.origin.capacity())?;
            add(&mut n, d.content_type.capacity())?;
            metrics(&mut n, &d.server_timing)?;
            add(
                &mut n,
                d.hops
                    .capacity()
                    .checked_mul(std::mem::size_of::<crate::platform::TimingHop>())
                    .ok_or("navigation timing metadata overflow")?,
            )?;
            for h in &d.hops {
                add(&mut n, h.url.capacity())?;
                strings(&mut n, &h.allow_origin)?;
            }
            if let Some(t) = &d.transport {
                add(&mut n, t.next_hop_protocol.capacity())?;
                if let Some(e) = &t.exposure {
                    add(&mut n, e.origin.capacity())?;
                    add(
                        &mut n,
                        e.hops
                            .capacity()
                            .checked_mul(std::mem::size_of::<
                                oryn_common::transport_timing::TimingHop,
                            >())
                            .ok_or("navigation timing metadata overflow")?,
                    )?;
                    for h in &e.hops {
                        add(&mut n, h.url.capacity())?;
                        strings(&mut n, &h.allow_origin)?;
                    }
                }
            }
        }
        Ok(n)
    }
}
fn server_timing_scratch(headers: &[(String, String)]) -> Result<usize, String> {
    // split() may retain comma and semicolon Vec<&str> simultaneously. Each
    // geometric Vec capacity is below twice its maximum delimiter count.
    let (mut remaining, mut largest) = (64 * 1024usize, 0usize);
    for (_, value) in headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("server-timing"))
    {
        if value.len() > remaining {
            break;
        }
        remaining -= value.len();
        largest = largest.max(value.len());
    }
    largest
        .checked_add(2)
        .and_then(|n| n.checked_mul(4 * std::mem::size_of::<&str>()))
        .ok_or_else(|| "navigation timing scratch overflow".into())
}
fn incoming_headers(
    url: &String,
    headers: &[(String, String)],
    details: Option<&ResourceTimingMetadata>,
) -> Result<usize, String> {
    let mut n = url
        .capacity()
        .checked_add(
            256 * std::mem::size_of::<crate::server_timing::Metric>()
                + 64 * std::mem::size_of::<String>(),
        )
        .ok_or("navigation timing metadata overflow")?;
    let mut add = |v: usize| -> Result<(), String> {
        n = n
            .checked_add(
                v.checked_mul(4)
                    .ok_or("navigation timing metadata overflow")?,
            )
            .ok_or("navigation timing metadata overflow")?;
        Ok(())
    };
    for (k, v) in headers {
        add(k.capacity())?;
        add(v.capacity())?;
    }
    if let Some(d) = details {
        add(d.origin.capacity())?;
        add(d.content_type.capacity())?;
        add(d
            .server_timing
            .capacity()
            .checked_mul(std::mem::size_of::<crate::server_timing::Metric>())
            .ok_or("navigation timing metadata overflow")?)?;
        for m in &d.server_timing {
            add(m.name.capacity())?;
            add(m.description.capacity())?;
        }
        add(d
            .hops
            .capacity()
            .checked_mul(std::mem::size_of::<crate::platform::TimingHop>())
            .ok_or("navigation timing metadata overflow")?)?;
        for h in &d.hops {
            add(h.url.capacity())?;
            add(h
                .allow_origin
                .capacity()
                .checked_mul(std::mem::size_of::<String>())
                .ok_or("navigation timing metadata overflow")?)?;
            for a in &h.allow_origin {
                add(a.capacity())?;
            }
        }
        if let Some(t) = &d.transport {
            add(t.next_hop_protocol.capacity())?;
            if let Some(e) = &t.exposure {
                add(e.origin.capacity())?;
                add(e
                    .hops
                    .capacity()
                    .checked_mul(std::mem::size_of::<oryn_common::transport_timing::TimingHop>())
                    .ok_or("navigation timing metadata overflow")?)?;
                for h in &e.hops {
                    add(h.url.capacity())?;
                    add(h
                        .allow_origin
                        .capacity()
                        .checked_mul(std::mem::size_of::<String>())
                        .ok_or("navigation timing metadata overflow")?)?;
                    for a in &h.allow_origin {
                        add(a.capacity())?;
                    }
                }
            }
        }
    }
    Ok(n)
}
impl NavigationTiming {
    pub fn admit_metadata(&self, budget: Arc<dyn MetadataBudget>) -> Result<(), String> {
        let mut r = self.inner.lock().expect("native navigation timing");
        if r.lease.is_some() {
            return if r
                .budget
                .as_ref()
                .is_some_and(|owner| owner.identity() == budget.identity())
            {
                Ok(())
            } else {
                Err("navigation timing metadata owner mismatch".into())
            };
        }
        let lease = budget
            .reserve(r.retained_bytes()?)
            .ok_or("navigation timing metadata capacity")?;
        r.budget = Some(budget);
        r.lease = Some(lease);
        Ok(())
    }
    fn update_metadata(
        &self,
        incoming: usize,
        scratch: usize,
        update: impl FnOnce(&mut Record),
    ) -> Result<(), String> {
        let mut current = self.inner.lock().expect("native navigation timing");
        // Old credit remains live. Reserve a conservative full candidate before
        // cloning or parsing; doubling covers container growth when appending.
        let _scratch = if let Some(budget) = &current.budget {
            if scratch == 0 {
                None
            } else {
                Some(
                    budget
                        .reserve(scratch)
                        .ok_or("navigation timing scratch capacity")?,
                )
            }
        } else {
            None
        };
        let lease = if let Some(budget) = &current.budget {
            let bytes = current
                .retained_bytes()?
                .checked_mul(2)
                .and_then(|n| n.checked_add(incoming))
                .ok_or("navigation timing metadata overflow")?;
            Some(
                budget
                    .reserve(bytes)
                    .ok_or("navigation timing metadata capacity")?,
            )
        } else {
            None
        };
        let mut next = current.clone();
        update(&mut next);
        next.lease = lease;
        *current = next;
        Ok(())
    }

    pub fn new(start: Instant, kind: Kind) -> Self {
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
            * 1000.
            - elapsed;
        Self {
            inner: Arc::new(Mutex::new(Record {
                budget: None,
                lease: None,
                start,
                wall,
                kind,
                url: String::new(),
                fetch: None,
                received: None,
                ended: None,
                bytes: 0,
                encoded_bytes: None,
                status: 0,
                content_type: String::new(),
                identity_encoding: true,
                server_timing: Vec::new(),
                details: None,
                redirects: Vec::new(),
                no_referrer: false,
                opaque: false,
                stages: BTreeMap::new(),
                worker_start: None,
                unload: None,
            })),
        }
    }
    pub fn completed(&self) -> bool {
        self.inner
            .lock()
            .expect("native navigation timing")
            .stages
            .contains_key(&Stage::LoadEnd)
    }
    pub fn set_kind(&self, kind: Kind) {
        self.inner.lock().expect("native navigation timing").kind = kind;
    }
    pub fn redirect_provenance(&self, hops: &[crate::platform::TimingHop]) -> Result<(), String> {
        if !self
            .inner
            .lock()
            .expect("native navigation timing")
            .redirects
            .is_empty()
        {
            return Ok(());
        }
        self.update_metadata(
            hops.iter().try_fold(0usize, |n, h| {
                let strings = h.allow_origin.iter().try_fold(h.url.capacity(), |n, v| {
                    n.checked_add(v.capacity())
                        .ok_or("navigation timing metadata overflow")
                })?;
                n.checked_add(
                    strings
                        .checked_mul(4)
                        .and_then(|n| n.checked_add(64 * std::mem::size_of::<String>()))
                        .ok_or("navigation timing metadata overflow")?,
                )
                .ok_or("navigation timing metadata overflow")
            })?,
            0,
            |record| {
                for hop in hops.iter().take(hops.len().saturating_sub(1)).take(20) {
                    record.redirects.push(Redirect {
                        url: hop.url.clone(),
                        allow: hop
                            .allow_origin
                            .iter()
                            .flat_map(|value| value.split(',').map(|part| part.trim().to_owned()))
                            .take(64)
                            .collect(),
                        start: None,
                        end: None,
                    });
                }
            },
        )
    }
    pub fn origin(&self) -> (Instant, f64) {
        let record = self.inner.lock().expect("native navigation timing");
        (record.start, record.wall)
    }
    pub fn mark(&self, stage: Stage, time: Instant) -> bool {
        let mut record = self.inner.lock().expect("native navigation timing");
        if record.stages.contains_key(&stage) {
            return false;
        }
        record.stages.insert(stage, time);
        true
    }
    pub fn redirect(
        &self,
        url: String,
        headers: &[(String, String)],
        start: Instant,
        end: Instant,
    ) -> Result<(), String> {
        self.update_metadata(incoming_headers(&url, headers, None)?, 0, |record| {
            if record.redirects.len() < 20 {
                record.redirects.push(Redirect {
                    url,
                    allow: headers
                        .iter()
                        .filter(|(name, _)| name.eq_ignore_ascii_case("timing-allow-origin"))
                        .flat_map(|(_, value)| {
                            value.split(',').map(|value| value.trim().to_owned())
                        })
                        .take(64)
                        .collect(),
                    start: Some(start),
                    end: Some(end),
                });
            }
        })
    }
    pub fn response(
        &self,
        url: String,
        status: u16,
        headers: &[(String, String)],
        fetch: Instant,
        received: Instant,
        details: Option<ResourceTimingMetadata>,
        no_referrer: bool,
    ) -> Result<(), String> {
        self.update_metadata(
            incoming_headers(&url, headers, details.as_ref())?,
            server_timing_scratch(headers)?,
            |record| {
                record.url = url;
                record.status = status;
                record.server_timing = crate::server_timing::parse(headers);
                record.identity_encoding = headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
                    .all(|(_, value)| value.trim().eq_ignore_ascii_case("identity"));
                record.content_type = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                    .map(|(_, value)| {
                        value
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_ascii_lowercase()
                    })
                    .unwrap_or_default();
                record.fetch = Some(fetch);
                record.received = Some(received);
                record.details = details.filter(|details| details.valid_native_snapshot());
                record.no_referrer = no_referrer;
            },
        )
    }
    /// A native decoded/snapshot source has availability and EOF, but carries no
    /// measured HTTP status, wire sizes, cache delivery or network phases.
    pub fn source_available(&self, time: Instant) {
        let mut record = self.inner.lock().expect("native navigation timing");
        if record.received.is_none() {
            record.fetch = Some(record.start);
            record.received = Some(time);
        }
    }
    pub fn finish(&self, time: Instant, bytes: u64) {
        self.finish_encoded(time, bytes, None);
    }
    pub fn finish_encoded(&self, time: Instant, bytes: u64, encoded_bytes: Option<u64>) {
        let mut record = self.inner.lock().expect("native navigation timing");
        if record.ended.is_none() {
            record.ended = Some(time);
            record.bytes = bytes;
            record.encoded_bytes = encoded_bytes;
        }
    }
    pub fn name_matches(&self, name: &str) -> bool {
        self.inner.lock().expect("native navigation timing").url == name
    }
    pub fn set_name_from(&self, url: &str) -> Result<(), String> {
        if !self
            .inner
            .lock()
            .expect("native navigation timing")
            .url
            .is_empty()
        {
            return Ok(());
        }
        self.update_metadata(url.len(), 0, |record| record.url = url.to_owned())
    }
    pub fn set_name(&self, url: String) -> Result<(), String> {
        if !self
            .inner
            .lock()
            .expect("native navigation timing")
            .url
            .is_empty()
        {
            return Ok(());
        }
        self.update_metadata(url.capacity(), 0, |record| record.url = url)
    }
    pub fn set_opaque(&self, opaque: bool) {
        self.inner.lock().expect("native navigation timing").opaque = opaque;
    }
    pub fn worker_start(&self, time: Instant) {
        let mut record = self.inner.lock().expect("native navigation timing");
        record.worker_start.get_or_insert(time);
    }
    pub fn unload(
        &self,
        source_origin: String,
        start: Instant,
        end: Instant,
    ) -> Result<(), String> {
        if self
            .inner
            .lock()
            .expect("native navigation timing")
            .unload
            .is_some()
        {
            return Ok(());
        }
        self.update_metadata(source_origin.capacity(), 0, |record| {
            record.unload = Some((source_origin, start, end))
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        let record = self.inner.lock().expect("native navigation timing");
        let ms = |time: Instant| {
            coarse(
                time.checked_duration_since(record.start)
                    .unwrap_or_default()
                    .as_secs_f64()
                    * 1000.,
            )
        };
        let stage = |stage| record.stages.get(&stage).copied().map(ms).unwrap_or(0.);
        let final_origin = (!record.opaque).then(|| origin(&record.url)).flatten();
        let redirects_visible = !record.no_referrer
            && final_origin.as_ref().is_some_and(|destination| {
                record.redirects.iter().all(|hop| {
                    origin(&hop.url).as_ref() == Some(destination)
                        || hop
                            .allow
                            .iter()
                            .any(|value| value == "*" || value == destination)
                })
            });
        let phase_origin = record.fetch.map(ms).unwrap_or(0.);
        let phases = record
            .details
            .as_ref()
            .and_then(|details| details.navigation_phases.as_ref());
        let fetch = coarse(
            phase_origin + phases.map_or(0, |phases| phases.fetch_start_micros) as f64 / 1000.,
        );
        let mut values = BTreeMap::from([
            ("startTime", 0.),
            ("duration", stage(Stage::LoadEnd)),
            ("fetchStart", fetch),
            (
                "responseStart",
                record
                    .details
                    .as_ref()
                    .map(|details| {
                        coarse(phase_origin + details.response_start_micros as f64 / 1000.)
                    })
                    .unwrap_or_else(|| record.received.map(ms).unwrap_or(0.)),
            ),
            ("responseEnd", record.ended.map(ms).unwrap_or(0.)),
            ("decodedBodySize", record.bytes as f64),
            (
                "encodedBodySize",
                (if record.identity_encoding && record.ended.is_some() {
                    record.bytes
                } else {
                    record
                        .encoded_bytes
                        .or_else(|| {
                            record
                                .details
                                .as_ref()
                                .and_then(|details| details.encoded_body_size)
                        })
                        .unwrap_or(0)
                }) as f64,
            ),
            ("transferSize", 0.),
            ("responseStatus", f64::from(record.status)),
            (
                "redirectCount",
                if redirects_visible {
                    record.redirects.len() as f64
                } else {
                    0.
                },
            ),
            (
                "redirectStart",
                if redirects_visible {
                    record
                        .redirects
                        .first()
                        .and_then(|hop| hop.start)
                        .map(ms)
                        .or_else(|| {
                            phases
                                .and_then(|phases| phases.redirect_start_micros)
                                .map(|value| coarse(phase_origin + value as f64 / 1000.))
                        })
                        .unwrap_or(0.)
                } else {
                    0.
                },
            ),
            (
                "redirectEnd",
                if redirects_visible {
                    record
                        .redirects
                        .last()
                        .and_then(|hop| hop.end)
                        .map(ms)
                        .or_else(|| {
                            phases
                                .and_then(|phases| phases.redirect_end_micros)
                                .map(|value| coarse(phase_origin + value as f64 / 1000.))
                        })
                        .unwrap_or(0.)
                } else {
                    0.
                },
            ),
            ("domInteractive", stage(Stage::DomInteractive)),
            (
                "domContentLoadedEventStart",
                stage(Stage::DomContentLoadedStart),
            ),
            (
                "domContentLoadedEventEnd",
                stage(Stage::DomContentLoadedEnd),
            ),
            ("domComplete", stage(Stage::DomComplete)),
            ("loadEventStart", stage(Stage::LoadStart)),
            ("loadEventEnd", stage(Stage::LoadEnd)),
            ("activationStart", stage(Stage::Activation)),
            ("unloadEventStart", 0.),
            ("unloadEventEnd", 0.),
            ("workerStart", record.worker_start.map(ms).unwrap_or(0.)),
            ("criticalCHRestart", 0.),
        ]);
        if let Some((source, start, end)) = &record.unload {
            if final_origin.as_ref() == Some(source)
                && record
                    .redirects
                    .iter()
                    .all(|hop| origin(&hop.url).as_ref() == Some(source))
            {
                values.insert("unloadEventStart", ms(*start));
                values.insert("unloadEventEnd", ms(*end));
            }
        }
        let transport = record
            .details
            .as_ref()
            .filter(|details| details.allow_details)
            .and_then(|details| details.transport.as_ref());
        for (name, phase) in [
            ("domainLookupStart", transport.and_then(|t| t.dns_start)),
            ("domainLookupEnd", transport.and_then(|t| t.dns_end)),
            ("connectStart", transport.and_then(|t| t.connect_start)),
            (
                "connectEnd",
                transport.and_then(|t| t.secure_end.or(t.connect_end)),
            ),
            (
                "secureConnectionStart",
                transport.and_then(|t| t.secure_start),
            ),
            ("requestStart", transport.map(|t| t.request_start)),
        ] {
            values.insert(
                name,
                if name == "secureConnectionStart" && phase.is_none() {
                    0.
                } else {
                    phase
                        .map(|value| coarse(phase_origin + value as f64 / 1000.))
                        .unwrap_or(fetch)
                },
            );
        }
        let delivery = transport
            .map(|transport| transport.delivery)
            .unwrap_or_default();
        values.insert(
            "transferSize",
            delivery.transfer_size(values["encodedBodySize"] as u64) as f64,
        );
        Snapshot {
            server_timing: record.server_timing.clone(),
            name: record.url.clone(),
            kind: record.kind.as_str(),
            delivery_type: delivery.delivery_type(),
            protocol: transport
                .map(|transport| transport.next_hop_protocol.clone())
                .unwrap_or_default(),
            content_type: record.content_type.clone(),
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }
    #[test]
    fn current_worker_start_uses_document_clock_and_is_not_replaced_by_later_delivery() {
        let start = Instant::now();
        let record = NavigationTiming::new(start, Kind::Navigate);
        assert_eq!(record.snapshot().values["workerStart"], 0.);
        record.worker_start(at(start, 3));
        record.worker_start(at(start, 9));
        assert_eq!(record.snapshot().values["workerStart"], 3.);
    }
    #[test]
    fn retained_document_clock_and_lifecycle_are_shared_and_once_only() {
        let start = Instant::now();
        let record = NavigationTiming::new(start, Kind::Navigate);
        record.set_name("https://a.test/original".into()).unwrap();
        let cached = record.clone();
        record.finish(at(start, 10), 37);
        assert!(record.mark(Stage::DomInteractive, at(start, 12)));
        assert!(record.mark(Stage::LoadStart, at(start, 15)));
        assert_eq!(cached.snapshot().values["loadEventEnd"], 0.);
        assert!(cached.mark(Stage::LoadEnd, at(start, 18)));
        assert!(!record.mark(Stage::LoadEnd, at(start, 99)));
        assert!(cached.mark(Stage::Activation, at(start, 40)));
        assert_eq!(record.origin(), cached.origin());
        record.set_name("https://a.test/fragment".into()).unwrap();
        let result = record.snapshot();
        assert_eq!(result.name, "https://a.test/original");
        assert_eq!(result.values["duration"], 18.);
        assert_eq!(result.values["activationStart"], 40.);
        assert_eq!(result.values["responseEnd"], 10.);
        assert_eq!(result.values["encodedBodySize"], 37.);
    }
    #[test]
    fn navigation_sizes_distinguish_network_local_and_validated_cache() {
        use crate::platform::ResourceTimingMetadata;
        use oryn_common::transport_timing::{ResponseDelivery, TransportTiming};
        for (delivery, expected, name) in [
            (ResponseDelivery::Network, 1324., ""),
            (ResponseDelivery::LocalCache, 0., "cache"),
            (ResponseDelivery::ValidatedCache, 300., ""),
        ] {
            let start = Instant::now();
            let record = NavigationTiming::new(start, Kind::Navigate);
            let details = ResourceTimingMetadata {
                server_timing: Vec::new(),
                response_end_micros: Some(2000),
                navigation_phases: None,
                producer_clock_micros: None,
                decoded_body_size: Some(2048),
                origin: "https://timing.test".into(),
                hops: Vec::new(),
                transport: Some(TransportTiming {
                    delivery,
                    ..Default::default()
                }),
                allow_details: true,
                response_status: 200,
                response_start_micros: 1000,
                encoded_body_size: Some(1024),
                content_type: "text/html".into(),
            };
            record
                .response(
                    "https://timing.test/".into(),
                    200,
                    &[("content-encoding".into(), "gzip".into())],
                    start,
                    at(start, 1),
                    Some(details),
                    false,
                )
                .unwrap();
            record.finish(at(start, 2), 2048);
            let snapshot = record.snapshot();
            assert_eq!(snapshot.values["transferSize"], expected);
            assert_eq!(snapshot.delivery_type, name);
            assert_eq!(snapshot.values["encodedBodySize"], 1024.);
            assert_eq!(snapshot.values["decodedBodySize"], 2048.);
        }
    }
    #[test]
    fn compressed_terminal_size_replaces_header_estimate_once() {
        let start = Instant::now();
        let record = NavigationTiming::new(start, Kind::Navigate);
        record
            .response(
                "https://timing.test/".into(),
                200,
                &[("content-encoding".into(), "gzip".into())],
                start,
                at(start, 1),
                None,
                false,
            )
            .unwrap();
        assert_eq!(record.snapshot().values["encodedBodySize"], 0.);
        record.finish_encoded(at(start, 2), 1024, Some(37));
        record.finish_encoded(at(start, 3), 2048, Some(99));
        let snapshot = record.snapshot();
        assert_eq!(snapshot.values["encodedBodySize"], 37.);
        assert_eq!(snapshot.values["decodedBodySize"], 1024.);
        assert_eq!(snapshot.values["responseEnd"], 2.);
    }
    #[test]
    fn completed_identity_sizes_override_declared_length_for_every_delivery() {
        use oryn_common::transport_timing::{ResponseDelivery, TransportTiming};
        for (delivery, transfer) in [
            (ResponseDelivery::Unknown, 0.),
            (ResponseDelivery::Network, 337.),
            (ResponseDelivery::LocalCache, 0.),
            (ResponseDelivery::ValidatedCache, 300.),
        ] {
            let start = Instant::now();
            let record = NavigationTiming::new(start, Kind::Navigate);
            let details = ResourceTimingMetadata {
                server_timing: Vec::new(),
                response_end_micros: None,
                navigation_phases: None,
                producer_clock_micros: None,
                decoded_body_size: None,
                origin: "https://timing.test".into(),
                hops: Vec::new(),
                transport: Some(TransportTiming {
                    delivery,
                    ..Default::default()
                }),
                allow_details: true,
                response_status: 200,
                response_start_micros: 1000,
                encoded_body_size: Some(99999),
                content_type: "text/html".into(),
            };
            record
                .response(
                    "https://timing.test/".into(),
                    200,
                    &[("content-encoding".into(), "identity".into())],
                    start,
                    at(start, 1),
                    Some(details),
                    false,
                )
                .unwrap();
            record.finish(at(start, 2), 37);
            record.finish(at(start, 9), 888);
            let snapshot = record.snapshot();
            assert_eq!(snapshot.values["encodedBodySize"], 37.);
            assert_eq!(snapshot.values["decodedBodySize"], 37.);
            assert_eq!(snapshot.values["transferSize"], transfer);
            assert_eq!(snapshot.values["responseEnd"], 2.);
        }
    }
    #[test]
    fn redirects_require_each_hop_permission_for_destination_and_referrer_policy() {
        let start = Instant::now();
        let headers = vec![(
            "Timing-Allow-Origin".into(),
            "https://destination.test".into(),
        )];
        let record = NavigationTiming::new(start, Kind::Navigate);
        record
            .redirect(
                "https://source.test/".into(),
                &headers,
                at(start, 1),
                at(start, 2),
            )
            .unwrap();
        record
            .response(
                "https://destination.test/".into(),
                200,
                &[],
                at(start, 3),
                at(start, 4),
                None,
                false,
            )
            .unwrap();
        assert_eq!(record.snapshot().values["redirectCount"], 1.);
        assert_eq!(record.snapshot().values["redirectEnd"], 2.);
        record
            .response(
                "https://destination.test/".into(),
                200,
                &[],
                at(start, 3),
                at(start, 4),
                None,
                true,
            )
            .unwrap();
        assert_eq!(record.snapshot().values["redirectCount"], 0.);
        record
            .response(
                "https://different.test/".into(),
                200,
                &[],
                at(start, 3),
                at(start, 4),
                None,
                false,
            )
            .unwrap();
        assert_eq!(record.snapshot().values["redirectStart"], 0.);
        record.set_opaque(true);
        assert_eq!(record.snapshot().values["redirectCount"], 0.);
    }
    #[test]
    fn unavailable_redirect_phases_are_not_fabricated_from_final_response() {
        let start = Instant::now();
        let record = NavigationTiming::new(start, Kind::BackForward);
        record.set_name("https://a.test/end".into()).unwrap();
        record
            .redirect_provenance(&[
                crate::platform::TimingHop {
                    url: "https://a.test/start".into(),
                    allow_origin: vec![],
                },
                crate::platform::TimingHop {
                    url: "https://a.test/end".into(),
                    allow_origin: vec![],
                },
            ])
            .unwrap();
        let snapshot = record.snapshot();
        assert_eq!(snapshot.kind, "back_forward");
        assert_eq!(snapshot.values["redirectCount"], 1.);
        assert_eq!(snapshot.values["redirectStart"], 0.);
        assert_eq!(snapshot.values["redirectEnd"], 0.);
    }
    #[test]
    fn unload_timing_cannot_cross_an_opaque_or_redirected_origin_boundary() {
        let start = Instant::now();
        let record = NavigationTiming::new(start, Kind::Navigate);
        record.set_name("https://a.test/next".into()).unwrap();
        record
            .unload("https://a.test".into(), at(start, 1), at(start, 2))
            .unwrap();
        assert_eq!(record.snapshot().values["unloadEventEnd"], 2.);
        record
            .redirect(
                "https://other.test/".into(),
                &[("Timing-Allow-Origin".into(), "*".into())],
                at(start, 3),
                at(start, 4),
            )
            .unwrap();
        assert_eq!(record.snapshot().values["unloadEventEnd"], 0.);
    }
}

/// A retained unload dispatch measures only its accepted callback interval.
/// Canceled preparation never invents a completed interval.
#[derive(Clone, Debug, Default)]
pub struct UnloadTiming(Arc<Mutex<(Option<Instant>, Option<Instant>)>>);
impl UnloadTiming {
    pub fn start(&self) {
        self.0
            .lock()
            .expect("unload timing")
            .0
            .get_or_insert_with(Instant::now);
    }
    pub fn finish(&self) {
        let mut times = self.0.lock().expect("unload timing");
        if times.0.is_some() {
            times.1.get_or_insert_with(Instant::now);
        }
    }
    pub fn interval(&self) -> Option<(Instant, Instant)> {
        let times = self.0.lock().expect("unload timing");
        Some((times.0?, times.1?))
    }
}
