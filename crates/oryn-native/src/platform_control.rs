//! BrowserContext-owned native platform state, shared across replacement documents.
use crate::agent_registry::AgentOwner;
use oryn_common::v3::PageId;
use oryn_js::platform_control::{PermissionPrompt, PromptChoice, PromptKind};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
struct PromptResolution {
    owner: AgentOwner,
    receipt: u64,
    id: String,
    choice: PromptChoice,
    response: Response,
}
struct PendingPrompt {
    owner: AgentOwner,
    top_origin: String,
    snapshot: PermissionPrompt,
    protocol: Option<(String, String)>,
}
use oryn_js::platform_control::{
    Coordinates, PermissionDescriptor, PermissionState, PositionError, RegistrationMode, Request,
    Response,
};

#[derive(Default)]
pub(crate) struct State {
    prompts: BTreeMap<String, PendingPrompt>,
    prompt_resolutions: VecDeque<PromptResolution>,
    bluetooth_devices: BTreeMap<String, BTreeSet<String>>,
    bluetooth_pending: BTreeMap<AgentOwner, BTreeSet<u64>>,
    permissions: BTreeMap<(String, String, String), PermissionState>,
    positions: BTreeMap<PageId, Result<Coordinates, PositionError>>,
    hidden: BTreeSet<PageId>,
    window_positions: BTreeMap<PageId, (i32, i32)>,
    modes: BTreeMap<PageId, RegistrationMode>,
    handlers: BTreeMap<String, (String, String)>,
    revision: u64,
}
impl State {
    pub(crate) fn bluetooth_snapshot(
        &self,
        owner: &AgentOwner,
        origin: &str,
        enabled: bool,
        allowed: bool,
        reports: Vec<String>,
    ) -> Response {
        Response::Bluetooth {
            allowed,
            reports,
            devices: self
                .bluetooth_devices
                .get(origin)
                .map(|devices| devices.iter().cloned().collect())
                .unwrap_or_default(),
            testing_enabled: enabled,
            pending_responses: self.bluetooth_pending.get(owner).map_or(0, BTreeSet::len),
        }
    }

    pub(crate) fn retire_page(&mut self, page: PageId) {
        self.bluetooth_pending
            .retain(|owner, _| !matches!(owner,AgentOwner::Window{page:owned,..}if *owned==page));
        self.prompts.retain(
            |_, prompt| !matches!(&prompt.owner,AgentOwner::Window{page:owned,..}if *owned==page),
        );
        self.prompt_resolutions.retain(
            |entry| !matches!(&entry.owner,AgentOwner::Window{page:owned,..}if *owned==page),
        );
        self.positions.remove(&page);
        self.hidden.remove(&page);
        self.window_positions.remove(&page);
        self.modes.remove(&page);
        // Grants and accepted protocol registrations belong to the context,
        // not the lifetime of the window that configured them.
    }
    pub(crate) fn retire_owner(&mut self, owner: &AgentOwner) {
        self.bluetooth_pending.remove(owner);
        self.prompt_resolutions
            .retain(|entry| &entry.owner != owner);
        self.prompts.retain(|_, prompt| &prompt.owner != owner);
    }
    pub(crate) fn prompt_kind(&self, owner: &AgentOwner, id: &str) -> Result<PromptKind, String> {
        self.prompts
            .get(id)
            .filter(|prompt| &prompt.owner == owner)
            .map(|prompt| prompt.snapshot.kind)
            .ok_or_else(|| "InvalidStateError: unknown or foreign permission prompt".into())
    }
    #[cfg(test)]
    pub(crate) fn begin_prompt(
        &mut self,
        owner: &AgentOwner,
        origin: &str,
        kind: PromptKind,
    ) -> Result<PermissionPrompt, String> {
        self.begin_prompt_partitioned(owner, origin, origin, kind)
    }
    pub(crate) fn begin_prompt_partitioned(
        &mut self,
        owner: &AgentOwner,
        top_origin: &str,
        origin: &str,
        kind: PromptKind,
    ) -> Result<PermissionPrompt, String> {
        if self.prompts.len() >= 1024 {
            return Err("QuotaExceededError: permission prompt limit".into());
        }
        if self.prompts.values().any(|prompt| &prompt.owner == owner) {
            return Err("InvalidStateError: permission prompt already open".into());
        }
        let mut state = PermissionState::Granted;
        for name in kind.names() {
            let descriptor = PermissionDescriptor {
                name: (*name).into(),
                sysex: None,
                pan_tilt_zoom: None,
                allow_without_gesture: None,
            };
            let current = self.permission_value_partitioned(top_origin, origin, &descriptor)?;
            if current == PermissionState::Denied {
                state = current;
                break;
            }
            if current == PermissionState::Prompt {
                state = current
            }
        }
        let mut bytes = [0u8; 24];
        getrandom::fill(&mut bytes)
            .map_err(|_| "InvalidStateError: permission prompt identity unavailable")?;
        let id = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let snapshot = PermissionPrompt {
            id: id.clone(),
            origin: origin.into(),
            kind,
            state,
        };
        if state == PermissionState::Prompt {
            self.prompts.insert(
                id,
                PendingPrompt {
                    owner: owner.clone(),
                    top_origin: top_origin.into(),
                    snapshot: snapshot.clone(),
                    protocol: None,
                },
            );
        }
        Ok(snapshot)
    }
    pub(crate) fn register_protocol(
        &mut self,
        owner: &AgentOwner,
        page: PageId,
        origin: &str,
        scheme: String,
        url: String,
    ) -> Result<Response, String> {
        let scheme = oryn_js::platform_control::protocol_scheme(&scheme)?;
        let response = self.query(
            page,
            origin,
            Request::RegisterProtocol {
                scheme: scheme.clone(),
                url: url.clone(),
            },
        )?;
        if self.modes.get(&page).copied().unwrap_or_default() != RegistrationMode::None {
            return Ok(response);
        }
        if self.handlers.get(&scheme) == Some(&(origin.to_owned(), url.clone())) {
            return Ok(Response::ProtocolRegistration { accepted: true });
        }
        if self.prompts.len() >= 1024 || self.prompts.values().any(|prompt| &prompt.owner == owner)
        {
            return Ok(Response::ProtocolRegistration { accepted: false });
        }
        let mut bytes = [0u8; 24];
        getrandom::fill(&mut bytes)
            .map_err(|_| "InvalidStateError: protocol prompt identity unavailable")?;
        let id = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let snapshot = PermissionPrompt {
            id: id.clone(),
            origin: origin.into(),
            kind: PromptKind::Protocol,
            state: PermissionState::Prompt,
        };
        self.prompts.insert(
            id,
            PendingPrompt {
                owner: owner.clone(),
                top_origin: origin.into(),
                snapshot: snapshot.clone(),
                protocol: Some((scheme, url)),
            },
        );
        Ok(Response::Prompt(snapshot))
    }
    pub(crate) fn answer_prompt(
        &mut self,
        owner: &AgentOwner,
        id: &str,
        choice: PromptChoice,
    ) -> Result<PermissionPrompt, String> {
        self.prompt_kind(owner, id)?;
        let mut snapshot = self
            .prompts
            .get(id)
            .ok_or("InvalidStateError: permission prompt disappeared")?
            .snapshot
            .clone();
        if snapshot.kind == PromptKind::Protocol {
            let (scheme, url) = self
                .prompts
                .get(id)
                .and_then(|prompt| prompt.protocol.clone())
                .ok_or("InvalidStateError: missing protocol prompt authority")?;
            if choice == PromptChoice::Allow {
                if !self.handlers.contains_key(&scheme) && self.handlers.len() >= 1024 {
                    return Err("QuotaExceededError: protocol handler limit".into());
                }
                self.bump()?;
                self.handlers.insert(scheme, (snapshot.origin.clone(), url));
                snapshot.state = PermissionState::Granted;
            } else if choice == PromptChoice::Deny {
                snapshot.state = PermissionState::Denied;
            }
            self.prompts.remove(id);
            return Ok(snapshot);
        }
        if choice != PromptChoice::Dismiss {
            let state = if choice == PromptChoice::Allow {
                PermissionState::Granted
            } else {
                PermissionState::Denied
            };
            // Reserve all grant entries before any mutation, so a full registry
            // cannot partially accept an audio/video prompt.
            let top_origin = self
                .prompts
                .get(id)
                .ok_or("InvalidStateError: permission prompt disappeared")?
                .top_origin
                .clone();
            let mut keys = Vec::new();
            for name in snapshot.kind.names() {
                let descriptor = PermissionDescriptor {
                    name: (*name).into(),
                    sysex: None,
                    pan_tilt_zoom: None,
                    allow_without_gesture: None,
                };
                keys.push((
                    top_origin.clone(),
                    snapshot.origin.clone(),
                    descriptor.key()?,
                ));
            }
            if self.permissions.len()
                + keys
                    .iter()
                    .filter(|key| !self.permissions.contains_key(*key))
                    .count()
                > 4096
            {
                return Err("QuotaExceededError: permission entry limit".into());
            }
            self.bump()?;
            for key in keys {
                self.permissions.insert(key, state);
            }
            snapshot.state = state;
        }
        self.prompts.remove(id);
        Ok(snapshot)
    }
    /// Replay acknowledges the original mutation; it never reinstalls a grant.
    pub(crate) fn prompt_resolution(
        &self,
        owner: &AgentOwner,
        id: &str,
        choice: PromptChoice,
        receipt: u64,
    ) -> Result<Option<Response>, String> {
        if receipt == 0 {
            return Err("InvalidStateError: invalid permission resolution receipt".into());
        }
        if let Some(entry) = self
            .prompt_resolutions
            .iter()
            .find(|entry| &entry.owner == owner && entry.receipt == receipt)
        {
            if entry.id != id || entry.choice != choice {
                return Err("InvalidStateError: conflicting permission resolution replay".into());
            }
            return Ok(Some(entry.response.clone()));
        }
        // A pruned/foreign receipt cannot resurrect a consumed prompt.
        self.prompt_kind(owner, id)?;
        Ok(None)
    }
    pub(crate) fn resolve_prompt(
        &mut self,
        owner: &AgentOwner,
        id: &str,
        choice: PromptChoice,
        receipt: u64,
    ) -> Result<Response, String> {
        if let Some(response) = self.prompt_resolution(owner, id, choice, receipt)? {
            return Ok(response);
        }
        let response = if choice == PromptChoice::Dismiss {
            self.cancel_prompt(owner, id)?;
            Response::PromptCanceled
        } else {
            Response::Prompt(self.answer_prompt(owner, id, choice)?)
        };
        if self.prompt_resolutions.len() == 1024 {
            self.prompt_resolutions.pop_front();
        }
        self.prompt_resolutions.push_back(PromptResolution {
            owner: owner.clone(),
            receipt,
            id: id.into(),
            choice,
            response: response.clone(),
        });
        Ok(response)
    }
    pub(crate) fn cancel_prompt(&mut self, owner: &AgentOwner, id: &str) -> Result<(), String> {
        if self
            .prompts
            .get(id)
            .is_some_and(|prompt| &prompt.owner != owner)
        {
            return Err("InvalidStateError: foreign permission prompt".into());
        }
        self.prompts.remove(id);
        Ok(())
    }
    fn bump(&mut self) -> Result<(), String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("platform revision exhausted")?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn permission(
        &mut self,
        origin: &str,
        descriptor: PermissionDescriptor,
        state: PermissionState,
    ) -> Result<(), String> {
        self.permission_partitioned(origin, origin, descriptor, state)
    }
    pub(crate) fn permission_partitioned(
        &mut self,
        top_origin: &str,
        origin: &str,
        descriptor: PermissionDescriptor,
        state: PermissionState,
    ) -> Result<(), String> {
        for origin in [top_origin, origin] {
            if origin.len() > 4096 {
                return Err("permission origin limit".into());
            }
            let parsed = url::Url::parse(origin).map_err(|_| "invalid permission origin")?;
            if parsed.origin().ascii_serialization() != origin || origin == "null" {
                return Err("permission origin must be a serialized nonopaque origin".into());
            }
        }
        let key = (top_origin.to_owned(), origin.to_owned(), descriptor.key()?);
        if state == PermissionState::Prompt {
            // Prompt is the default, so resetting an absent override is free
            // even at capacity or after revision exhaustion.
            if self.permissions.contains_key(&key) {
                self.bump()?;
                self.permissions.remove(&key);
            }
            return Ok(());
        }
        if !self.permissions.contains_key(&key) && self.permissions.len() >= 4096 {
            return Err("permission entry limit".into());
        }
        if self.permissions.get(&key) != Some(&state) {
            self.bump()?;
            self.permissions.insert(key, state);
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn location(
        &mut self,
        page: PageId,
        value: Option<Result<Coordinates, PositionError>>,
    ) -> Result<(), String> {
        self.locations(&BTreeSet::from([page]), value)
    }
    pub(crate) fn locations(
        &mut self,
        pages: &BTreeSet<PageId>,
        value: Option<Result<Coordinates, PositionError>>,
    ) -> Result<(), String> {
        if let Some(Ok(value)) = &value {
            value.validate()?;
        }
        if let Some(Err(error)) = &value
            && (error.code != 2 || error.message.len() > 4096)
        {
            return Err("invalid geolocation override error".into());
        }
        if value.is_some()
            && self.positions.len().saturating_add(
                pages
                    .iter()
                    .filter(|page| !self.positions.contains_key(page))
                    .count(),
            ) > 1024
        {
            return Err("location override limit".into());
        }
        if pages.is_empty() {
            return Ok(());
        }
        // Reserve revision/capacity for the whole request before mutating a page.
        self.bump()?;
        for page in pages {
            if let Some(value) = &value {
                self.positions.insert(*page, value.clone());
            } else {
                self.positions.remove(page);
            }
        }
        Ok(())
    }
    pub(crate) fn window_position(&self, page: PageId) -> (i32, i32) {
        self.window_positions.get(&page).copied().unwrap_or((0, 0))
    }
    pub(crate) fn move_window(&mut self, page: PageId, x: i32, y: i32) -> Result<(), String> {
        if self.window_position(page) == (x, y) {
            return Ok(());
        }
        if (x, y) != (0, 0)
            && !self.window_positions.contains_key(&page)
            && self.window_positions.len() >= 1024
        {
            return Err("window position capacity exceeded".into());
        }
        self.bump()?;
        if (x, y) == (0, 0) {
            self.window_positions.remove(&page);
        } else {
            self.window_positions.insert(page, (x, y));
        }
        Ok(())
    }
    pub(crate) fn visibility(&mut self, page: PageId, hidden: bool) -> Result<(), String> {
        if hidden && !self.hidden.contains(&page) && self.hidden.len() >= 1024 {
            return Err("hidden window limit".into());
        }
        if self.hidden.contains(&page) != hidden {
            self.bump()?;
            if hidden {
                self.hidden.insert(page);
            } else {
                self.hidden.remove(&page);
            }
        }
        Ok(())
    }
    pub(crate) fn registration_mode(
        &mut self,
        page: PageId,
        mode: RegistrationMode,
    ) -> Result<(), String> {
        if self.modes.get(&page).copied().unwrap_or_default() == mode {
            return Ok(());
        }
        if mode != RegistrationMode::None
            && !self.modes.contains_key(&page)
            && self.modes.len() >= 1024
        {
            return Err("registration mode limit".into());
        }
        self.bump()?;
        if mode == RegistrationMode::None {
            self.modes.remove(&page);
        } else {
            self.modes.insert(page, mode);
        }
        Ok(())
    }
    pub(crate) fn query(
        &mut self,
        page: PageId,
        origin: &str,
        request: Request,
    ) -> Result<Response, String> {
        self.query_partitioned(page, origin, origin, request)
    }
    pub(crate) fn query_partitioned(
        &mut self,
        page: PageId,
        top_origin: &str,
        origin: &str,
        request: Request,
    ) -> Result<Response, String> {
        match request {
            Request::Storage { .. } | Request::StorageEvents { .. } | Request::CspEvents { .. } => {
                Err("CSP events require native document authority".into())
            }
            Request::BluetoothDevices | Request::BluetoothTestingState => {
                Err("Bluetooth requires native document authority".into())
            }
            Request::InstallEligibility => {
                Err("install eligibility requires native document authority".into())
            }
            Request::UiLocaleData { locale } => {
                if locale.len() > 128
                    || !locale.split('-').all(|part| {
                        !part.is_empty()
                            && part.len() <= 8
                            && part.bytes().all(|b| b.is_ascii_alphanumeric())
                    })
                {
                    return Err("invalid control locale".into());
                }
                Ok(Response::UiLocaleData {
                    profile: crate::native_locale_data::profile(&locale).map(Box::new),
                })
            }
            Request::UiLocale => Ok(Response::UiLocale {
                locale: crate::native_locale::current(),
            }),
            Request::PromptBegin { .. }
            | Request::PromptAnswer { .. }
            | Request::PromptCancel { .. }
            | Request::PromptResolve { .. }
            | Request::Media { .. } => {
                Err("InvalidStateError: media requires native principal validation".into())
            }
            Request::Permission { descriptor } => Ok(Response::Permission {
                state: self.permission_value_partitioned(top_origin, origin, &descriptor)?,
                revision: self.revision,
            }),
            Request::PermissionsPolicySnapshot => {
                Err("SecurityError: policy inspection requires native document authority".into())
            }
            Request::WindowPosition => {
                let (x, y) = self.window_position(page);
                Ok(Response::WindowPosition { x, y })
            }
            Request::StorageAccess { .. } => {
                Err("storage access requires native document cookie authority".into())
            }
            Request::DocumentFocus => {
                Err("focus inspection requires native document authority".into())
            }
            Request::WindowState => Ok(Response::WindowState {
                hidden: self.hidden.contains(&page),
                revision: self.revision,
            }),
            Request::Geolocation => {
                let descriptor = PermissionDescriptor {
                    name: "geolocation".into(),
                    sysex: None,
                    pan_tilt_zoom: None,
                    allow_without_gesture: None,
                };
                let position =
                    if self.permission_value_partitioned(top_origin, origin, &descriptor)?
                        != PermissionState::Granted
                    {
                        Err(PositionError {
                            code: 1,
                            message: "Geolocation permission is not granted".into(),
                        })
                    } else {
                        self.positions.get(&page).cloned().unwrap_or_else(|| {
                            Err(PositionError {
                                code: 2,
                                message: "Native location provider has no position".into(),
                            })
                        })
                    };
                Ok(Response::Geolocation {
                    position,
                    revision: self.revision,
                })
            }
            operation @ (Request::RegisterProtocol { .. } | Request::UnregisterProtocol { .. }) => {
                let (scheme, url, remove) = match operation {
                    Request::RegisterProtocol { scheme, url } => (scheme, url, false),
                    Request::UnregisterProtocol { scheme, url } => (scheme, url, true),
                    _ => unreachable!(),
                };
                let scheme = oryn_js::platform_control::protocol_scheme(&scheme)?;
                if url.len() > 65536 {
                    return Err("protocol registration payload limit".into());
                }
                if !url.contains("%s") {
                    return Err("SyntaxError: handler URL has no substitution token".into());
                }
                let parsed =
                    url::Url::parse(&url).map_err(|_| "SyntaxError: invalid handler URL")?;
                if !matches!(parsed.scheme(), "http" | "https")
                    || parsed.origin().ascii_serialization() != origin
                {
                    return Err("SecurityError: handler URL has a foreign origin".into());
                }
                if remove {
                    let registered = self
                        .handlers
                        .get(&scheme)
                        .is_some_and(|(owner, handler)| owner == origin && handler == &url);
                    if registered {
                        self.bump()?;
                    }
                    self.prompts.retain(|_, prompt| {
                        prompt.snapshot.origin != origin
                            || prompt.protocol.as_ref() != Some(&(scheme.clone(), url.clone()))
                    });
                    if registered {
                        self.handlers.remove(&scheme);
                    }
                    return Ok(Response::ProtocolRegistration { accepted: false });
                }
                let accepted = match self.modes.get(&page).copied().unwrap_or_default() {
                    RegistrationMode::Accept => true,
                    RegistrationMode::Reject => false,
                    RegistrationMode::None => false,
                };
                if accepted {
                    let key = scheme;
                    if !self.handlers.contains_key(&key) && self.handlers.len() >= 1024 {
                        return Err("protocol handler entry limit".into());
                    }
                    let handler = (origin.to_owned(), url);
                    if self.handlers.get(&key) != Some(&handler) {
                        self.bump()?;
                        self.handlers.insert(key, handler);
                    }
                }
                Ok(Response::ProtocolRegistration { accepted })
            }
        }
    }
    pub(crate) fn resolve_navigation(&self, url: &url::Url) -> Result<Option<url::Url>, String> {
        let Some((_, template)) = self.handlers.get(url.scheme()) else {
            return Ok(None);
        };
        let mut encoded = String::new();
        for byte in url.as_str().bytes() {
            if byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
                )
            {
                encoded.push(byte as char);
            } else {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
        let substituted = template.replacen("%s", &encoded, 1);
        if substituted.len() > 1024 * 1024 {
            return Err("protocol navigation URL limit".into());
        }
        let resolved =
            url::Url::parse(&substituted).map_err(|_| "invalid registered handler URL")?;
        if !matches!(resolved.scheme(), "http" | "https") {
            return Err("registered handler no longer resolves to HTTP(S)".into());
        }
        Ok(Some(resolved))
    }
    #[cfg(test)]
    fn permission_value(
        &self,
        origin: &str,
        descriptor: &PermissionDescriptor,
    ) -> Result<PermissionState, String> {
        self.permission_value_partitioned(origin, origin, descriptor)
    }
    fn permission_value_partitioned(
        &self,
        top_origin: &str,
        origin: &str,
        descriptor: &PermissionDescriptor,
    ) -> Result<PermissionState, String> {
        let key = descriptor.key()?;
        if origin == "null" || top_origin == "null" {
            return Ok(PermissionState::Denied);
        }
        Ok(self
            .permissions
            .get(&(top_origin.to_owned(), origin.to_owned(), key))
            .copied()
            .unwrap_or(PermissionState::Prompt))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_reset_is_capacity_free_and_idempotent() {
        let mut state = State::default();
        for id in 0..1024 {
            state
                .registration_mode(PageId(id), RegistrationMode::Accept)
                .unwrap();
        }
        let revision = state.revision;
        state
            .registration_mode(PageId(2048), RegistrationMode::None)
            .unwrap();
        assert_eq!(state.revision, revision);
        assert_eq!(state.modes.len(), 1024);
        state
            .registration_mode(PageId(0), RegistrationMode::None)
            .unwrap();
        assert_eq!(state.modes.len(), 1023);
        state
            .registration_mode(PageId(2048), RegistrationMode::Reject)
            .unwrap();
        let revision = state.revision;
        state
            .registration_mode(PageId(2048), RegistrationMode::Reject)
            .unwrap();
        assert_eq!(state.revision, revision);
    }

    fn geo() -> PermissionDescriptor {
        PermissionDescriptor {
            name: "geolocation".into(),
            sysex: None,
            pan_tilt_zoom: None,
            allow_without_gesture: None,
        }
    }
    #[test]
    fn page_retirement_reclaims_window_overrides_but_keeps_context_grants() {
        let mut state = State::default();
        let page = PageId(1);
        state
            .permission("https://a.test", geo(), PermissionState::Granted)
            .unwrap();
        state.visibility(page, true).unwrap();
        state
            .registration_mode(page, RegistrationMode::Accept)
            .unwrap();
        state
            .location(
                page,
                Some(Err(PositionError {
                    code: 2,
                    message: "test".into(),
                })),
            )
            .unwrap();
        state.retire_page(page);
        state.retire_page(page);
        assert!(state.positions.is_empty() && state.hidden.is_empty() && state.modes.is_empty());
        assert_eq!(
            state.permission_value("https://a.test", &geo()).unwrap(),
            PermissionState::Granted
        );
    }
    #[test]
    fn permissions_are_origin_scoped_and_location_requires_grant() {
        let mut state = State::default();
        let page = PageId(1);
        state
            .location(
                page,
                Some(Ok(Coordinates {
                    latitude: 50.,
                    longitude: 0.,
                    accuracy: 100.,
                    altitude: None,
                    altitude_accuracy: None,
                    heading: None,
                    speed: None,
                })),
            )
            .unwrap();
        assert!(matches!(
            state
                .query(page, "https://a.test", Request::Geolocation)
                .unwrap(),
            Response::Geolocation {
                position: Err(PositionError { code: 1, .. }),
                ..
            }
        ));
        state
            .permission("https://a.test", geo(), PermissionState::Granted)
            .unwrap();
        assert!(matches!(
            state
                .query(page, "https://a.test", Request::Geolocation)
                .unwrap(),
            Response::Geolocation {
                position: Ok(_),
                ..
            }
        ));
        assert!(matches!(
            state
                .query(
                    page,
                    "https://b.test",
                    Request::Permission { descriptor: geo() }
                )
                .unwrap(),
            Response::Permission {
                state: PermissionState::Prompt,
                ..
            }
        ));
        assert!(matches!(
            state
                .query(PageId(2), "https://a.test", Request::Geolocation)
                .unwrap(),
            Response::Geolocation {
                position: Err(PositionError { code: 2, .. }),
                ..
            }
        ));
    }
    #[test]
    fn embedded_permission_partitions_isolate_top_level_origin_and_native_consumers() {
        let mut state = State::default();
        let page = PageId(1);
        state
            .permission_partitioned(
                "https://top.test",
                "https://embedded.test",
                geo(),
                PermissionState::Granted,
            )
            .unwrap();
        for (top, embedded, expected) in [
            (
                "https://top.test",
                "https://embedded.test",
                PermissionState::Granted,
            ),
            (
                "https://other.test",
                "https://embedded.test",
                PermissionState::Prompt,
            ),
            (
                "https://embedded.test",
                "https://embedded.test",
                PermissionState::Prompt,
            ),
            (
                "https://top.test",
                "https://top.test",
                PermissionState::Prompt,
            ),
            ("null", "https://embedded.test", PermissionState::Denied),
        ] {
            assert_eq!(
                state
                    .permission_value_partitioned(top, embedded, &geo())
                    .unwrap(),
                expected
            );
        }
        state
            .location(
                page,
                Some(Ok(Coordinates {
                    latitude: 50.,
                    longitude: 0.,
                    accuracy: 100.,
                    altitude: None,
                    altitude_accuracy: None,
                    heading: None,
                    speed: None,
                })),
            )
            .unwrap();
        assert!(matches!(
            state
                .query_partitioned(
                    page,
                    "https://top.test",
                    "https://embedded.test",
                    Request::Geolocation
                )
                .unwrap(),
            Response::Geolocation {
                position: Ok(_),
                ..
            }
        ));
        assert!(matches!(
            state
                .query_partitioned(
                    page,
                    "https://other.test",
                    "https://embedded.test",
                    Request::Geolocation
                )
                .unwrap(),
            Response::Geolocation {
                position: Err(PositionError { code: 1, .. }),
                ..
            }
        ));
        let revision = state.revision;
        let entries = state.permissions.clone();
        for (top, embedded) in [
            ("null", "https://embedded.test"),
            ("https://top.test", "null"),
            ("https://top.test/path", "https://embedded.test"),
            ("https://top.test", "not an origin"),
        ] {
            assert!(
                state
                    .permission_partitioned(top, embedded, geo(), PermissionState::Denied)
                    .is_err()
            );
        }
        assert_eq!(state.revision, revision);
        assert_eq!(state.permissions, entries);
    }
    #[test]
    fn permission_prompt_answer_retains_embedding_partition() {
        let mut state = State::default();
        let owner = AgentOwner::Window {
            page: PageId(1),
            target: oryn_js::message_wire::WindowTarget::Frame("frame".into()),
            generation: 1,
            epoch: 0,
        };
        let prompt = state
            .begin_prompt_partitioned(
                &owner,
                "https://top.test",
                "https://embedded.test",
                PromptKind::Geolocation,
            )
            .unwrap();
        assert_eq!(prompt.state, PermissionState::Prompt);
        state
            .answer_prompt(&owner, &prompt.id, PromptChoice::Allow)
            .unwrap();
        assert_eq!(
            state
                .permission_value_partitioned("https://top.test", "https://embedded.test", &geo())
                .unwrap(),
            PermissionState::Granted
        );
        assert_eq!(
            state
                .permission_value_partitioned("https://other.test", "https://embedded.test", &geo())
                .unwrap(),
            PermissionState::Prompt
        );
        assert_eq!(
            state
                .permission_value("https://embedded.test", &geo())
                .unwrap(),
            PermissionState::Prompt
        );
    }
    #[test]
    fn registration_mode_cannot_bypass_scheme_or_origin_checks() {
        let mut state = State::default();
        let page = PageId(1);
        state
            .registration_mode(page, RegistrationMode::Accept)
            .unwrap();
        for (scheme, url) in [
            ("javascript", "https://a.test/%s"),
            ("web+test", "https://b.test/%s"),
            ("web+test", "https://a.test/no-token"),
        ] {
            assert!(
                state
                    .query(
                        page,
                        "https://a.test",
                        Request::RegisterProtocol {
                            scheme: scheme.into(),
                            url: url.into()
                        }
                    )
                    .is_err()
            );
        }
        assert!(matches!(
            state
                .query(
                    page,
                    "https://a.test",
                    Request::RegisterProtocol {
                        scheme: "web+test".into(),
                        url: "https://a.test/%s".into()
                    }
                )
                .unwrap(),
            Response::ProtocolRegistration { accepted: true }
        ));
    }
    #[test]
    fn handler_navigation_substitutes_encoded_canonical_url_without_rewriting_resources() {
        let mut state = State::default();
        state
            .registration_mode(PageId(1), RegistrationMode::Accept)
            .unwrap();
        state
            .query(
                PageId(1),
                "https://a.test",
                Request::RegisterProtocol {
                    scheme: "web+test".into(),
                    url: "https://a.test/handler?q=%s#tail".into(),
                },
            )
            .unwrap();
        let target = url::Url::parse("web+test:%01abc?x=1#f").unwrap();
        assert_eq!(
            state.resolve_navigation(&target).unwrap().unwrap().as_str(),
            "https://a.test/handler?q=web%2Btest%3A%2501abc%3Fx%3D1%23f#tail"
        );
        assert!(
            state
                .resolve_navigation(&url::Url::parse("https://a.test/ordinary").unwrap())
                .unwrap()
                .is_none()
        );
        assert!(
            State::default()
                .resolve_navigation(&target)
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod permission_prompt_tests {
    use super::*;
    fn owner(generation: u64) -> AgentOwner {
        AgentOwner::Window {
            page: PageId(1),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation,
            epoch: 0,
        }
    }
    #[test]
    fn microphone_prompt_grants_only_audio_and_rejects_retired_owner() {
        let mut state = State::default();
        let first = owner(1);
        let origin = "https://capture.test";
        let prompt = state
            .begin_prompt(&first, origin, PromptKind::Microphone)
            .unwrap();
        state
            .answer_prompt(&first, &prompt.id, PromptChoice::Allow)
            .unwrap();
        for (name, expected) in [
            ("microphone", PermissionState::Granted),
            ("camera", PermissionState::Prompt),
        ] {
            let descriptor = PermissionDescriptor {
                name: name.into(),
                sysex: None,
                pan_tilt_zoom: None,
                allow_without_gesture: None,
            };
            assert_eq!(
                state.permission_value(origin, &descriptor).unwrap(),
                expected
            );
        }
        let pending = state
            .begin_prompt(&owner(2), "https://other.test", PromptKind::Microphone)
            .unwrap();
        state.retire_owner(&owner(2));
        assert!(
            state
                .answer_prompt(&owner(2), &pending.id, PromptChoice::Allow)
                .is_err()
        );
    }
    #[test]
    fn explicit_native_choice_mutates_both_grants_and_rejects_foreign_or_replayed_prompt() {
        let mut state = State::default();
        let first = owner(1);
        let prompt = state
            .begin_prompt(&first, "https://capture.test", PromptKind::UserMedia)
            .unwrap();
        assert_eq!(prompt.state, PermissionState::Prompt);
        assert!(
            state
                .answer_prompt(&owner(2), &prompt.id, PromptChoice::Allow)
                .is_err()
        );
        let accepted = state
            .answer_prompt(&first, &prompt.id, PromptChoice::Allow)
            .unwrap();
        assert_eq!(accepted.state, PermissionState::Granted);
        for name in ["camera", "microphone"] {
            assert_eq!(
                state
                    .permission_value(
                        "https://capture.test",
                        &PermissionDescriptor {
                            name: name.into(),
                            sysex: None,
                            pan_tilt_zoom: None,
                            allow_without_gesture: None
                        }
                    )
                    .unwrap(),
                PermissionState::Granted
            )
        }
        assert!(
            state
                .answer_prompt(&first, &prompt.id, PromptChoice::Allow)
                .is_err()
        );
    }
    #[test]
    fn native_resolution_lost_ack_replays_without_regrant_and_rejects_conflicts() {
        let mut state = State::default();
        let first = owner(1);
        let origin = "https://capture.test";
        let descriptor = || PermissionDescriptor {
            name: "geolocation".into(),
            sysex: None,
            pan_tilt_zoom: None,
            allow_without_gesture: None,
        };
        let prompt = state
            .begin_prompt(&first, origin, PromptKind::Geolocation)
            .unwrap();
        assert!(
            matches!(state.resolve_prompt(&first,&prompt.id,PromptChoice::Allow,17).unwrap(),Response::Prompt(p)if p.state==PermissionState::Granted)
        );
        state
            .permission(origin, descriptor(), PermissionState::Denied)
            .unwrap();
        let revision = state.revision;
        assert!(
            matches!(state.resolve_prompt(&first,&prompt.id,PromptChoice::Allow,17).unwrap(),Response::Prompt(p)if p.state==PermissionState::Granted)
        );
        assert_eq!(state.revision, revision);
        assert_eq!(
            state.permission_value(origin, &descriptor()).unwrap(),
            PermissionState::Denied
        );
        assert!(
            state
                .resolve_prompt(&first, &prompt.id, PromptChoice::Deny, 17)
                .is_err()
        );
        assert!(
            state
                .resolve_prompt(&owner(2), &prompt.id, PromptChoice::Allow, 17)
                .is_err()
        );
        assert!(
            state
                .answer_prompt(&first, &prompt.id, PromptChoice::Allow)
                .is_err()
        );
        state
            .permission(origin, descriptor(), PermissionState::Prompt)
            .unwrap();
        let next = state
            .begin_prompt(&first, origin, PromptKind::Geolocation)
            .unwrap();
        assert!(
            state
                .resolve_prompt(&first, &next.id, PromptChoice::Allow, 17)
                .is_err()
        );
        assert!(state.prompts.contains_key(&next.id));
        assert!(matches!(
            state
                .resolve_prompt(&first, &next.id, PromptChoice::Dismiss, 18)
                .unwrap(),
            Response::PromptCanceled
        ));
        assert!(matches!(
            state
                .resolve_prompt(&first, &next.id, PromptChoice::Dismiss, 18)
                .unwrap(),
            Response::PromptCanceled
        ));
        state.retire_owner(&first);
        assert!(state.prompt_resolutions.is_empty());
        assert!(
            state
                .resolve_prompt(&first, &next.id, PromptChoice::Dismiss, 18)
                .is_err()
        );
    }
    #[test]
    fn native_resolution_failure_keeps_prompt_and_pruning_never_resurrects_it() {
        let mut state = State::default();
        let first = owner(1);
        let origin = "https://capture.test";
        let prompt = state
            .begin_prompt(&first, origin, PromptKind::Geolocation)
            .unwrap();
        state.revision = u64::MAX;
        assert!(
            state
                .resolve_prompt(&first, &prompt.id, PromptChoice::Allow, 1)
                .is_err()
        );
        assert!(state.prompt_resolutions.is_empty());
        assert!(state.prompts.contains_key(&prompt.id));
        state.revision = 0;
        state
            .resolve_prompt(&first, &prompt.id, PromptChoice::Dismiss, 1)
            .unwrap();
        for receipt in 2..=1025 {
            let next = state
                .begin_prompt(&first, origin, PromptKind::Geolocation)
                .unwrap();
            state
                .resolve_prompt(&first, &next.id, PromptChoice::Dismiss, receipt)
                .unwrap();
        }
        assert_eq!(state.prompt_resolutions.len(), 1024);
        assert!(
            state
                .resolve_prompt(&first, &prompt.id, PromptChoice::Dismiss, 1)
                .is_err()
        );
        state.retire_page(PageId(1));
        assert!(state.prompt_resolutions.is_empty());
    }
    #[test]
    fn bluetooth_test_consumption_is_real_owner_scoped_accounting() {
        let mut state = State::default();
        let first = owner(1);
        let second = owner(2);
        state
            .bluetooth_pending
            .insert(first.clone(), BTreeSet::from([17]));
        assert!(matches!(
            state.bluetooth_snapshot(&first, "https://test", true, true, Vec::new()),
            Response::Bluetooth {
                pending_responses: 1,
                ..
            }
        ));
        assert!(matches!(
            state.bluetooth_snapshot(&second, "https://test", true, true, Vec::new()),
            Response::Bluetooth {
                pending_responses: 0,
                ..
            }
        ));
        state.retire_owner(&first);
        assert!(state.bluetooth_pending.is_empty());
        assert!(
            matches!(state.bluetooth_snapshot(&first,"https://test",false,true,Vec::new()),Response::Bluetooth{testing_enabled:false,devices,..}if devices.is_empty())
        );
    }
    #[test]
    fn dismissal_and_retirement_do_not_create_permission() {
        let mut state = State::default();
        let owner = owner(1);
        let prompt = state
            .begin_prompt(&owner, "https://capture.test", PromptKind::Geolocation)
            .unwrap();
        state.cancel_prompt(&owner, &prompt.id).unwrap();
        assert!(state.permissions.is_empty());
        let prompt = state
            .begin_prompt(&owner, "https://capture.test", PromptKind::Geolocation)
            .unwrap();
        state.retire_owner(&owner);
        assert!(
            state
                .answer_prompt(&owner, &prompt.id, PromptChoice::Allow)
                .is_err()
        );
        assert!(state.permissions.is_empty());
    }
}

#[cfg(test)]
mod protocol_handler_tests {
    use super::*;
    fn owner(page: u64) -> AgentOwner {
        AgentOwner::Window {
            page: PageId(page),
            target: oryn_js::message_wire::WindowTarget::Root,
            generation: 1,
            epoch: 0,
        }
    }
    #[test]
    fn default_handler_registration_owns_prompt_and_unregister_cancels_exact_template() {
        let mut state = State::default();
        let origin = "https://handler.test";
        let url = "https://handler.test/?q=%s";
        let Response::Prompt(prompt) = state
            .register_protocol(&owner(1), PageId(1), origin, "WeB+Demo".into(), url.into())
            .unwrap()
        else {
            panic!("native prompt")
        };
        assert_eq!(prompt.kind, PromptKind::Protocol);
        assert!(state.handlers.is_empty());
        assert!(state.permissions.is_empty());
        assert!(
            state
                .answer_prompt(&owner(2), &prompt.id, PromptChoice::Allow)
                .is_err()
        );
        state
            .answer_prompt(&owner(1), &prompt.id, PromptChoice::Allow)
            .unwrap();
        assert!(state.permissions.is_empty());
        assert!(
            state
                .resolve_navigation(&url::Url::parse("web+demo:hello").unwrap())
                .unwrap()
                .is_some()
        );
        state
            .query(
                PageId(1),
                origin,
                Request::UnregisterProtocol {
                    scheme: "WEB+DEMO".into(),
                    url: "https://handler.test/other?%s".into(),
                },
            )
            .unwrap();
        assert!(!state.handlers.is_empty());
        state
            .query(
                PageId(1),
                origin,
                Request::UnregisterProtocol {
                    scheme: "WEB+DEMO".into(),
                    url: url.into(),
                },
            )
            .unwrap();
        assert!(state.handlers.is_empty());
        let Response::Prompt(prompt) = state
            .register_protocol(&owner(1), PageId(1), origin, "sftp".into(), url.into())
            .unwrap()
        else {
            panic!()
        };
        state
            .query(
                PageId(1),
                origin,
                Request::UnregisterProtocol {
                    scheme: "SFTP".into(),
                    url: url.into(),
                },
            )
            .unwrap();
        assert!(
            state
                .answer_prompt(&owner(1), &prompt.id, PromptChoice::Allow)
                .is_err()
        );
        assert!(state.handlers.is_empty());
    }
    #[test]
    fn safelist_case_and_unregister_validation_match_registration() {
        let mut state = State::default();
        let page = PageId(1);
        for scheme in [
            "FTP",
            "FtPs",
            "SFTP",
            "TEL",
            "web+abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
        ] {
            assert!(
                state
                    .query(
                        page,
                        "https://handler.test",
                        Request::RegisterProtocol {
                            scheme: scheme.into(),
                            url: "https://handler.test/%s".into()
                        }
                    )
                    .is_ok()
            );
            assert!(
                state
                    .query(
                        page,
                        "https://handler.test",
                        Request::UnregisterProtocol {
                            scheme: scheme.into(),
                            url: "https://handler.test/%s".into()
                        }
                    )
                    .is_ok()
            );
        }
        for (scheme, url) in [
            ("javascript", "https://handler.test/%s"),
            ("web+", "https://handler.test/%s"),
            ("tel", "https://foreign.test/%s"),
            ("tel", "https://handler.test/no-token"),
        ] {
            assert!(
                state
                    .query(
                        page,
                        "https://handler.test",
                        Request::UnregisterProtocol {
                            scheme: scheme.into(),
                            url: url.into()
                        }
                    )
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod location_batch_tests {
    use super::*;
    #[test]
    fn batch_capacity_and_revision_failures_preserve_every_existing_override() {
        let mut state = State::default();
        let error = Err(PositionError {
            code: 2,
            message: "original".into(),
        });
        for id in 1..=1024 {
            state.positions.insert(PageId(id), error.clone());
        }
        let revision = state.revision;
        assert!(
            state
                .locations(
                    &BTreeSet::from([PageId(1), PageId(1025)]),
                    Some(Err(PositionError {
                        code: 2,
                        message: "replacement".into()
                    }))
                )
                .is_err()
        );
        assert_eq!(state.revision, revision);
        assert_eq!(state.positions[&PageId(1)], error);
        assert!(!state.positions.contains_key(&PageId(1025)));
        state
            .locations(&BTreeSet::from([PageId(1), PageId(1025)]), None)
            .unwrap();
        assert!(!state.positions.contains_key(&PageId(1)));
        assert_eq!(state.positions.len(), 1023);
        state.revision = u64::MAX;
        assert!(
            state
                .locations(&BTreeSet::from([PageId(2), PageId(3)]), None)
                .is_err()
        );
        assert!(state.positions.contains_key(&PageId(2)));
        assert!(state.positions.contains_key(&PageId(3)));
    }
    #[test]
    fn permission_prompt_reset_reclaims_capacity_and_is_idempotent() {
        let geo = || PermissionDescriptor {
            name: "geolocation".into(),
            sysex: None,
            pan_tilt_zoom: None,
            allow_without_gesture: None,
        };
        let mut state = State::default();
        for index in 0..4096 {
            state
                .permission(
                    &format!("https://site{index}.test"),
                    geo(),
                    PermissionState::Granted,
                )
                .unwrap();
        }
        let revision = state.revision;
        state
            .permission("https://absent.test", geo(), PermissionState::Prompt)
            .unwrap();
        assert_eq!(state.revision, revision);
        assert!(
            state
                .permission("https://new.test", geo(), PermissionState::Granted)
                .is_err()
        );
        state
            .permission("https://site0.test", geo(), PermissionState::Prompt)
            .unwrap();
        assert_eq!(state.permissions.len(), 4095);
        assert_eq!(
            state
                .permission_value_partitioned("https://site0.test", "https://site0.test", &geo())
                .unwrap(),
            PermissionState::Prompt
        );
        state
            .permission("https://new.test", geo(), PermissionState::Granted)
            .unwrap();
        state.revision = u64::MAX;
        state
            .permission("https://absent.test", geo(), PermissionState::Prompt)
            .unwrap();
        assert!(
            state
                .permission("https://new.test", geo(), PermissionState::Prompt)
                .is_err()
        );
        assert_eq!(
            state
                .permission_value_partitioned("https://new.test", "https://new.test", &geo())
                .unwrap(),
            PermissionState::Granted
        );
    }

    #[test]
    fn protocol_registration_revision_failure_does_not_publish_or_remove_handler() {
        let mut state = State::default();
        let page = PageId(1);
        let origin = "https://handler.test";
        state
            .registration_mode(page, RegistrationMode::Accept)
            .unwrap();
        let register = || Request::RegisterProtocol {
            scheme: "web+demo".into(),
            url: "https://handler.test/?q=%s".into(),
        };
        let saved = state.revision;
        state.revision = u64::MAX;
        assert!(state.query(page, origin, register()).is_err());
        assert!(state.handlers.is_empty());
        state.revision = saved;
        state.query(page, origin, register()).unwrap();
        state.revision = u64::MAX;
        // Re-registering the same accepted handler is an idempotent success.
        assert!(state.query(page, origin, register()).is_ok());
        assert!(
            state
                .query(
                    page,
                    origin,
                    Request::UnregisterProtocol {
                        scheme: "web+demo".into(),
                        url: "https://handler.test/?q=%s".into()
                    }
                )
                .is_err()
        );
        assert!(
            state
                .resolve_navigation(&url::Url::parse("web+demo:retained").unwrap())
                .unwrap()
                .is_some()
        );
    }
}

#[cfg(test)]
mod window_position_tests {
    use super::*;
    #[test]
    fn native_position_publication_is_atomic_bounded_and_reclaims_on_retirement() {
        let mut state = State::default();
        let page = PageId(17);
        state.move_window(page, 100, 200).unwrap();
        let revision = state.revision;
        state.move_window(page, 100, 200).unwrap();
        assert_eq!(state.revision, revision);
        state.revision = u64::MAX;
        assert!(state.move_window(page, 300, 400).is_err());
        assert_eq!(state.window_position(page), (100, 200));
        state.retire_page(page);
        assert_eq!(state.window_position(page), (0, 0));
        assert!(state.window_positions.is_empty());
    }
}
