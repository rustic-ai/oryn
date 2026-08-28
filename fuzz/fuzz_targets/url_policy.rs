#![no_main]

use libfuzzer_sys::fuzz_target;
use oryn_native::NetworkPolicy;

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = std::str::from_utf8(data)
        && let Ok(url) = url::Url::parse(value)
    {
        let policy = NetworkPolicy::default();
        let _ = policy.check_url(&url);
        if let Some(host) = url.host_str()
            && let Ok(address) = host.parse()
        {
            let _ = policy.check_ip(address);
        }
    }
});
