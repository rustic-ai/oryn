//! Parent-side provisioning of origin-scoped secrets from an inherited file
//! descriptor. Secret values never pass through argv, environment variables,
//! OIL, MCP JSON-RPC, or worker initialization.

use std::io::Read;

use oryn_native::BrowserContext;
use serde::Deserialize;

const MAX_SECRET_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretProvision {
    origin: String,
    label: String,
    value: String,
}

#[cfg(unix)]
pub fn provision_context_from_fd(
    context: &BrowserContext,
    descriptor: std::os::fd::RawFd,
) -> anyhow::Result<usize> {
    use std::{fs::File, os::fd::FromRawFd};

    if descriptor < 3 {
        anyhow::bail!("secret descriptor must not alias stdin, stdout, or stderr");
    }
    // SAFETY: `--secret-fd` transfers ownership of an inherited descriptor to
    // this process. It is read exactly once and closed before browser work.
    let file = unsafe { File::from_raw_fd(descriptor) };
    provision_context(context, file)
}

#[cfg(not(unix))]
pub fn provision_context_from_fd(
    _context: &BrowserContext,
    _descriptor: i32,
) -> anyhow::Result<usize> {
    anyhow::bail!("inherited secret descriptors are supported on Unix platforms")
}

fn provision_context(context: &BrowserContext, reader: impl Read) -> anyhow::Result<usize> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_SECRET_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SECRET_INPUT_BYTES {
        anyhow::bail!("secret provisioning input exceeds the 1 MiB limit");
    }
    let input =
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("secret input is not UTF-8"))?;
    let records = if input.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<SecretProvision>>(&input)?
    } else {
        input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<SecretProvision>)
            .collect::<Result<Vec<_>, _>>()?
    };
    for record in &records {
        if record.origin.is_empty() || record.label.is_empty() {
            anyhow::bail!("secret origin and label must be non-empty");
        }
        context.import_secret(
            record.origin.clone(),
            record.label.clone(),
            record.value.as_bytes().to_vec(),
        );
    }
    Ok(records.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_native::Browser;

    #[test]
    fn json_lines_provision_only_parent_owned_labels() {
        let context = Browser::new().new_context();
        let count = provision_context(
            &context,
            br#"{"origin":"https://example.test","label":"password","value":"s3cret"}
"#
            .as_slice(),
        )
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(context.secret_labels(), vec!["password"]);
        assert!(
            context
                .secret_handle("https://example.test", "password")
                .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn inherited_descriptor_is_consumed_without_environment_or_argv() {
        use std::{
            io::Write,
            os::{fd::IntoRawFd, unix::net::UnixStream},
        };

        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer
            .write_all(
                br#"[{"origin":"https://fd.test","label":"token","value":"descriptor-only"}]"#,
            )
            .unwrap();
        drop(writer);
        let context = Browser::new().new_context();
        assert_eq!(
            provision_context_from_fd(&context, reader.into_raw_fd()).unwrap(),
            1
        );
        assert!(context.secret_handle("https://fd.test", "token").is_ok());
    }
}
