//! HTML object resource-type detection, separate from Fetch response headers.
//! Unsupported optional plugin handlers never acquire another response owner.
use oryn_common::mime_type::MimeType;

pub(crate) fn sniff_input(kind: &str, supplied: Option<&str>) -> Option<String> {
    if kind == "object"
        && supplied
            .and_then(MimeType::parse)
            .is_some_and(|mime| mime.essence() == "text/plain")
    {
        // Object's text/binary branch applies regardless of charset parameters.
        Some("text/plain".into())
    } else {
        supplied.map(str::to_owned)
    }
}
fn supported(mime: &MimeType) -> bool {
    let value = mime.essence();
    value == "text/html"
        || value == "application/xml"
        || value == "text/xml"
        || value.ends_with("+xml")
        || oryn_dom::xml::is_text_mime(value)
        || matches!(
            value,
            "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "image/bmp"
                | "image/x-icon"
                | "image/vnd.microsoft.icon"
                | "image/avif"
        )
        || value.starts_with("audio/")
        || value.starts_with("video/")
}
pub(crate) fn select(
    kind: &str,
    supplied: Option<&str>,
    declared: Option<&str>,
    computed: &str,
    no_sniff: bool,
) -> Result<String, String> {
    let has_declared = declared.is_some();
    let supplied = supplied.and_then(MimeType::parse);
    let declared = declared.and_then(MimeType::parse);
    let selected = if kind == "embed" {
        declared
            .filter(supported)
            .or_else(|| supplied.filter(supported))
    } else if let Some(supplied) = supplied {
        let binary = supplied.essence() == "application/octet-stream"
            || (supplied.essence() == "text/plain" && computed == "application/octet-stream");
        if no_sniff || !binary {
            Some(supplied)
        } else {
            declared.filter(|mime| {
                mime.essence().starts_with("image/") && !mime.essence().ends_with("+xml")
            })
        }
    } else {
        (if has_declared {
            declared
        } else {
            MimeType::parse(computed)
        })
        .filter(|mime| mime.essence() != "application/octet-stream")
    };
    selected
        .filter(supported)
        .map(|mime| mime.serialize())
        .ok_or_else(|| "embedded resource type has no native handler".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_declared_type_is_conditional_on_response_metadata() {
        assert_eq!(
            select("object", None, Some("image/svg+xml"), "text/plain", false).unwrap(),
            "image/svg+xml"
        );
        assert_eq!(
            select(
                "object",
                Some("text/html"),
                Some("image/png"),
                "text/html",
                false
            )
            .unwrap(),
            "text/html"
        );
        assert_eq!(
            select(
                "object",
                Some("application/octet-stream"),
                Some("image/png"),
                "application/octet-stream",
                false
            )
            .unwrap(),
            "image/png"
        );
        for declared in [
            None,
            Some("image/svg+xml"),
            Some("text/html"),
            Some("application/octet-stream"),
        ] {
            assert!(
                select(
                    "object",
                    Some("application/octet-stream"),
                    declared,
                    "application/octet-stream",
                    false
                )
                .is_err()
            );
        }
        assert!(
            select(
                "object",
                Some("text/plain;charset=windows-1252"),
                Some("text/html"),
                "application/octet-stream",
                false
            )
            .is_err()
        );
        assert_eq!(
            sniff_input("object", Some("text/plain;charset=windows-1252")).as_deref(),
            Some("text/plain")
        );
        assert_eq!(
            select(
                "object",
                Some("text/plain;charset=windows-1252"),
                Some("text/html"),
                "text/plain",
                false
            )
            .unwrap(),
            "text/plain;charset=windows-1252"
        );
    }
    #[test]
    fn embed_supported_declaration_precedes_transport_type_without_url_suffix_plugins() {
        assert_eq!(
            select(
                "embed",
                Some("text/html"),
                Some("image/svg+xml"),
                "text/html",
                false
            )
            .unwrap(),
            "image/svg+xml"
        );
        assert_eq!(
            select(
                "embed",
                Some("image/png"),
                Some("application/x-unsupported"),
                "image/png",
                false
            )
            .unwrap(),
            "image/png"
        );
        assert!(select("embed", None, None, "text/html", false).is_err());
    }
}
