//! Enforcing cross-origin-isolated Permissions-Policy declarations. Native
//! response/ancestor identities are inputs; report-only fields do not grant it.
fn origin_matches(pattern: &str, target: &url::Url) -> bool {
    if let Some((scheme, host)) = pattern.split_once("://*.") {
        let Some(actual) = target.host_str() else {
            return false;
        };
        let Ok(base) = url::Url::parse(&format!("{scheme}://{host}")) else {
            return false;
        };
        if !base.username().is_empty()
            || base.password().is_some()
            || base.path() != "/"
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return false;
        }
        let Some(suffix) = base.host_str() else {
            return false;
        };
        return target.scheme() == scheme
            && target.port_or_known_default() == base.port_or_known_default()
            && actual
                .strip_suffix(suffix)
                .is_some_and(|prefix| prefix.ends_with('.') && !prefix.is_empty());
    }
    url::Url::parse(pattern).is_ok_and(|origin| {
        origin.username().is_empty()
            && origin.password().is_none()
            && origin.path() == "/"
            && origin.query().is_none()
            && origin.fragment().is_none()
            && origin.origin() == target.origin()
    })
}
fn members(value: &str) -> Option<Vec<String>> {
    let mut rest = value.trim();
    let mut result = Vec::new();
    while !rest.is_empty() {
        if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"')?;
            let token = &quoted[..end];
            if token.contains('\\') || token.bytes().any(|byte| byte < 0x20 || byte > 0x7e) {
                return None;
            }
            result.push(token.to_owned());
            rest = &quoted[end + 1..];
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let token = &rest[..end];
            if token != "self" {
                return None;
            }
            result.push(token.to_owned());
            rest = &rest[end..];
        }
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        rest = rest.trim_start();
        if result.len() > 256 {
            return None;
        }
    }
    Some(result)
}
/// Missing declaration imposes no additional inherited restriction. Container
/// default-self behavior is evaluated separately at the committed iframe edge.
pub fn allows(headers: &[(String, String)], document_origin: &str, target_origin: &str) -> bool {
    allows_feature(
        headers,
        "cross-origin-isolated",
        document_origin,
        target_origin,
    )
}
pub fn allows_feature(
    headers: &[(String, String)],
    feature: &str,
    document_origin: &str,
    target_origin: &str,
) -> bool {
    let Ok(target) = url::Url::parse(target_origin) else {
        return false;
    };
    let mut seen = false;
    for (_, header) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
    {
        for directive in header.split(',') {
            let Some((name, value)) = directive.trim().split_once('=') else {
                continue;
            };
            if name.trim() != feature {
                continue;
            }
            if seen {
                return false;
            }
            seen = true;
            let value = value.trim();
            if value == "*" {
                continue;
            }
            let Some(value) = value
                .strip_prefix('(')
                .and_then(|value| value.strip_suffix(')'))
            else {
                return false;
            };
            let Some(values) = members(value) else {
                return false;
            };
            if !values.iter().any(|value| {
                if value == "self" {
                    document_origin == target_origin
                } else {
                    origin_matches(value, &target)
                }
            }) {
                return false;
            }
        }
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolation_allowlist_uses_native_origin_and_ignores_report_only() {
        let fields = |value: &str| vec![("Permissions-Policy".into(), value.into())];
        assert!(allows(
            &fields("cross-origin-isolated=(self \"https://b.test\")"),
            "https://a.test",
            "https://b.test"
        ));
        assert!(!allows(
            &fields("cross-origin-isolated=(self)"),
            "https://a.test",
            "https://b.test"
        ));
        assert!(!allows(
            &fields("cross-origin-isolated=()"),
            "https://a.test",
            "https://a.test"
        ));
        assert!(allows(
            &fields("cross-origin-isolated=(\"https://*.b.test\")"),
            "https://a.test",
            "https://sub.b.test"
        ));
        assert!(!allows(
            &fields("cross-origin-isolated=(\"https://*.b.test\")"),
            "https://a.test",
            "https://b.test"
        ));
        assert!(!allows(
            &fields("cross-origin-isolated=(\"https://*.b.test\")"),
            "https://a.test",
            "https://sub.b.test:444"
        ));
        assert!(!allows(
            &fields("cross-origin-isolated=*, cross-origin-isolated=()"),
            "https://a.test",
            "https://a.test"
        ));
        assert!(allows(
            &[(
                "Permissions-Policy-Report-Only".into(),
                "cross-origin-isolated=()".into()
            )],
            "https://a.test",
            "https://a.test"
        ));
    }
}

pub fn container_allows(
    allow: &str,
    parent_origin: &str,
    source_url: &str,
    target_origin: &str,
) -> bool {
    container_allows_feature(
        allow,
        "cross-origin-isolated",
        parent_origin,
        source_url,
        target_origin,
    )
}
pub fn container_allows_feature(
    allow: &str,
    feature: &str,
    parent_origin: &str,
    source_url: &str,
    target_origin: &str,
) -> bool {
    let Ok(target) = url::Url::parse(target_origin) else {
        return false;
    };
    let source = url::Url::parse(source_url).ok().map(|url| {
        if url.scheme() == "about" {
            parent_origin.to_owned()
        } else {
            url.origin().ascii_serialization()
        }
    });
    for directive in allow.split(';') {
        let mut tokens = directive.split_ascii_whitespace();
        if tokens.next() != Some(feature) {
            continue;
        }
        let tokens = tokens.collect::<Vec<_>>();
        if tokens.is_empty() {
            return source.as_deref() == Some(target_origin);
        }
        if tokens.contains(&"'none'") {
            return false;
        }
        return tokens.iter().any(|token| match *token {
            "*" => true,
            "'self'" => parent_origin == target_origin,
            "'src'" => source.as_deref() == Some(target_origin),
            origin => origin_matches(origin, &target),
        });
    }
    matches!(feature, "picture-in-picture" | "deferred-fetch-minimal")
        || parent_origin == target_origin
}

#[cfg(test)]
mod container_tests {
    use super::*;
    #[test]
    fn committed_container_defaults_src_and_origin_delegation() {
        assert!(container_allows(
            "",
            "https://a.test",
            "https://a.test/frame",
            "https://a.test"
        ));
        assert!(!container_allows(
            "",
            "https://a.test",
            "https://b.test/frame",
            "https://b.test"
        ));
        assert!(container_allows(
            "cross-origin-isolated",
            "https://a.test",
            "https://b.test/frame",
            "https://b.test"
        ));
        assert!(!container_allows(
            "cross-origin-isolated 'src'",
            "https://a.test",
            "https://b.test/frame",
            "https://redirect.test"
        ));
        assert!(container_allows(
            "cross-origin-isolated 'self'",
            "https://a.test",
            "about:srcdoc",
            "https://a.test"
        ));
        assert!(container_allows(
            "camera 'none'; cross-origin-isolated https://b.test",
            "https://a.test",
            "https://c.test",
            "https://b.test"
        ));
        assert!(!container_allows(
            "cross-origin-isolated 'none' *",
            "https://a.test",
            "https://b.test",
            "https://b.test"
        ));
        assert!(container_allows(
            "cross-origin-isolated *",
            "https://a.test",
            "https://b.test",
            "https://b.test"
        ));
        assert!(
            !allows(
                &[(
                    "permissions-policy".into(),
                    "cross-origin-isolated=(self)".into()
                )],
                "https://a.test",
                "https://b.test"
            ),
            "container wildcard cannot override ancestor declaration"
        );
    }
}

#[cfg(test)]
mod geolocation_feature_tests {
    use super::*;
    #[test]
    fn geolocation_policy_uses_feature_specific_headers_and_container_origins() {
        let headers = vec![(
            "Permissions-Policy".into(),
            "camera=*, geolocation=(self \"https://child.test\")".into(),
        )];
        assert!(allows_feature(
            &headers,
            "geolocation",
            "https://top.test",
            "https://child.test"
        ));
        assert!(!allows_feature(
            &headers,
            "geolocation",
            "https://top.test",
            "https://other.test"
        ));
        assert!(allows_feature(
            &headers,
            "camera",
            "https://top.test",
            "https://other.test"
        ));
        assert!(!container_allows_feature(
            "camera *",
            "geolocation",
            "https://top.test",
            "https://child.test/frame",
            "https://child.test"
        ));
        assert!(container_allows_feature(
            "geolocation",
            "geolocation",
            "https://top.test",
            "https://child.test/frame",
            "https://child.test"
        ));
        assert!(!container_allows_feature(
            "geolocation",
            "geolocation",
            "https://top.test",
            "https://child.test/frame",
            "https://redirect.test"
        ));
        assert!(!container_allows_feature(
            "geolocation 'none' *",
            "geolocation",
            "https://top.test",
            "https://child.test/frame",
            "https://child.test"
        ));
    }
}

#[cfg(test)]
mod pip_policy_tests {
    use super::*;
    #[test]
    fn pip_default_star_does_not_override_explicit_restrictions() {
        assert!(container_allows_feature(
            "",
            "picture-in-picture",
            "https://a.test",
            "https://b.test/video",
            "https://b.test"
        ));
        assert!(!container_allows_feature(
            "picture-in-picture 'none'",
            "picture-in-picture",
            "https://a.test",
            "https://b.test/video",
            "https://b.test"
        ));
        assert!(!container_allows_feature(
            "picture-in-picture 'src'",
            "picture-in-picture",
            "https://a.test",
            "https://b.test/video",
            "https://c.test"
        ));
        assert!(!allows_feature(
            &[("permissions-policy".into(), "picture-in-picture=()".into())],
            "picture-in-picture",
            "https://a.test",
            "https://b.test"
        ));
    }
}

/// Features with native enforcement consumers. Advertising recognition does
/// not imply that a device or a permission grant exists.
pub const FEATURES: &[&str] = &[
    "accelerometer",
    "autoplay",
    "bluetooth",
    "camera",
    "clipboard-read",
    "clipboard-write",
    "cross-origin-isolated",
    "deferred-fetch",
    "deferred-fetch-minimal",
    "display-capture",
    "document-domain",
    "encrypted-media",
    "fullscreen",
    "geolocation",
    "gyroscope",
    "local-fonts",
    "magnetometer",
    "microphone",
    "midi",
    "payment",
    "picture-in-picture",
    "screen-wake-lock",
    "speaker-selection",
    "storage-access",
    "usb",
    "web-share",
    "window-management",
    "xr-spatial-tracking",
];
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Constraint {
    Header {
        origin: String,
        headers: Vec<(String, String)>,
    },
    Container {
        origin: String,
        source: String,
        allow: String,
    },
    GateContainer {
        origin: String,
        source: String,
        allow: String,
        target: String,
    },
    /// Ancestor admission for its own document must remain enabled before any
    /// descendant allowlist can delegate the feature.
    Gate {
        origin: String,
        target: String,
        headers: Vec<(String, String)>,
    },
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub origin: String,
    pub container: bool,
    pub constraints: Vec<Constraint>,
}
#[derive(Clone, Debug)]
enum List {
    All,
    Origins(Vec<String>),
}
fn default_list(feature: &str, origin: &str) -> List {
    if matches!(
        feature,
        "picture-in-picture" | "document-domain" | "deferred-fetch-minimal"
    ) {
        List::All
    } else {
        List::Origins(vec![origin.into()])
    }
}
fn valid_origin(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    (!url.cannot_be_a_base())
        .then(|| url.origin().ascii_serialization())
        .filter(|origin| origin != "null")
}
fn origin_pattern(value: &str) -> Option<String> {
    if let Some((scheme, host)) = value.split_once("://*.") {
        let probe = url::Url::parse(&format!("{scheme}://oryn-policy-probe.{host}")).ok()?;
        return origin_matches(value, &probe).then(|| value.to_owned());
    }
    let url = url::Url::parse(value).ok()?;
    origin_matches(value, &url).then(|| url.origin().ascii_serialization())
}
fn header_list(headers: &[(String, String)], feature: &str, origin: &str) -> Option<List> {
    let mut found = None;
    for (_, header) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
    {
        for directive in header.split(',') {
            let Some((name, value)) = directive.trim().split_once('=') else {
                continue;
            };
            if name.trim() != feature {
                continue;
            }
            if found.is_some() {
                return Some(List::Origins(Vec::new()));
            }
            let value = value.trim();
            found = Some(if value == "*" {
                List::All
            } else {
                List::Origins(
                    value
                        .strip_prefix('(')
                        .and_then(|value| value.strip_suffix(')'))
                        .and_then(members)
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|value| {
                            if value == "self" {
                                Some(origin.into())
                            } else {
                                origin_pattern(&value)
                            }
                        })
                        .collect(),
                )
            });
        }
    }
    found
}
fn container_list(allow: &str, feature: &str, origin: &str, source: &str) -> List {
    for directive in allow.split(';') {
        let mut tokens = directive.split_ascii_whitespace();
        if tokens.next() != Some(feature) {
            continue;
        }
        let mut values = tokens.collect::<Vec<_>>();
        if values.is_empty() {
            values.push("'src'")
        }
        if values.contains(&"'none'") {
            return List::Origins(Vec::new());
        }
        if values.contains(&"*") {
            return List::All;
        }
        return List::Origins(
            values
                .into_iter()
                .filter_map(|value| match value {
                    "'self'" => Some(origin.into()),
                    "'src'" => {
                        if source.starts_with("about:") {
                            Some(origin.into())
                        } else {
                            valid_origin(source)
                        }
                    }
                    value => origin_pattern(value),
                })
                .collect(),
        );
    }
    default_list(feature, origin)
}
fn pattern_contains(broad: &str, narrow: &str) -> bool {
    if broad == narrow {
        return true;
    }
    if let Some((scheme, host)) = narrow.split_once("://*.") {
        let representative = format!("{scheme}://oryn-native-origin-probe.{host}");
        return broad.contains("://*.")
            && url::Url::parse(&representative).is_ok_and(|url| origin_matches(broad, &url));
    }
    url::Url::parse(narrow).is_ok_and(|url| origin_matches(broad, &url))
}
impl List {
    fn intersection(self, other: Self) -> Self {
        match (self, other) {
            (Self::All, value) | (value, Self::All) => value,
            (Self::Origins(left), Self::Origins(right)) => {
                let mut values = Vec::new();
                for a in left {
                    for b in &right {
                        let value = if pattern_contains(&a, b) {
                            Some(b.clone())
                        } else if pattern_contains(b, &a) {
                            Some(a.clone())
                        } else {
                            None
                        };
                        if let Some(value) = value {
                            if !values.contains(&value) {
                                values.push(value)
                            }
                        }
                    }
                }
                Self::Origins(values)
            }
        }
    }
    fn contains(&self, origin: &str) -> bool {
        let Ok(url) = url::Url::parse(origin) else {
            return false;
        };
        match self {
            Self::All => url.origin().ascii_serialization() != "null",
            Self::Origins(values) => values.iter().any(|value| origin_matches(value, &url)),
        }
    }
    fn strings(self) -> Vec<String> {
        match self {
            Self::All => vec!["*".into()],
            Self::Origins(values) => values,
        }
    }
}
impl Snapshot {
    pub fn validate(&self) -> bool {
        self.origin.len() <= 8192
            && self.constraints.len() <= 1024
            && self.constraints.iter().all(|constraint| match constraint {
                Constraint::Header { origin, headers }
                | Constraint::Gate {
                    origin, headers, ..
                } => {
                    origin.len() <= 8192
                        && headers.len() <= 256
                        && headers
                            .iter()
                            .all(|(name, value)| name.len() <= 256 && value.len() <= 65536)
                }
                Constraint::Container {
                    origin,
                    source,
                    allow,
                }
                | Constraint::GateContainer {
                    origin,
                    source,
                    allow,
                    ..
                } => origin.len() <= 8192 && source.len() <= 8192 && allow.len() <= 65536,
            })
    }
    fn effective(&self, feature: &str) -> List {
        if !FEATURES.contains(&feature) || !self.validate() {
            return List::Origins(Vec::new());
        }
        let own = self
            .constraints
            .first()
            .and_then(|constraint| match constraint {
                Constraint::Header { origin, headers } if origin == &self.origin => {
                    header_list(headers, feature, origin)
                }
                _ => None,
            });
        let mut list = if self.container {
            List::All
        } else {
            own.unwrap_or_else(|| default_list(feature, &self.origin))
        };
        for constraint in &self.constraints {
            match constraint {
                Constraint::Header { origin, headers } => {
                    if let Some(value) = header_list(headers, feature, origin) {
                        list = list.intersection(value)
                    }
                }
                Constraint::Container {
                    origin,
                    source,
                    allow,
                } => list = list.intersection(container_list(allow, feature, origin, source)),
                Constraint::GateContainer {
                    origin,
                    source,
                    allow,
                    target,
                } => {
                    if !(feature == "document-domain"
                        && !allow
                            .split(';')
                            .any(|part| part.split_ascii_whitespace().next() == Some(feature)))
                        && !container_allows_feature(allow, feature, origin, source, target)
                    {
                        return List::Origins(Vec::new());
                    }
                }
                Constraint::Gate {
                    origin,
                    target,
                    headers,
                } => {
                    if !allows_feature(headers, feature, origin, target) {
                        return List::Origins(Vec::new());
                    }
                }
            }
        }
        list
    }
    pub fn allows(&self, feature: &str, origin: Option<&str>) -> bool {
        let origin = origin.unwrap_or(&self.origin);
        let Some(origin) = valid_origin(origin) else {
            return false;
        };
        self.effective(feature).contains(&origin)
    }
    pub fn allowlist(&self, feature: &str) -> Vec<String> {
        self.effective(feature).strings()
    }
    pub fn iframe(&self, origin: String, source: String, allow: String) -> Self {
        let mut result = self.clone();
        result.container = true;
        result.constraints.push(Constraint::Container {
            origin: self.origin.clone(),
            source,
            allow,
        });
        result.origin = origin;
        result
    }
}
impl Snapshot {
    pub fn child(
        &self,
        origin: String,
        headers: Vec<(String, String)>,
        allow: String,
        source: String,
    ) -> Self {
        let mut constraints = vec![Constraint::Header {
            origin: origin.clone(),
            headers,
        }];
        for constraint in &self.constraints {
            match constraint {
                Constraint::Header {
                    origin: parent,
                    headers,
                } => {
                    constraints.push(Constraint::Gate {
                        origin: parent.clone(),
                        target: parent.clone(),
                        headers: headers.clone(),
                    });
                    constraints.push(Constraint::Gate {
                        origin: parent.clone(),
                        target: origin.clone(),
                        headers: headers.clone(),
                    });
                }
                constraint => constraints.push(constraint.clone()),
            }
        }
        constraints.push(Constraint::GateContainer {
            origin: self.origin.clone(),
            source,
            allow,
            target: origin.clone(),
        });
        Self {
            origin,
            container: false,
            constraints,
        }
    }
}
#[cfg(test)]
mod snapshot_tests {
    use super::*;
    fn root(policy: &str) -> Snapshot {
        Snapshot {
            origin: "https://top.test".into(),
            container: false,
            constraints: vec![Constraint::Header {
                origin: "https://top.test".into(),
                headers: vec![("permissions-policy".into(), policy.into())],
            }],
        }
    }
    #[test]
    fn inspection_uses_inherited_boolean_admission_and_current_document_allowlist() {
        let root = root("geolocation=(self \"https://child.test\")");
        let child = root.child(
            "https://child.test".into(),
            vec![("permissions-policy".into(), "geolocation=*".into())],
            "geolocation *".into(),
            "https://child.test/frame".into(),
        );
        assert!(child.allows("geolocation", Some("https://third.test")));
        let nested = child.child(
            "https://third.test".into(),
            Vec::new(),
            "geolocation *".into(),
            "https://third.test/frame".into(),
        );
        assert!(nested.allows("geolocation", None));
        assert_eq!(nested.allowlist("geolocation"), vec!["https://third.test"]);
        let denied = root.child(
            "https://other.test".into(),
            Vec::new(),
            "geolocation *".into(),
            "https://other.test".into(),
        );
        assert!(!denied.allows("geolocation", None));
        assert!(denied.allowlist("geolocation").is_empty());
    }
    #[test]
    fn iframe_allowlists_intersect_headers_defaults_and_explicit_container_rules() {
        let parent = root("picture-in-picture=(\"https://*.example.test\")");
        let child = parent.iframe(
            "https://a.example.test".into(),
            "https://a.example.test/frame".into(),
            "picture-in-picture https://a.example.test".into(),
        );
        assert_eq!(
            child.allowlist("picture-in-picture"),
            vec!["https://a.example.test"]
        );
        assert!(child.allows("picture-in-picture", None));
        assert!(!child.allows("picture-in-picture", Some("https://b.example.test")));
        assert!(!child.allows("unknown-feature", None));
        assert!(child.allowlist("unknown-feature").is_empty());
        let defaults = root("");
        assert_eq!(defaults.allowlist("picture-in-picture"), vec!["*"]);
        assert_eq!(defaults.allowlist("camera"), vec!["https://top.test"]);
    }
    #[test]
    fn malformed_origin_declarations_and_report_only_do_not_grant_policy() {
        let snapshot = root("geolocation=(\"https://child.test/not-an-origin\")");
        assert!(snapshot.allowlist("geolocation").is_empty());
        let mut snapshot = root("");
        if let Constraint::Header { headers, .. } = &mut snapshot.constraints[0] {
            headers.push((
                "permissions-policy-report-only".into(),
                "picture-in-picture=()".into(),
            ));
        }
        assert!(snapshot.allows("picture-in-picture", None));
    }
}

#[cfg(test)]
mod deferred_fetch_policy_tests {
    use super::*;
    #[test]
    fn deferred_fetch_full_self_minimal_star_and_explicit_denial() {
        assert!(FEATURES.contains(&"deferred-fetch"));
        assert!(FEATURES.contains(&"deferred-fetch-minimal"));
        assert!(!container_allows_feature(
            "",
            "deferred-fetch",
            "https://a.test",
            "https://b.test/frame",
            "https://b.test"
        ));
        assert!(container_allows_feature(
            "",
            "deferred-fetch-minimal",
            "https://a.test",
            "https://b.test/frame",
            "https://b.test"
        ));
        assert!(!container_allows_feature(
            "deferred-fetch-minimal 'none'",
            "deferred-fetch-minimal",
            "https://a.test",
            "https://b.test/frame",
            "https://b.test"
        ));
        assert!(!allows_feature(
            &[(
                "permissions-policy".into(),
                "deferred-fetch-minimal=()".into()
            )],
            "deferred-fetch-minimal",
            "https://a.test",
            "https://b.test"
        ));
    }
}
