//! Final-response embedding checks. Redirect policy does not become the
//! destination document's embedding policy; every actual ancestor participates.
use crate::csp::CspPolicySet;
use url::Url;

// Embedding policy applies only to responses that may create a Document.
pub(crate) fn creates_document(status: u16, headers: &[(String, String)]) -> bool {
    !oryn_net::fetch::is_redirect_status(status)
        && !matches!(status, 204 | 205)
        && !crate::download::response_is_download(headers)
}

pub(crate) fn authorize(
    url: &str,
    headers: &[(String, String)],
    ancestors: &[String],
) -> Result<(), String> {
    let policy = CspPolicySet::from_headers(url, headers);
    if !policy.authorize_frame_ancestors(ancestors).allowed {
        return Err("Content-Security-Policy frame-ancestors blocked the embedded document".into());
    }
    authorize_xfo(url, headers, ancestors, &policy)
}

pub(crate) fn authorize_xfo(
    url: &str,
    headers: &[(String, String)],
    ancestors: &[String],
    policy: &CspPolicySet,
) -> Result<(), String> {
    if policy.has_enforced_frame_ancestors() {
        return Ok(());
    }
    // HTTP get/decode/split preserves quoted strings and trims only HTTP
    // whitespace. An invalid quoted directive is not a recognized directive.
    let mut values = Vec::new();
    for (_, value) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("x-frame-options"))
    {
        let (mut start, mut quoted, mut escaped) = (0, false, false);
        for (index, ch) in value.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if quoted && ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                quoted = !quoted;
            }
            if ch == ',' && !quoted {
                values.push(
                    value[start..index]
                        .trim_matches([' ', '\t'])
                        .to_ascii_lowercase(),
                );
                start = index + 1;
            }
        }
        values.push(
            value[start..]
                .trim_matches([' ', '\t'])
                .to_ascii_lowercase(),
        );
    }
    if !values
        .iter()
        .any(|value| matches!(value.as_str(), "deny" | "sameorigin" | "allowall"))
    {
        return Ok(());
    }
    let first = &values[0];
    if values.iter().any(|value| value != first) || first == "deny" {
        return Err("X-Frame-Options denied embedding".into());
    }
    if first == "sameorigin" {
        let origin = Url::parse(url)
            .map_err(|_| "invalid embedded response URL")?
            .origin();
        if matches!(origin, url::Origin::Opaque(_))
            || !ancestors.iter().all(|ancestor| {
                Url::parse(ancestor).is_ok_and(|ancestor| ancestor.origin() == origin)
            })
        {
            return Err("X-Frame-Options SAMEORIGIN denied an ancestor".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_document_responses_do_not_enter_embedding_policy() {
        let deny = vec![("X-Frame-Options".into(), "DENY".into())];
        for status in [204, 205, 301, 302, 303, 307, 308] {
            assert!(!creates_document(status, &deny));
        }
        assert!(creates_document(200, &deny));
        assert!(creates_document(404, &deny));
        assert!(!creates_document(
            200,
            &[(
                "Content-Disposition".into(),
                "Attachment; filename=report.html".into()
            )]
        ));
        assert!(creates_document(
            200,
            &[("Content-Disposition".into(), "inline".into())]
        ));
    }
    #[test]
    fn all_ancestors_and_enforced_csp_precedence() {
        let source = "https://a.test/frame";
        let same = vec!["https://a.test/root".into(), "https://a.test/parent".into()];
        let cross = vec!["https://a.test/root".into(), "https://b.test/parent".into()];
        let fields = |value: &str| vec![("X-Frame-Options".into(), value.into())];
        assert!(authorize(source, &fields("sameOrigin"), &same).is_ok());
        assert!(authorize(source, &fields("sameOrigin"), &cross).is_err());
        for value in [
            "DENY",
            "SAMEORIGIN, INVALID",
            "SAMEORIGIN,",
            "ALLOWALL,INVALID",
            ",SAMEORIGIN,,DENY,",
        ] {
            assert!(authorize(source, &fields(value), &same).is_err(), "{value}");
        }
        for value in [
            "INVALID",
            "INVALID,",
            "\u{b}DENY",
            "\u{c}DENY",
            "\"SAMEORIGIN,DENY\"",
            "ALLOWALL",
        ] {
            assert!(authorize(source, &fields(value), &cross).is_ok(), "{value}");
        }
        let mut headers = fields("DENY");
        headers.push(("Content-Security-Policy".into(), "frame-ancestors *".into()));
        assert!(authorize(source, &headers, &cross).is_ok());
        headers[1].0 = "Content-Security-Policy-Report-Only".into();
        assert!(authorize(source, &headers, &cross).is_err());
        assert!(
            authorize(
                source,
                &[(
                    "Content-Security-Policy".into(),
                    "frame-ancestors 'self'".into()
                )],
                &cross
            )
            .is_err()
        );
    }
}
