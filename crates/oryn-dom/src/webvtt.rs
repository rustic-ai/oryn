//! Bounded WebVTT file parsing. Cue payload remains text until getCueAsHTML.
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub settings: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub cues: Vec<Cue>,
    pub styles: Vec<String>,
    pub regions: Vec<String>,
}
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CUES: usize = 65536;
pub fn timestamp(value: &str) -> Option<f64> {
    let mut parts = value.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let seconds = parts.pop()?;
    let (sec, millis) = seconds.split_once('.')?;
    if sec.len() != 2
        || millis.len() != 3
        || !sec
            .bytes()
            .chain(millis.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let minutes = parts.pop()?;
    if minutes.len() != 2 || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours = if let Some(hours) = parts.pop() {
        if hours.len() < 2 || !hours.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        hours.parse::<u64>().ok()?
    } else {
        0
    };
    let minutes = minutes.parse::<u64>().ok()?;
    let sec = sec.parse::<u64>().ok()?;
    if minutes > 59 || sec > 59 {
        return None;
    }
    Some(
        hours.checked_mul(3600)?.checked_add(minutes * 60 + sec)? as f64
            + millis.parse::<u64>().ok()? as f64 / 1000.,
    )
}
fn settings(text: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for token in text.split_ascii_whitespace() {
        let Some((name, value)) = token.split_once(':') else {
            continue;
        };
        let percentage = |value: &str| {
            value
                .strip_suffix('%')
                .and_then(|v| v.parse::<f64>().ok())
                .is_some_and(|v| v.is_finite() && (0.0..=100.0).contains(&v))
        };
        let valid = match name {
            "vertical" => matches!(value, "rl" | "lr"),
            "align" => matches!(value, "start" | "center" | "end" | "left" | "right"),
            "size" => percentage(value),
            "line" => {
                let mut values = value.split(',');
                let line = values.next().unwrap_or("");
                let align = values.next();
                values.next().is_none()
                    && (line == "auto" || percentage(line) || line.parse::<i64>().is_ok())
                    && align.is_none_or(|v| matches!(v, "start" | "center" | "end"))
            }
            "position" => {
                let mut values = value.split(',');
                let position = values.next().unwrap_or("");
                let align = values.next();
                values.next().is_none()
                    && percentage(position)
                    && align
                        .is_none_or(|v| matches!(v, "line-left" | "center" | "line-right" | "auto"))
            }
            "region" => !value.is_empty(),
            _ => false,
        };
        if valid {
            result.entry(name.into()).or_insert_with(|| value.into());
        }
    }
    result
}
pub fn parse(bytes: &[u8]) -> Result<Document, &'static str> {
    if bytes.len() > MAX_BYTES {
        return Err("WebVTT byte limit exceeded");
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(&text)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "\u{fffd}");
    let mut lines = text.lines();
    let header = lines.next().ok_or("missing WebVTT signature")?;
    if header != "WEBVTT"
        && !header
            .strip_prefix("WEBVTT")
            .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('\t'))
        || header.contains("-->")
    {
        return Err("invalid WebVTT signature");
    }
    let mut lines = lines.peekable();
    while lines.peek().is_some_and(|line| !line.is_empty()) {
        lines.next();
    }
    if lines.peek().is_some() {
        lines.next();
    }
    let mut document = Document {
        cues: Vec::new(),
        styles: Vec::new(),
        regions: Vec::new(),
    };
    let mut remaining = lines.collect::<Vec<_>>();
    remaining.push("");
    let mut cursor = 0;
    while cursor < remaining.len() {
        while cursor < remaining.len() && remaining[cursor].is_empty() {
            cursor += 1
        }
        if cursor == remaining.len() {
            break;
        }
        let first = remaining[cursor];
        if first == "NOTE" || first.starts_with("NOTE ") || first.starts_with("NOTE\t") {
            while cursor < remaining.len() && !remaining[cursor].is_empty() {
                cursor += 1
            }
            continue;
        }
        if document.cues.is_empty() && matches!(first, "STYLE" | "REGION") {
            let start = cursor + 1;
            cursor = start;
            while cursor < remaining.len() && !remaining[cursor].is_empty() {
                cursor += 1
            }
            if !remaining[start..cursor]
                .iter()
                .any(|line| line.contains("-->"))
            {
                let value = remaining[start..cursor].join("\n");
                if first == "STYLE" {
                    document.styles.push(value)
                } else {
                    document.regions.push(value)
                }
            }
            continue;
        }
        let id = if first.contains("-->") {
            String::new()
        } else {
            cursor += 1;
            first.to_owned()
        };
        if cursor >= remaining.len() {
            break;
        }
        let timing = remaining[cursor];
        cursor += 1;
        let parsed = timing.split_once("-->").and_then(|(start, tail)| {
            let mut tail = tail.split_ascii_whitespace();
            let start = timestamp(start.trim())?;
            let end = tail.next().and_then(timestamp)?;
            (end > start).then(|| (start, end, settings(&tail.collect::<Vec<_>>().join(" "))))
        });
        let start_text = cursor;
        // A timing line also terminates a payload when its separating empty line is
        // missing. Leave that line for the next cue instead of swallowing its timing.
        while cursor < remaining.len()
            && !remaining[cursor].is_empty()
            && !remaining[cursor].contains("-->")
        {
            cursor += 1
        }
        if let Some((start, end, settings)) = parsed {
            if document.cues.len() >= MAX_CUES {
                return Err("WebVTT cue limit exceeded");
            }
            document.cues.push(Cue {
                id,
                start,
                end,
                text: remaining[start_text..cursor].join("\n"),
                settings,
            });
        }
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_newlines_identifiers_settings_and_utf8_replacement() {
        let document=parse(b"\xef\xbb\xbfWEBVTT note\r\n\r\nNOTE ignored\r\ntext\r\n\r\nid\r\n00:01.001 --> 00:02.000 align:start align:end size:101% line:20%,center\r\nfirst\xff\r\nsecond\r\n").unwrap();
        assert_eq!(document.cues.len(), 1);
        let cue = &document.cues[0];
        assert_eq!(cue.id, "id");
        assert_eq!(cue.start, 1.001);
        assert_eq!(cue.text, "first\u{fffd}\nsecond");
        assert_eq!(cue.settings["align"], "start");
        assert!(!cue.settings.contains_key("size"));
    }
    #[test]
    fn malformed_blocks_do_not_poison_later_cues() {
        let document =
            parse(b"WEBVTT\n\nbad\n00:61.000 --> 00:62.000\nno\n\n00:00.000 --> 00:01.000\nyes")
                .unwrap();
        assert_eq!(document.cues.len(), 1);
        assert_eq!(document.cues[0].text, "yes");
        assert!(parse(b"WEBVTTbad\n\n").is_err());
        assert_eq!(timestamp("100:00:00.000"), Some(360000.));
        assert_eq!(timestamp("0:00.000"), None);
    }
    #[test]
    fn timing_lines_recover_without_blank_separator() {
        let document =
            parse(b"WEBVTT\n\n00:00.000 --> 00:01.000\nfirst\n00:01.000 --> 00:02.000\nsecond\n")
                .unwrap();
        assert_eq!(document.cues.len(), 2);
        assert_eq!(document.cues[0].text, "first");
        assert_eq!(document.cues[1].text, "second");
    }
}
