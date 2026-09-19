//! Shared active mixed-content admission for native local and parent loaders.
pub(crate) fn enforce(source: &str, target: &url::Url) -> Result<(), String> {
    let trustworthy = |value: &str| {
        url::Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "https" | "wss")
                || url.host_str().is_some_and(|host| {
                    host == "localhost"
                        || host.ends_with(".localhost")
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                })
        })
    };
    if trustworthy(source)
        && matches!(target.scheme(), "http" | "ws")
        && !trustworthy(target.as_str())
    {
        return Err(format!(
            "active mixed content blocked: {source} -> {target}"
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn local_and_parent_admission_share_trustworthy_loopback_rules() {
        assert!(
            super::enforce(
                "https://secure.test",
                &url::Url::parse("http://other.test/x").unwrap()
            )
            .is_err()
        );
        assert!(
            super::enforce(
                "https://secure.test",
                &url::Url::parse("http://127.0.0.1/x").unwrap()
            )
            .is_ok()
        );
        assert!(
            super::enforce(
                "http://public.test",
                &url::Url::parse("http://other.test/x").unwrap()
            )
            .is_ok()
        );
    }
}
