use std::net::IpAddr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub allow_loopback: bool,
    pub allow_private_networks: bool,
    pub allowed_schemes: Vec<String>,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            allow_loopback: false,
            allow_private_networks: false,
            allowed_schemes: vec!["http".into(), "https".into()],
        }
    }
}

impl NetworkPolicy {
    pub fn check_ip(&self, address: IpAddr) -> Result<(), PolicyDenied> {
        if address.is_loopback() && !self.allow_loopback {
            return Err(PolicyDenied::Loopback);
        }
        if is_private(address) && !self.allow_private_networks {
            return Err(PolicyDenied::PrivateNetwork);
        }
        Ok(())
    }

    pub fn check_scheme(&self, scheme: &str) -> Result<(), PolicyDenied> {
        if self.allowed_schemes.iter().any(|item| item == scheme) {
            Ok(())
        } else {
            Err(PolicyDenied::Scheme(scheme.to_string()))
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PolicyDenied {
    #[error("loopback access is denied")]
    Loopback,
    #[error("private-network access is denied")]
    PrivateNetwork,
    #[error("URL scheme is denied: {0}")]
    Scheme(String),
}

fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            address.is_private()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_multicast()
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                || octets[0] >= 240
        }
        IpAddr::V6(address) => {
            address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_unspecified()
                || address.is_multicast()
                || address.segments()[0..2] == [0x2001, 0x0db8]
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_private(IpAddr::V4(mapped)))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    fn default_policy_denies_local_targets_and_file_urls() {
        let policy = NetworkPolicy::default();
        assert_eq!(
            policy.check_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Err(PolicyDenied::Loopback)
        );
        assert_eq!(
            policy.check_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            Err(PolicyDenied::PrivateNetwork)
        );
        assert_eq!(
            policy.check_scheme("file"),
            Err(PolicyDenied::Scheme("file".into()))
        );
        assert_eq!(
            policy.check_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))),
            Err(PolicyDenied::PrivateNetwork)
        );
        assert_eq!(
            policy.check_ip(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))),
            Err(PolicyDenied::PrivateNetwork)
        );
    }
}
