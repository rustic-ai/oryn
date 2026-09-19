//! Committed response policy for native agent-cluster admission. Report-only
//! headers never grant isolation, and duplicate malformed field values fail shut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum EmbedderPolicy {
    #[default]
    None,
    RequireCorp,
    Credentialless,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct DocumentIsolationPolicy {
    pub same_origin_opener: bool,
    pub embedder: EmbedderPolicy,
    pub same_origin_permission: bool,
}
impl DocumentIsolationPolicy {
    pub(crate) fn append_native_headers(self, headers: &mut Vec<(String, String)>) {
        if self.same_origin_opener {
            headers.push(("cross-origin-opener-policy".into(), "same-origin".into()));
        }
        match self.embedder {
            EmbedderPolicy::RequireCorp => {
                headers.push(("cross-origin-embedder-policy".into(), "require-corp".into()))
            }
            EmbedderPolicy::Credentialless => headers.push((
                "cross-origin-embedder-policy".into(),
                "credentialless".into(),
            )),
            EmbedderPolicy::None => {}
        }
        if !self.same_origin_permission {
            headers.push((
                "permissions-policy".into(),
                "cross-origin-isolated=()".into(),
            ));
        }
    }
    /// Credentialless affects cross-origin no-CORS network requests, including
    /// every redirect hop. Same-origin and CORS credentials remain unchanged.
    pub(crate) fn omit_credentials(self, cors: bool, cross_origin: bool) -> bool {
        !cors && cross_origin && self.embedder == EmbedderPolicy::Credentialless
    }
    /// Evaluate raw response headers before filtering or exposing body bytes.
    /// A CORS response reaches this method only after native CORS validation.
    pub(crate) fn authorize_response(
        self,
        origin: &str,
        opaque: bool,
        target: &url::Url,
        cors: bool,
        credentials: bool,
        headers: &[(String, String)],
    ) -> Result<(), String> {
        if cors {
            return Ok(());
        }
        let same_origin = !opaque && target.origin().ascii_serialization() == origin;
        if same_origin {
            return Ok(());
        }
        let mut fields = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cross-origin-resource-policy"));
        let first = fields.next().map(|(_, value)| value.trim());
        // Multiple/invalid CORP values are not an affirmative policy grant.
        let corp = if fields.next().is_none() { first } else { None };
        match corp {
            Some("same-origin") => {
                return Err("NetworkError: resource CORP requires same origin".into());
            }
            Some("same-site") => {
                let same_site = !opaque
                    && url::Url::parse(origin).ok().is_some_and(|source| {
                        if source.scheme() == "https" && target.scheme() != "https" {
                            return false;
                        }
                        if source.host_str() == target.host_str() {
                            return true;
                        }
                        crate::cookies::SchemefulSite::from_url(&source)
                            .zip(crate::cookies::SchemefulSite::from_url(target))
                            .is_some_and(|(source, target)| {
                                source.registrable_domain == target.registrable_domain
                            })
                    });
                return if same_site {
                    Ok(())
                } else {
                    Err("NetworkError: resource CORP requires same site".into())
                };
            }
            Some("cross-origin") => return Ok(()),
            _ => {}
        }
        if self.embedder == EmbedderPolicy::RequireCorp
            || (self.embedder == EmbedderPolicy::Credentialless && credentials)
        {
            return Err("NetworkError: embedder policy requires affirmative CORP".into());
        }
        Ok(())
    }
    pub(crate) fn authorize_frame_response(
        self,
        headers: &[(String, String)],
    ) -> Result<(), String> {
        if self.embedder != EmbedderPolicy::None
            && Self::from_headers(headers).embedder == EmbedderPolicy::None
        {
            return Err(
                "Cross-Origin-Embedder-Policy requires the embedded document to opt in".into(),
            );
        }
        Ok(())
    }
    pub(crate) fn from_headers(headers: &[(String, String)]) -> Self {
        let item = |name: &str| {
            let mut values = headers
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(name));
            let value = values.next()?.1.as_str();
            if values.next().is_some() {
                return None;
            }
            structured_token(value)
        };
        Self {
            same_origin_permission: !headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
                .flat_map(|(_, value)| value.split(','))
                .any(|directive| {
                    directive
                        .trim()
                        .strip_prefix("cross-origin-isolated")
                        .is_some_and(|value| {
                            value
                                .trim_start()
                                .strip_prefix('=')
                                .is_some_and(|value| value.trim() == "()")
                        })
                }),
            same_origin_opener: item("cross-origin-opener-policy") == Some("same-origin"),
            embedder: match item("cross-origin-embedder-policy") {
                Some("require-corp") => EmbedderPolicy::RequireCorp,
                Some("credentialless") => EmbedderPolicy::Credentialless,
                _ => EmbedderPolicy::None,
            },
        }
    }
    pub(crate) fn top_level_isolated(self, secure_context: bool, permission: bool) -> bool {
        secure_context
            && permission
            && self.same_origin_permission
            && self.same_origin_opener
            && self.embedder != EmbedderPolicy::None
    }
    pub(crate) fn embedded_isolated(
        self,
        secure_context: bool,
        parent_isolated: bool,
        permission: bool,
    ) -> bool {
        secure_context
            && parent_isolated
            && permission
            && self.same_origin_permission
            && self.embedder != EmbedderPolicy::None
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;
    fn policy(embedder: EmbedderPolicy) -> DocumentIsolationPolicy {
        DocumentIsolationPolicy {
            embedder,
            same_origin_opener: true,
            same_origin_permission: true,
        }
    }
    #[test]
    fn embedded_document_opt_in_is_required_and_report_only_does_not_grant_it() {
        for embedder in [EmbedderPolicy::RequireCorp, EmbedderPolicy::Credentialless] {
            let parent = policy(embedder);
            assert!(parent.authorize_frame_response(&[]).is_err());
            assert!(
                parent
                    .authorize_frame_response(&[(
                        "Cross-Origin-Embedder-Policy-Report-Only".into(),
                        "require-corp".into()
                    )])
                    .is_err()
            );
            for value in ["require-corp", "credentialless"] {
                assert!(
                    parent
                        .authorize_frame_response(&[(
                            "Cross-Origin-Embedder-Policy".into(),
                            value.into()
                        )])
                        .is_ok()
                );
            }
        }
        assert!(
            policy(EmbedderPolicy::None)
                .authorize_frame_response(&[])
                .is_ok()
        );
    }
    #[test]
    fn corp_enforces_native_origin_site_and_credentialless_without_disabling_cors() {
        let target = url::Url::parse("https://static.example.com/asset").unwrap();
        let headers = |value: &str| vec![("cross-origin-resource-policy".into(), value.into())];
        let require = policy(EmbedderPolicy::RequireCorp);
        assert!(
            require
                .authorize_response("https://app.example.com", false, &target, false, true, &[])
                .is_err()
        );
        assert!(
            require
                .authorize_response(
                    "https://app.example.com",
                    false,
                    &target,
                    false,
                    true,
                    &headers("same-site")
                )
                .is_ok()
        );
        assert!(
            require
                .authorize_response(
                    "https://attacker.example.org",
                    false,
                    &target,
                    false,
                    false,
                    &headers("same-site")
                )
                .is_err()
        );
        assert!(
            require
                .authorize_response("null", true, &target, false, false, &headers("same-site"))
                .is_err()
        );
        assert!(
            require
                .authorize_response(
                    "null",
                    true,
                    &target,
                    false,
                    false,
                    &headers("cross-origin")
                )
                .is_ok()
        );
        assert!(
            require
                .authorize_response("https://app.example.com", false, &target, true, true, &[])
                .is_ok()
        );
        let credentialless = policy(EmbedderPolicy::Credentialless);
        assert!(credentialless.omit_credentials(false, true));
        assert!(!credentialless.omit_credentials(true, true));
        assert!(!credentialless.omit_credentials(false, false));
        assert!(
            credentialless
                .authorize_response("https://app.example.com", false, &target, false, false, &[])
                .is_ok()
        );
        assert!(
            credentialless
                .authorize_response(
                    "https://app.example.com",
                    false,
                    &target,
                    false,
                    false,
                    &headers("same-origin")
                )
                .is_err()
        );
        assert!(
            credentialless
                .authorize_response("https://app.example.com", false, &target, false, true, &[])
                .is_err()
        );
    }
    #[test]
    fn duplicate_corp_does_not_grant_isolation_and_secure_source_rejects_site_downgrade() {
        let require = policy(EmbedderPolicy::RequireCorp);
        let duplicate = vec![
            ("cross-origin-resource-policy".into(), "cross-origin".into()),
            ("Cross-Origin-Resource-Policy".into(), "cross-origin".into()),
        ];
        let target = url::Url::parse("https://static.example.com/x").unwrap();
        assert!(
            require
                .authorize_response(
                    "https://app.example.com",
                    false,
                    &target,
                    false,
                    false,
                    &duplicate
                )
                .is_err()
        );
        let headers = vec![("cross-origin-resource-policy".into(), "same-site".into())];
        let insecure = url::Url::parse("http://static.example.com/x").unwrap();
        assert!(
            require
                .authorize_response(
                    "https://app.example.com",
                    false,
                    &insecure,
                    false,
                    false,
                    &headers
                )
                .is_err()
        );
        assert!(
            require
                .authorize_response(
                    "http://app.example.com",
                    false,
                    &target,
                    false,
                    false,
                    &headers
                )
                .is_ok()
        );
    }
}
// RFC 8941 token item followed by parameters. The policy algorithms use only
// the item token, but malformed parameters invalidate the entire field.
pub(crate) fn structured_token(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    let mut at = 0;
    while bytes.get(at) == Some(&b' ') {
        at += 1
    }
    let start = at;
    if !bytes
        .get(at)
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'*')
    {
        return None;
    }
    at += 1;
    while bytes
        .get(at)
        .is_some_and(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~:/".contains(b))
    {
        at += 1
    }
    let token = &value[start..at];
    loop {
        if at == bytes.len() {
            return Some(token);
        }
        if bytes[at] == b' ' {
            while bytes.get(at) == Some(&b' ') {
                at += 1
            }
            return (at == bytes.len()).then_some(token);
        }
        if bytes[at] != b';' {
            return None;
        }
        at += 1;
        while bytes.get(at) == Some(&b' ') {
            at += 1
        }
        if !bytes
            .get(at)
            .is_some_and(|b| b.is_ascii_lowercase() || *b == b'*')
        {
            return None;
        }
        at += 1;
        while bytes
            .get(at)
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.*-".contains(b))
        {
            at += 1
        }
        if bytes.get(at) != Some(&b'=') {
            continue;
        }
        at += 1;
        match *bytes.get(at)? {
            b'"' => {
                at += 1;
                loop {
                    let byte = *bytes.get(at)?;
                    at += 1;
                    if byte == b'"' {
                        break;
                    }
                    if byte == b'\\' {
                        let escaped = *bytes.get(at)?;
                        at += 1;
                        if escaped != b'\\' && escaped != b'"' {
                            return None;
                        }
                    } else if !(0x20..=0x7e).contains(&byte) {
                        return None;
                    }
                }
            }
            b'?' => {
                at += 1;
                if !matches!(bytes.get(at), Some(b'0' | b'1')) {
                    return None;
                }
                at += 1;
            }
            b'-' | b'0'..=b'9' => {
                let number = at;
                if bytes[at] == b'-' {
                    at += 1
                }
                let digits = at;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1
                }
                let integer = at - digits;
                if integer == 0 {
                    return None;
                }
                if bytes.get(at) == Some(&b'.') {
                    if integer > 12 {
                        return None;
                    }
                    at += 1;
                    let fraction = at;
                    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                        at += 1
                    }
                    if !(1..=3).contains(&(at - fraction)) {
                        return None;
                    }
                } else if integer > 15 {
                    return None;
                }
                if at == number {
                    return None;
                }
            }
            b':' => {
                at += 1;
                let start = at;
                while bytes
                    .get(at)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b"+/=".contains(b))
                {
                    at += 1
                }
                if bytes.get(at) != Some(&b':') {
                    return None;
                }
                use base64::Engine;
                if base64::engine::general_purpose::STANDARD
                    .decode(&bytes[start..at])
                    .is_err()
                {
                    return None;
                }
                at += 1;
            }
            byte if byte.is_ascii_alphabetic() || byte == b'*' => {
                at += 1;
                while bytes
                    .get(at)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~:/".contains(b))
                {
                    at += 1
                }
            }
            _ => return None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolation_requires_enforcing_policy_and_native_secure_ancestry() {
        let fields = vec![
            (
                "Cross-Origin-Opener-Policy".into(),
                "same-origin; report-to=\"group,one\"".into(),
            ),
            ("Cross-Origin-Embedder-Policy".into(), "require-corp".into()),
        ];
        let policy = DocumentIsolationPolicy::from_headers(&fields);
        assert!(policy.top_level_isolated(true, true));
        assert!(!policy.top_level_isolated(false, true));
        assert!(!policy.embedded_isolated(true, false, true));
        assert!(!policy.top_level_isolated(true, false));
        let mut duplicate = fields.clone();
        duplicate.push(fields[1].clone());
        assert!(!DocumentIsolationPolicy::from_headers(&duplicate).top_level_isolated(true, true));
        let report = fields
            .iter()
            .map(|(key, value)| (format!("{key}-Report-Only"), value.clone()))
            .collect::<Vec<_>>();
        assert!(!DocumentIsolationPolicy::from_headers(&report).top_level_isolated(true, true));
        for malformed in [
            "require-corp, unsafe-none",
            "require-corp;Bad=1",
            "require-corp;report-to=\"bad\\q\"",
            "require-corp\r\n",
            "require-corp;x=?2",
        ] {
            assert!(structured_token(malformed).is_none());
        }
    }
}

/// Effective capability derived only from the parent-admitted response policy.
/// Shared worker identity reuse must not silently retain a different capability.
pub(crate) fn worker_isolated_capability(bootstrap: &oryn_js::worker_wire::AgentBootstrap) -> bool {
    let policy = DocumentIsolationPolicy::from_headers(&bootstrap.policy.headers);
    !bootstrap.origin.opaque
        && bootstrap.secure_context
        && policy.embedded_isolated(true, true, policy.same_origin_permission)
        && crate::isolation_permissions::allows(
            &bootstrap.policy.headers,
            &bootstrap.origin.serialized,
            &bootstrap.origin.serialized,
        )
}
