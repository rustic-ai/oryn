//! Request admission independent of session identifiers and page origins.
use std::net::SocketAddr;

pub struct DriverAuth {
    authority: String,
    token: [u8; 64],
}

impl DriverAuth {
    pub fn new(address: SocketAddr, token: &str) -> Result<Self, &'static str> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err("driver endpoint must be explicit loopback");
        }
        if token.len() != 64
            || !token
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("invalid driver credential");
        }
        let mut bytes = [0; 64];
        bytes.copy_from_slice(token.as_bytes());
        Ok(Self {
            authority: address.to_string(),
            token: bytes,
        })
    }

    /// HTTP decoding must reject duplicate security headers before this call.
    /// No error text contains a supplied or configured credential.
    pub fn permits(
        &self,
        host: Option<&str>,
        origin: Option<&str>,
        authorization: Option<&str>,
    ) -> bool {
        if host != Some(self.authority.as_str()) || origin.is_some() {
            return false;
        }
        let Some(value) = authorization else {
            return false;
        };
        let Some((scheme, token)) = value.split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("bearer") || token.len() != 64 {
            return false;
        }
        let mut difference = 0u8;
        for (expected, actual) in self.token.iter().zip(token.bytes()) {
            difference |= *expected ^ actual;
        }
        difference == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_exact_host_private_credential_and_no_browser_origin() {
        let token = "a".repeat(64);
        let auth = DriverAuth::new("127.0.0.1:8123".parse().unwrap(), &token).unwrap();
        let bearer = format!("Bearer {token}");
        assert!(auth.permits(Some("127.0.0.1:8123"), None, Some(&bearer)));
        for host in [
            None,
            Some("localhost:8123"),
            Some("127.0.0.1:9999"),
            Some("example.test:8123"),
        ] {
            assert!(!auth.permits(host, None, Some(&bearer)));
        }
        for origin in ["null", "http://web-platform.test", "http://127.0.0.1:8123"] {
            assert!(!auth.permits(Some("127.0.0.1:8123"), Some(origin), Some(&bearer)));
        }
        for supplied in [None, Some("Bearer wrong"), Some("Basic token")] {
            assert!(!auth.permits(Some("127.0.0.1:8123"), None, supplied));
        }
        let wrong = format!("Bearer {}b", "a".repeat(63));
        assert!(!auth.permits(Some("127.0.0.1:8123"), None, Some(&wrong)));
    }
    #[test]
    fn refuses_public_listeners_and_malformed_configuration() {
        assert!(DriverAuth::new("0.0.0.0:8123".parse().unwrap(), &"a".repeat(64)).is_err());
        assert!(DriverAuth::new("127.0.0.1:0".parse().unwrap(), &"a".repeat(64)).is_err());
        assert!(DriverAuth::new("127.0.0.1:8123".parse().unwrap(), "short").is_err());
    }
}
