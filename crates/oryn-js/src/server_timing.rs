//! Server-Timing response metadata. Parsing never evaluates authored text.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Metric {
    pub name: String,
    pub duration: f64,
    pub description: String,
}
// Equality compares the transported number representation; invalid/nonfinite
// input is rejected by native snapshot validation before exposure.
impl PartialEq for Metric {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.description == other.description
            && self.duration.to_bits() == other.duration.to_bits()
    }
}
impl Eq for Metric {}
fn token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}
fn split(value: &str, delimiter: u8) -> Vec<&str> {
    let mut result = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (index, byte) in value.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == b'"' {
            quoted = !quoted
        } else if byte == delimiter && !quoted {
            result.push(&value[start..index]);
            start = index + 1;
        }
    }
    result.push(&value[start..]);
    result
}
fn value(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Some(raw) = raw.strip_prefix('"') {
        let mut result = String::new();
        let mut escaped = false;
        for (cut, ch) in raw.char_indices() {
            if escaped {
                result.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                return raw[cut + 1..].trim().is_empty().then_some(result);
            } else {
                result.push(ch)
            }
        }
        None
    } else {
        (!raw.is_empty() && raw.bytes().all(token)).then(|| raw.to_owned())
    }
}
pub fn parse(headers: &[(String, String)]) -> Vec<Metric> {
    let mut metrics = Vec::new();
    let mut remaining = 64 * 1024usize;
    for (_, header) in headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("server-timing"))
    {
        if header.len() > remaining {
            break;
        }
        remaining -= header.len();
        for item in split(header, b',') {
            if metrics.len() == 256 {
                return metrics;
            }
            let parts = split(item, b';');
            let name = parts[0].trim();
            if name.is_empty() || !name.bytes().all(token) {
                continue;
            }
            let mut metric = Metric {
                name: name.into(),
                duration: 0.,
                description: String::new(),
            };
            let (mut duration, mut description) = (false, false);
            for part in &parts[1..] {
                let Some((key, raw)) = part.trim().split_once('=') else {
                    continue;
                };
                let Some(value) = value(raw) else { continue };
                if key.trim().eq_ignore_ascii_case("dur") && !duration {
                    duration = true;
                    metric.duration = value
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v >= 0.)
                        .unwrap_or(0.);
                } else if key.trim().eq_ignore_ascii_case("desc") && !description {
                    description = true;
                    metric.description = value;
                }
            }
            metrics.push(metric);
        }
    }
    metrics
}
pub fn valid(metrics: &[Metric]) -> bool {
    metrics.len() <= 256
        && metrics.iter().all(|m| {
            !m.name.is_empty()
                && m.name.bytes().all(token)
                && m.duration.is_finite()
                && m.duration >= 0.
                && !m.description.contains(['\r', '\n'])
        })
        && metrics
            .iter()
            .map(|m| m.name.len() + m.description.len())
            .sum::<usize>()
            <= 64 * 1024
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csp_identifiers_quotes_duplicates_and_multiple_fields() {
        let parsed = parse(&[
            (
                "Server-Timing".into(),
                r#"uuid;desc="first,with;separators", hash;desc="sha256-a\"b";dur=1.25;dur=9"#
                    .into(),
            ),
            (
                "server-timing".into(),
                "uuid2;desc=second, invalid name;dur=1, empty;dur=NaN".into(),
            ),
        ]);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].description, "first,with;separators");
        assert_eq!(parsed[1].description, "sha256-a\"b");
        assert_eq!(parsed[1].duration, 1.25);
        assert_eq!(parsed[2].name, "uuid2");
        assert_eq!(parsed[3].duration, 0.);
        assert!(valid(&parsed));
    }
}
