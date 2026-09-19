/// Private delivery provenance; historical absence cannot assert a cache hit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResponseDelivery {
    #[default]
    Unknown,
    Network,
    LocalCache,
    ValidatedCache,
}
impl ResponseDelivery {
    pub fn transfer_size(self, encoded: u64) -> u64 {
        match self {
            Self::Network => encoded.saturating_add(300),
            Self::ValidatedCache => 300,
            Self::Unknown | Self::LocalCache => 0,
        }
    }
    pub fn delivery_type(self) -> &'static str {
        if self == Self::LocalCache {
            "cache"
        } else {
            ""
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TimingHop {
    pub url: String,
    pub allow_origin: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TimingExposure {
    pub origin: String,
    pub hops: Vec<TimingHop>,
    pub allowed: bool,
}
// Private measured transport phase offsets, in microseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransportTiming {
    #[serde(default)]
    pub encoded_body_size: Option<u64>,
    #[serde(default)]
    pub measured_response: bool,
    #[serde(default)]
    pub worker_start_clock_micros: Option<u64>,
    #[serde(default)]
    pub delivery: ResponseDelivery,
    #[serde(default)]
    pub exposure: Option<TimingExposure>,
    pub dns_start: Option<u64>,
    pub dns_end: Option<u64>,
    pub connect_start: Option<u64>,
    pub connect_end: Option<u64>,
    pub secure_start: Option<u64>,
    pub secure_end: Option<u64>,
    pub request_start: u64,
    pub response_start: u64,
    #[serde(default)]
    pub response_end: Option<u64>,
    pub next_hop_protocol: String,
}
impl TransportTiming {
    pub fn valid_snapshot(&self) -> bool {
        self.next_hop_protocol.len() <= 64
            && self.dns_start.zip(self.dns_end).is_none_or(|(a, b)| a <= b)
            && self
                .connect_start
                .zip(self.connect_end)
                .is_none_or(|(a, b)| a <= b)
            && self
                .secure_start
                .zip(self.secure_end)
                .is_none_or(|(a, b)| a <= b)
            && self
                .response_end
                .is_none_or(|end| self.response_start <= end)
            && self.exposure.as_ref().is_none_or(|exposure| {
                exposure.origin.len() <= 65536
                    && exposure.hops.len() <= 21
                    && exposure.hops.iter().all(|hop| {
                        hop.url.len() <= 65536
                            && hop.allow_origin.len() <= 64
                            && hop.allow_origin.iter().all(|value| value.len() <= 65536)
                    })
                    && exposure
                        .hops
                        .iter()
                        .map(|hop| {
                            hop.url.len() + hop.allow_origin.iter().map(String::len).sum::<usize>()
                        })
                        .sum::<usize>()
                        <= 256 * 1024
            })
    }

    pub fn shifted(mut self, offset: u64) -> Self {
        for field in [
            &mut self.dns_start,
            &mut self.dns_end,
            &mut self.connect_start,
            &mut self.connect_end,
            &mut self.secure_start,
            &mut self.secure_end,
            &mut self.response_end,
        ] {
            *field = field.map(|value| value.saturating_add(offset));
        }
        self.request_start = self.request_start.saturating_add(offset);
        self.response_start = self.response_start.saturating_add(offset);
        self
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    #[test]
    fn transfer_size_uses_native_delivery_and_fixed_header_accounting() {
        assert_eq!(ResponseDelivery::Network.transfer_size(1024), 1324);
        assert_eq!(ResponseDelivery::ValidatedCache.transfer_size(1024), 300);
        assert_eq!(ResponseDelivery::LocalCache.transfer_size(1024), 0);
        assert_eq!(ResponseDelivery::Unknown.transfer_size(1024), 0);
        assert_eq!(ResponseDelivery::LocalCache.delivery_type(), "cache");
        assert_eq!(ResponseDelivery::ValidatedCache.delivery_type(), "");
        assert_eq!(ResponseDelivery::Network.transfer_size(u64::MAX), u64::MAX);
    }
}
