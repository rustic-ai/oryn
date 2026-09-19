//! Hash-bound, descriptor-anchored fixture configuration for parent transport.
//! Configuration establishes intended routes, not successful topology qualification.
use super::CommandError;
use oryn_native::{Browser, ContextOptions};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    net::{IpAddr, Ipv4Addr},
    path::{Component, Path, PathBuf},
};

const REVISION: &str = "719d5e38fdd0903a18ed9007aba816c98cc491e0";
const MAX_ARTIFACT_BYTES: usize = 1024 * 1024;
const MAX_HOSTS: usize = 256;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: PathBuf,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tls {
    ca: Artifact,
    certificate: Artifact,
    key: Artifact,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    protocol: String,
    port: u16,
    connect_ip: IpAddr,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    web_features: oryn_native::NativeFeatureProfile,
    third_party_cookies: oryn_native::ThirdPartyCookiePolicy,
    #[serde(default)]
    bluetooth_testing: bool,
    #[serde(default)]
    media_devices: oryn_native::MediaDeviceConfiguration,
    schema_version: u32,
    upstream_revision: String,
    certificate_bypass: bool,
    hosts: Vec<String>,
    ports: BTreeMap<String, Vec<u16>>,
    endpoint_routes: Vec<Route>,
    tls: Tls,
}

pub struct Fixture {
    web_features: oryn_native::NativeFeatureProfile,
    third_party_cookies: oryn_native::ThirdPartyCookiePolicy,
    bluetooth_testing: bool,
    media_devices: oryn_native::MediaDeviceConfiguration,
    ca: Vec<u8>,
    routes: oryn_native::network::EndpointRoutes,
    hosts: BTreeSet<String>,
    ports: BTreeSet<u16>,
    // Preserve the exact pairs when checking an existing caller's restrictions.
    endpoints: BTreeSet<(String, u16)>,
}

fn invalid(message: &str) -> CommandError {
    CommandError::invalid(message)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_valid(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read_bounded(mut file: File, limit: usize) -> Result<Vec<u8>, CommandError> {
    let metadata = file
        .metadata()
        .map_err(|_| invalid("cannot inspect fixture artifact"))?;
    if !metadata.is_file() {
        return Err(invalid("fixture artifact must be a regular file"));
    }
    if metadata.len() > limit as u64 {
        return Err(invalid("fixture artifact exceeds size limit"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid("cannot read fixture artifact"))?;
    if bytes.len() > limit {
        return Err(invalid("fixture artifact exceeds size limit"));
    }
    Ok(bytes)
}

// openat pins every directory and refuses symlinks at every relative component.
// O_NONBLOCK prevents a substituted FIFO/device from blocking before fstat.
#[cfg(unix)]
fn open_at(
    directory: &File,
    name: &std::ffi::OsStr,
    is_directory: bool,
) -> Result<File, CommandError> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let name =
        CString::new(name.as_bytes()).map_err(|_| invalid("invalid fixture path component"))?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | if is_directory { libc::O_DIRECTORY } else { 0 };
    // SAFETY: directory owns a live descriptor, name is NUL-terminated, and no
    // creation flags requiring a mode are used. Ownership transfers only on success.
    let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(invalid("cannot securely open fixture path"));
    }
    // SAFETY: successful openat returned a new descriptor owned by this call.
    let file = unsafe { File::from_raw_fd(descriptor) };
    let metadata = file
        .metadata()
        .map_err(|_| invalid("cannot inspect fixture path"))?;
    if (is_directory && !metadata.is_dir()) || (!is_directory && !metadata.is_file()) {
        return Err(invalid("fixture path has an unexpected file type"));
    }
    Ok(file)
}

#[cfg(unix)]
fn open_directory(path: &Path) -> Result<File, CommandError> {
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};
    if !path.is_absolute() {
        return Err(invalid("fixture directory must be absolute"));
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open("/")
        .map_err(|_| invalid("cannot open filesystem root"))?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => directory = open_at(&directory, name, true)?,
            _ => return Err(invalid("fixture directory is not canonical")),
        }
    }
    Ok(directory)
}

#[cfg(unix)]
fn open_artifact(root: &File, artifact: &Artifact) -> Result<File, CommandError> {
    if !hash_valid(&artifact.sha256) || artifact.path.is_absolute() {
        return Err(invalid("invalid fixture artifact reference"));
    }
    let components = artifact.path.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid(
            "fixture artifact path must contain only relative normal components",
        ));
    }
    let mut directory = root
        .try_clone()
        .map_err(|_| invalid("cannot retain fixture directory"))?;
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            unreachable!()
        };
        let file = open_at(&directory, name, index + 1 < components.len())?;
        if index + 1 == components.len() {
            return Ok(file);
        }
        directory = file;
    }
    Err(invalid("empty fixture artifact path"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressSpace {
    Loopback,
    Local,
    Public,
}

fn ipv4_space(ip: Ipv4Addr) -> Option<AddressSpace> {
    let [a, b, c, _] = ip.octets();
    if a == 127 {
        return Some(AddressSpace::Loopback);
    }
    if a == 10
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 169 && b == 254)
    {
        return Some(AddressSpace::Local);
    }
    // Do not accept documentation, benchmark, shared, multicast, reserved or
    // transition space as evidence of an owned globally routed fixture endpoint.
    if a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113)
    {
        return None;
    }
    Some(AddressSpace::Public)
}

fn address_space(ip: IpAddr) -> Option<AddressSpace> {
    match ip {
        IpAddr::V4(ip) => ipv4_space(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return ipv4_space(mapped);
            }
            if ip.is_loopback() {
                return Some(AddressSpace::Loopback);
            }
            let words = ip.segments();
            if words[0] & 0xfe00 == 0xfc00 || words[0] & 0xffc0 == 0xfe80 {
                return Some(AddressSpace::Local);
            }
            // Native fixtures use ordinary global unicast IPv6, excluding
            // special-purpose/transition/documentation allocations. Other forms
            // require a separately reviewed topology contract, never a guess.
            if words[0] & 0xe000 != 0x2000
                || (words[0] == 0x2001 && (words[1] < 0x0200 || words[1] == 0x0db8))
                || words[0] == 0x2002
                || (words[0] == 0x3fff && words[1] & 0xf000 == 0)
            {
                return None;
            }
            Some(AddressSpace::Public)
        }
    }
}

fn fixture_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.is_ascii()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label != "nonexistent"
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        && ["web-platform.test", "not-web-platform.test"]
            .iter()
            .any(|domain| {
                host == *domain
                    || host
                        .strip_suffix(*domain)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
}

impl Fixture {
    #[cfg(unix)]
    pub fn load(path: &Path, expected_sha256: &str) -> Result<Self, CommandError> {
        if !path.is_absolute() || !hash_valid(expected_sha256) {
            return Err(invalid("fixture requires absolute path and SHA256"));
        }
        // Resolve ordinary platform aliases once (e.g. macOS /var), then anchor
        // subsequent opens to the canonical directory descriptor, never path joins.
        let root_path = path
            .parent()
            .ok_or_else(|| invalid("missing fixture directory"))?
            .canonicalize()
            .map_err(|_| invalid("invalid fixture directory"))?;
        let root = open_directory(&root_path)?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("missing fixture filename"))?;
        let bytes = read_bounded(open_at(&root, name, false)?, MAX_ARTIFACT_BYTES)?;
        if digest(&bytes) != expected_sha256 {
            return Err(invalid("fixture configuration identity mismatch"));
        }
        let contract: Contract =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid fixture configuration"))?;
        // Validate references by opening regular files, but never read server key
        // bytes into the browser. Upstream independently validates server artifacts.
        drop(open_artifact(&root, &contract.tls.certificate)?);
        drop(open_artifact(&root, &contract.tls.key)?);
        let ca = read_bounded(open_artifact(&root, &contract.tls.ca)?, MAX_ARTIFACT_BYTES)?;
        if digest(&ca) != contract.tls.ca.sha256 {
            return Err(invalid("fixture CA identity mismatch"));
        }
        Self::from_contract(contract, ca)
    }

    #[cfg(not(unix))]
    pub fn load(_path: &Path, _expected_sha256: &str) -> Result<Self, CommandError> {
        Err(invalid(
            "secure fixture loading is not implemented on this platform",
        ))
    }

    fn from_contract(contract: Contract, ca: Vec<u8>) -> Result<Self, CommandError> {
        if contract.schema_version != 1
            || contract.upstream_revision != REVISION
            || contract.certificate_bypass
        {
            return Err(invalid("fixture version or certificate policy mismatch"));
        }
        let hosts = contract.hosts.iter().cloned().collect::<BTreeSet<_>>();
        if hosts.is_empty()
            || hosts.len() != contract.hosts.len()
            || hosts.len() > MAX_HOSTS
            || !hosts.contains("web-platform.test")
            || hosts.iter().any(|host| !fixture_host(host))
        {
            return Err(invalid("invalid fixture hostname set"));
        }
        let expected_counts = BTreeMap::from([
            ("http", 2),
            ("https", 2),
            ("h2", 1),
            ("ws", 1),
            ("wss", 1),
            ("http-local", 1),
            ("https-local", 1),
            ("http-public", 1),
            ("https-public", 1),
        ]);
        if contract.ports.len() != expected_counts.len() {
            return Err(invalid("incomplete fixture protocols"));
        }
        let mut ports = BTreeSet::new();
        let mut expected = BTreeSet::new();
        for (protocol, count) in expected_counts {
            let values = contract
                .ports
                .get(protocol)
                .ok_or_else(|| invalid("missing fixture protocol"))?;
            if values.len() != count {
                return Err(invalid("incorrect fixture port count"));
            }
            for port in values {
                if *port == 0 || !ports.insert(*port) {
                    return Err(invalid("ambiguous fixture port"));
                }
                expected.insert((protocol.to_string(), *port));
            }
        }
        let mut entries = Vec::new();
        let mut endpoints = BTreeSet::new();
        for route in contract.endpoint_routes {
            if !expected.remove(&(route.protocol.clone(), route.port)) {
                return Err(invalid("unexpected or duplicate fixture route"));
            }
            // Canonicalize mapped IPv4 before both classification and parent
            // authorization; IPv6 loopback predicates do not cover mapped 127/8.
            let connect_ip = match route.connect_ip {
                IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .map(IpAddr::V4)
                    .unwrap_or(IpAddr::V6(ip)),
                ip => ip,
            };
            let required_space = if route.protocol.ends_with("-local") {
                AddressSpace::Local
            } else if route.protocol.ends_with("-public") {
                AddressSpace::Public
            } else {
                AddressSpace::Loopback
            };
            if address_space(connect_ip) != Some(required_space) {
                return Err(invalid(
                    "fixture endpoint does not use its required IP address space",
                ));
            }
            let scheme = match route.protocol.as_str() {
                "http" | "http-local" | "http-public" => "http",
                "https" | "https-local" | "https-public" | "h2" => "https",
                "ws" => "ws",
                "wss" => "wss",
                _ => return Err(invalid("unknown fixture protocol")),
            };
            endpoints.insert((scheme.to_owned(), route.port));
            for host in &hosts {
                entries.push((scheme.to_owned(), host.clone(), route.port, connect_ip));
            }
        }
        if !expected.is_empty() {
            return Err(invalid("missing fixture routes"));
        }
        let routes = oryn_native::network::EndpointRoutes::new(entries)
            .map_err(|_| invalid("invalid fixture endpoint mapping"))?;
        Ok(Self {
            web_features: contract.web_features,
            ca,
            routes,
            hosts,
            ports,
            endpoints,
            third_party_cookies: contract.third_party_cookies,
            bluetooth_testing: contract.bluetooth_testing,
            media_devices: contract.media_devices,
        })
    }

    fn configure_policy(&self, options: &mut ContextOptions) -> Result<(), CommandError> {
        // Fixture authorization may narrow unrestricted defaults, but cannot
        // silently override an existing host/port/scheme denial supplied by caller.
        for host in &self.hosts {
            for (scheme, port) in &self.endpoints {
                let url = url::Url::parse(&format!("{scheme}://{host}:{port}/"))
                    .map_err(|_| invalid("invalid fixture URL"))?;
                options
                    .network_policy
                    .check_url(&url)
                    .map_err(|_| invalid("fixture conflicts with caller network policy"))?;
            }
        }
        options.network_policy.allow_loopback = true;
        options.network_policy.allow_private_networks = true;
        options.network_policy.allowed_hosts = self.hosts.clone();
        options.network_policy.allowed_ports = self.ports.clone();
        options.third_party_cookies = self.third_party_cookies;
        options.web_features = self.web_features;
        options.media_devices = self.media_devices;
        options.bluetooth_testing = self.bluetooth_testing;
        Ok(())
    }

    pub fn configure(
        self,
        browser: Browser,
        mut options: ContextOptions,
    ) -> Result<(Browser, ContextOptions), CommandError> {
        self.configure_policy(&mut options)?;
        let browser = browser
            .with_fixture_network(vec![self.ca], self.routes)
            .map_err(|_| invalid("fixture CA or native transport configuration rejected"))?;
        Ok((browser, options))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn contract_json() -> Value {
        let artifact = json!({"path":"tls/ca.pem", "sha256":digest(b"public CA")});
        let mut ports = BTreeMap::new();
        let mut routes = Vec::new();
        let mut port = 8000u16;
        for (protocol, count) in [
            ("http", 2),
            ("https", 2),
            ("h2", 1),
            ("ws", 1),
            ("wss", 1),
            ("http-local", 1),
            ("https-local", 1),
            ("http-public", 1),
            ("https-public", 1),
        ] {
            let mut values = Vec::new();
            for _ in 0..count {
                values.push(port);
                routes.push(json!({"protocol":protocol, "port":port, "connect_ip":
                    if protocol.ends_with("-local") { "10.23.0.2" }
                    else if protocol.ends_with("-public") { "20.40.50.60" } else { "127.0.0.1" }}));
                port += 1;
            }
            ports.insert(protocol, values);
        }
        json!({"web_features":{"usermedia_element":"standard"},"schema_version":1, "upstream_revision":REVISION, "certificate_bypass":false,"third_party_cookies":"block",
            "hosts":["web-platform.test", "www.web-platform.test", "not-web-platform.test"],
            "ports":ports, "endpoint_routes":routes,
            "tls":{"ca":artifact.clone(), "certificate":artifact.clone(), "key":artifact}})
    }

    fn fixture(value: Value) -> Result<Fixture, CommandError> {
        let contract: Contract =
            serde_json::from_value(value).map_err(|_| invalid("invalid test contract"))?;
        Fixture::from_contract(contract, b"public CA".to_vec())
    }
    #[test]
    fn cookie_profile_is_explicit_and_applies_to_the_whole_native_context() {
        for (policy, expected) in [
            ("block", oryn_native::ThirdPartyCookiePolicy::Block),
            ("allow", oryn_native::ThirdPartyCookiePolicy::Allow),
        ] {
            let mut value = contract_json();
            value["third_party_cookies"] = json!(policy);
            let mut options = ContextOptions::default();
            fixture(value)
                .unwrap()
                .configure_policy(&mut options)
                .unwrap();
            assert_eq!(options.third_party_cookies, expected);
        }
        for policy in [
            json!(null),
            json!(true),
            json!("auto"),
            json!({"/cookies/":"allow"}),
        ] {
            let mut value = contract_json();
            value["third_party_cookies"] = policy;
            assert!(fixture(value).is_err());
        }
        let mut value = contract_json();
        value.as_object_mut().unwrap().remove("third_party_cookies");
        assert!(fixture(value).is_err());
    }
    #[test]
    fn web_feature_profile_is_explicit_and_precedes_context_creation() {
        for (mode, expected) in [
            ("standard", oryn_native::UserMediaElementMode::Standard),
            ("legacy", oryn_native::UserMediaElementMode::Legacy),
        ] {
            let mut value = contract_json();
            value["web_features"] = json!({"usermedia_element":mode});
            let mut options = ContextOptions::default();
            fixture(value)
                .unwrap()
                .configure_policy(&mut options)
                .unwrap();
            assert_eq!(options.web_features.usermedia_element, expected);
        }
        for profile in [
            json!(null),
            json!({}),
            json!({"usermedia_element":true}),
            json!({"usermedia_element":"auto"}),
            json!({"usermedia_element":{"/legacy/":"legacy"}}),
        ] {
            let mut value = contract_json();
            value["web_features"] = profile;
            assert!(fixture(value).is_err());
        }
        let mut value = contract_json();
        value.as_object_mut().unwrap().remove("web_features");
        assert!(fixture(value).is_err());
    }
    #[test]
    fn capture_devices_require_explicit_hash_bound_fixture_configuration() {
        let mut options = ContextOptions::default();
        fixture(contract_json())
            .unwrap()
            .configure_policy(&mut options)
            .unwrap();
        assert_eq!(
            options.media_devices,
            oryn_native::MediaDeviceConfiguration::None
        );
        let mut value = contract_json();
        value["media_devices"] = json!("synthetic_fixture_v1");
        fixture(value)
            .unwrap()
            .configure_policy(&mut options)
            .unwrap();
        assert_eq!(
            options.media_devices,
            oryn_native::MediaDeviceConfiguration::SyntheticFixtureV1
        );
        for value in [json!(true), json!("camera"), json!(null)] {
            let mut contract = contract_json();
            contract["media_devices"] = value;
            assert!(fixture(contract).is_err());
        }
    }

    #[test]
    fn address_spaces_reject_reserved_and_documentation_substitutes() {
        for (values, expected) in [
            (
                vec!["127.0.0.1", "127.9.8.7", "::1", "::ffff:127.0.0.1"],
                Some(AddressSpace::Loopback),
            ),
            (
                vec![
                    "10.23.0.2",
                    "172.16.0.1",
                    "172.31.255.254",
                    "192.168.1.1",
                    "169.254.8.1",
                    "fd01::1",
                    "fe80::1",
                ],
                Some(AddressSpace::Local),
            ),
            (
                vec!["20.40.50.60", "8.8.8.8", "2603:1030:20e:3::1"],
                Some(AddressSpace::Public),
            ),
            (
                vec![
                    "0.0.0.0",
                    "100.64.0.1",
                    "100.127.255.255",
                    "192.0.0.1",
                    "192.0.2.1",
                    "198.18.0.1",
                    "198.51.100.4",
                    "203.0.113.1",
                    "224.0.0.1",
                    "255.255.255.255",
                    "::",
                    "ff02::1",
                    "2001:db8::1",
                    "2001::1",
                    "2002::1",
                    "3fff::1",
                ],
                None,
            ),
        ] {
            for value in values {
                assert_eq!(address_space(value.parse().unwrap()), expected, "{value}");
            }
        }
    }

    #[test]
    fn route_space_is_checked_for_every_protocol() {
        let valid = contract_json();
        assert!(fixture(valid.clone()).is_ok());
        for index in 0..valid["endpoint_routes"].as_array().unwrap().len() {
            let mut value = valid.clone();
            let protocol = value["endpoint_routes"][index]["protocol"]
                .as_str()
                .unwrap()
                .to_owned();
            value["endpoint_routes"][index]["connect_ip"] =
                json!(if protocol.ends_with("-public") {
                    "127.0.0.1"
                } else {
                    "20.40.50.60"
                });
            assert!(fixture(value).is_err(), "{protocol}");
        }
        for ip in [
            "192.0.2.1",
            "198.51.100.2",
            "203.0.113.3",
            "::ffff:127.0.0.1",
        ] {
            let mut value = valid.clone();
            let route = value["endpoint_routes"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|route| route["protocol"] == "http-public")
                .unwrap();
            route["connect_ip"] = json!(ip);
            assert!(fixture(value).is_err(), "{ip}");
        }
    }

    #[test]
    fn malformed_unknown_duplicate_and_missing_configuration_is_rejected() {
        for (field, replacement) in [
            ("schema_version", json!(2)),
            ("upstream_revision", json!("other")),
            ("certificate_bypass", json!(true)),
            ("unknown", json!(true)),
        ] {
            let mut value = contract_json();
            value[field] = replacement;
            assert!(fixture(value).is_err(), "{field}");
        }
        let mut missing = contract_json();
        missing["ports"]
            .as_object_mut()
            .unwrap()
            .remove("http-public");
        assert!(fixture(missing).is_err());
        let mut duplicate = contract_json();
        duplicate["ports"]["http"][1] = duplicate["ports"]["http"][0].clone();
        assert!(fixture(duplicate).is_err());
        let mut zero = contract_json();
        zero["ports"]["http"][0] = json!(0);
        assert!(fixture(zero).is_err());
        let mut missing = contract_json();
        missing["endpoint_routes"].as_array_mut().unwrap().pop();
        assert!(fixture(missing).is_err());
        let mut duplicate = contract_json();
        let first = duplicate["endpoint_routes"][0].clone();
        duplicate["endpoint_routes"]
            .as_array_mut()
            .unwrap()
            .push(first);
        assert!(fixture(duplicate).is_err());
    }

    #[test]
    fn host_names_are_canonical_and_negative_dns_names_are_not_mapped() {
        for host in [
            "outside.example",
            "web-platform.test.evil",
            ".web-platform.test",
            "x..web-platform.test",
            "UPPER.web-platform.test",
            "-x.web-platform.test",
            "x-.web-platform.test",
            "x_y.web-platform.test",
            "nonexistent.web-platform.test",
            "www.nonexistent.web-platform.test",
            "web-platform.test.",
            "web-platform.test/path",
            "user@web-platform.test",
        ] {
            let mut value = contract_json();
            value["hosts"].as_array_mut().unwrap().push(json!(host));
            assert!(fixture(value).is_err(), "{host}");
        }
        let mut duplicate = contract_json();
        duplicate["hosts"]
            .as_array_mut()
            .unwrap()
            .push(json!("web-platform.test"));
        assert!(fixture(duplicate).is_err());
        assert!(fixture_host("xn--lve-6lad.web-platform.test"));
    }

    #[test]
    fn exact_routes_preserve_url_identity_and_real_ip_policy() {
        let value = contract_json();
        let fixture = fixture(value.clone()).unwrap();
        let mut options = ContextOptions::default();
        fixture.configure_policy(&mut options).unwrap();
        for route in value["endpoint_routes"].as_array().unwrap() {
            let protocol = route["protocol"].as_str().unwrap();
            let scheme = if protocol.starts_with("https") || protocol == "h2" {
                "https"
            } else if protocol.starts_with("http") {
                "http"
            } else {
                protocol
            };
            let url = url::Url::parse(&format!(
                "{scheme}://www.web-platform.test:{}/resource?variant=1",
                route["port"]
            ))
            .unwrap();
            let before = url.clone();
            let addresses = fixture
                .routes
                .resolve(&options.network_policy, &url)
                .unwrap();
            assert_eq!(addresses.len(), 1);
            assert_eq!(
                addresses[0].ip(),
                route["connect_ip"]
                    .as_str()
                    .unwrap()
                    .parse::<IpAddr>()
                    .unwrap()
            );
            assert_eq!(url, before);
        }
        for unknown in [
            "http://nonexistent.web-platform.test:8000/",
            "http://unlisted.web-platform.test:8000/",
            "http://web-platform.test:65534/",
        ] {
            assert!(
                fixture
                    .routes
                    .resolve(&options.network_policy, &url::Url::parse(unknown).unwrap())
                    .is_err()
            );
        }
        let url = url::Url::parse("http://web-platform.test:8000/").unwrap();
        options.network_policy.allow_loopback = false;
        assert!(
            fixture
                .routes
                .resolve(&options.network_policy, &url)
                .is_err()
        );
        let denied = url::Url::parse("http://outside.example:8000/").unwrap();
        assert!(
            fixture
                .routes
                .resolve(&options.network_policy, &denied)
                .is_err()
        );
    }

    #[test]
    fn mapped_ipv4_is_canonical_before_authorization() {
        let mut value = contract_json();
        value["endpoint_routes"][0]["connect_ip"] = json!("::ffff:127.0.0.1");
        let fixture = fixture(value).unwrap();
        let mut options = ContextOptions::default();
        fixture.configure_policy(&mut options).unwrap();
        let url = url::Url::parse("http://web-platform.test:8000/").unwrap();
        assert_eq!(
            fixture
                .routes
                .resolve(&options.network_policy, &url)
                .unwrap()[0]
                .ip(),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        options.network_policy.allow_loopback = false;
        assert!(
            fixture
                .routes
                .resolve(&options.network_policy, &url)
                .is_err()
        );
    }

    #[test]
    fn existing_caller_denials_are_not_widened_and_failure_is_atomic() {
        let fixture = fixture(contract_json()).unwrap();
        for case in 0..4 {
            let mut options = ContextOptions::default();
            match case {
                0 => {
                    options
                        .network_policy
                        .allowed_hosts
                        .insert("unrelated.example".into());
                }
                1 => {
                    options.network_policy.allowed_ports.insert(443);
                }
                2 => {
                    options.network_policy.allowed_schemes = vec!["https".into()];
                }
                _ => {
                    options
                        .network_policy
                        .denied_hosts
                        .insert("web-platform.test".into());
                }
            }
            let original = options.network_policy.clone();
            assert!(fixture.configure_policy(&mut options).is_err());
            assert_eq!(options.network_policy, original);
        }
    }

    #[cfg(unix)]
    struct Directory(PathBuf);
    #[cfg(unix)]
    impl Directory {
        fn new() -> Self {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!("oryn-fixture-{}", digest(&random)));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn write_contract(&self, value: &Value) -> (PathBuf, String) {
            let bytes = serde_json::to_vec(value).unwrap();
            let path = self.0.join("fixture.json");
            std::fs::write(&path, &bytes).unwrap();
            (path, digest(&bytes))
        }
        fn prepare(&self) -> Value {
            std::fs::create_dir(self.0.join("tls")).unwrap();
            std::fs::write(self.0.join("tls/ca.pem"), b"public CA").unwrap();
            contract_json()
        }
    }
    #[cfg(unix)]
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn loader_binds_configuration_and_ca_bytes_with_size_limits() {
        let directory = Directory::new();
        let value = directory.prepare();
        let (path, expected) = directory.write_contract(&value);
        assert!(Fixture::load(&path, &expected).is_ok());
        assert!(Fixture::load(&path, &"0".repeat(64)).is_err());
        assert!(Fixture::load(&path, &expected.to_uppercase()).is_err());
        assert!(Fixture::load(Path::new("fixture.json"), &expected).is_err());
        std::fs::write(directory.0.join("tls/ca.pem"), b"changed CA").unwrap();
        assert!(Fixture::load(&path, &expected).is_err());
        std::fs::write(
            directory.0.join("tls/ca.pem"),
            vec![0; MAX_ARTIFACT_BYTES + 1],
        )
        .unwrap();
        assert!(Fixture::load(&path, &expected).is_err());
        std::fs::write(&path, vec![0; MAX_ARTIFACT_BYTES + 1]).unwrap();
        assert!(Fixture::load(&path, &expected).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn loader_rejects_traversal_final_and_intermediate_symlinks() {
        use std::os::unix::fs::symlink;
        let directory = Directory::new();
        let value = directory.prepare();
        let (path, expected) = directory.write_contract(&value);
        symlink(&path, directory.0.join("linked.json")).unwrap();
        assert!(Fixture::load(&directory.0.join("linked.json"), &expected).is_err());
        symlink(directory.0.join("tls"), directory.0.join("alias")).unwrap();
        symlink(
            directory.0.join("tls/ca.pem"),
            directory.0.join("tls/linked.pem"),
        )
        .unwrap();
        for reference in [
            "../tls/ca.pem",
            "alias/ca.pem",
            "tls/linked.pem",
            "/etc/passwd",
            "",
        ] {
            let mut value = value.clone();
            value["tls"]["ca"]["path"] = json!(reference);
            let (path, hash) = directory.write_contract(&value);
            assert!(Fixture::load(&path, &hash).is_err(), "{reference}");
        }
        for role in ["certificate", "key"] {
            let mut value = value.clone();
            value["tls"][role]["path"] = json!("tls/linked.pem");
            let (path, hash) = directory.write_contract(&value);
            assert!(Fixture::load(&path, &hash).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_artifacts_are_rejected_without_waiting_for_fifo_writers() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let directory = Directory::new();
        let mut value = directory.prepare();
        let path = directory.0.join("tls/fifo");
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a valid temporary-directory path owned by this test.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        value["tls"]["ca"]["path"] = json!("tls/fifo");
        let (path, hash) = directory.write_contract(&value);
        assert!(Fixture::load(&path, &hash).is_err());
        value["tls"]["ca"]["path"] = json!("tls");
        let (path, hash) = directory.write_contract(&value);
        assert!(Fixture::load(&path, &hash).is_err());
    }

    #[test]
    fn invalid_ca_cannot_configure_browser_transport() {
        let fixture = fixture(contract_json()).unwrap();
        assert!(
            fixture
                .configure(Browser::new(), ContextOptions::default())
                .is_err()
        );
    }
}
