//! Computed document MIME types. This never changes supplied Fetch headers.
use oryn_common::mime_type::MimeType;

pub const PREFIX_LIMIT: usize = 1445;
// Canonical MIME records lose optional HTTP whitespace. Preserve the Apache
// fallback distinction: an explicit non-default charset/other parameter is
// authoritative plaintext, rather than a server's default binary label.
fn apache_plain(supplied: Option<&str>) -> bool {
    let Some(mime) = supplied.and_then(MimeType::parse) else {
        return false;
    };
    matches!(
        mime.serialize().to_ascii_lowercase().as_str(),
        "text/plain" | "text/plain;charset=iso-8859-1" | "text/plain;charset=utf-8"
    )
}
fn binary(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .any(|byte| matches!(byte, 0..=8 | 11 | 14..=26 | 28..=31))
}
fn text_or_binary(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\xef\xbb\xbf")
        || bytes.starts_with(b"\xff\xfe")
        || bytes.starts_with(b"\xfe\xff")
        || !binary(bytes)
    {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}
fn trimmed(bytes: &[u8]) -> &[u8] {
    let first = bytes
        .iter()
        .position(|byte| !matches!(byte, 9 | 10 | 12 | 13 | 32))
        .unwrap_or(bytes.len());
    &bytes[first..]
}
fn html(bytes: &[u8]) -> bool {
    let bytes = trimmed(bytes);
    if bytes.starts_with(b"<!--") {
        return true;
    }
    [
        "<!DOCTYPE HTML",
        "<HTML",
        "<HEAD",
        "<SCRIPT",
        "<IFRAME",
        "<H1",
        "<DIV",
        "<FONT",
        "<TABLE",
        "<A",
        "<STYLE",
        "<TITLE",
        "<B",
        "<BODY",
        "<BR",
        "<P",
    ]
    .iter()
    .any(|tag| {
        bytes
            .get(..tag.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(tag.as_bytes()))
            && bytes
                .get(tag.len())
                .is_some_and(|next| matches!(next, b' ' | b'>'))
    })
}
fn image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x00\x00\x01\x00") || bytes.starts_with(b"\x00\x00\x02\x00") {
        Some("image/x-icon")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.get(..4) == Some(b"RIFF") && bytes.get(8..14) == Some(b"WEBPVP") {
        Some("image/webp")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else {
        None
    }
}
fn audio_video(bytes: &[u8]) -> Option<&'static str> {
    if bytes.get(..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"AIFF") {
        Some("audio/aiff")
    } else if bytes.starts_with(b"ID3") {
        Some("audio/mpeg")
    } else if bytes.starts_with(b"OggS\x00") {
        Some("application/ogg")
    } else if bytes.starts_with(b"MThd\x00\x00\x00\x06") {
        Some("audio/midi")
    } else if bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"AVI ") {
        Some("video/avi")
    } else if bytes.get(..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        Some("audio/wave")
    } else if mp4(bytes) {
        Some("video/mp4")
    } else if webm(bytes) {
        Some("video/webm")
    } else if mp3(bytes) {
        Some("audio/mpeg")
    } else {
        None
    }
}
fn mp4(bytes: &[u8]) -> bool {
    let Some(size) = bytes
        .get(..4)
        .and_then(|value| <[u8; 4]>::try_from(value).ok())
        .map(u32::from_be_bytes)
    else {
        return false;
    };
    let size = size as usize;
    if size < 12
        || !size.is_multiple_of(4)
        || size > bytes.len()
        || bytes.get(4..8) != Some(b"ftyp")
    {
        return false;
    }
    (8..size)
        .step_by(4)
        .filter(|offset| *offset != 12)
        .any(|offset| bytes.get(offset..offset + 3) == Some(b"mp4"))
}
fn ebml_value(bytes: &[u8], offset: &mut usize, id: bool) -> Option<u64> {
    let first = *bytes.get(*offset)?;
    let length = first.leading_zeros() as usize + 1;
    if length > 8 || (id && length > 4) {
        return None;
    }
    let end = (*offset).checked_add(length)?;
    let encoded = bytes.get(*offset..end)?;
    let mut value = if id {
        u64::from(first)
    } else {
        u64::from(first & ((1u8 << (8 - length)) - 1))
    };
    for byte in &encoded[1..] {
        value = (value << 8) | u64::from(*byte);
    }
    *offset = end;
    Some(value)
}
fn webm(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"\x1a\x45\xdf\xa3") {
        return false;
    }
    let mut offset = 4;
    let Some(size) =
        ebml_value(bytes, &mut offset, false).and_then(|size| usize::try_from(size).ok())
    else {
        return false;
    };
    let Some(end) = offset.checked_add(size).filter(|end| *end <= bytes.len()) else {
        return false;
    };
    let bytes = &bytes[..end];
    while offset < end {
        let Some(id) = ebml_value(bytes, &mut offset, true) else {
            return false;
        };
        let Some(size) =
            ebml_value(bytes, &mut offset, false).and_then(|size| usize::try_from(size).ok())
        else {
            return false;
        };
        let Some(next) = offset.checked_add(size).filter(|next| *next <= end) else {
            return false;
        };
        if id == 0x4282 {
            return &bytes[offset..next] == b"webm";
        }
        offset = next;
    }
    false
}
fn mp3_frame(bytes: &[u8]) -> Option<(usize, u32, u8)> {
    let h = bytes.get(..4)?;
    if h[0] != 0xff || h[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version = (h[1] >> 3) & 3;
    let layer = (h[1] >> 1) & 3;
    let bitrate = (h[2] >> 4) as usize;
    let rate = ((h[2] >> 2) & 3) as usize;
    if version == 1 || layer == 0 || bitrate == 0 || bitrate == 15 || rate == 3 {
        return None;
    }
    const V1: [[u32; 16]; 3] = [
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
        ],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
        ],
        [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
        ],
    ];
    const V2: [u32; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    const V2L1: [u32; 16] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
    ];
    let rate = [44100, 48000, 32000][rate]
        / if version == 3 {
            1
        } else if version == 2 {
            2
        } else {
            4
        };
    let bitrate = if version == 3 {
        V1[(layer - 1) as usize][bitrate]
    } else if layer == 3 {
        V2L1[bitrate]
    } else {
        V2[bitrate]
    };
    let padding = u32::from((h[2] >> 1) & 1);
    let length = if layer == 3 {
        (12 * bitrate * 1000 / rate + padding) * 4
    } else {
        (if layer == 1 && version != 3 { 72 } else { 144 }) * bitrate * 1000 / rate + padding
    };
    Some((length as usize, rate, (version << 2) | layer))
}
fn mp3(bytes: &[u8]) -> bool {
    let Some((length, rate, kind)) = mp3_frame(bytes) else {
        return false;
    };
    bytes
        .get(length..)
        .and_then(mp3_frame)
        .is_some_and(|(_, next_rate, next_kind)| rate == next_rate && kind == next_kind)
}
/// Whether a bounded body prefix is needed before choosing a Document parser.
/// Explicit XML and HTML MIME types remain authoritative.
pub fn needs_prefix(supplied: Option<&str>, no_sniff: bool) -> bool {
    if no_sniff {
        return false;
    }
    let mime = supplied.and_then(MimeType::parse);
    mime.as_ref().is_none_or(|mime| {
        matches!(
            mime.essence(),
            "unknown/unknown" | "application/unknown" | "*/*"
        ) || apache_plain(supplied)
            || (mime.essence().starts_with("image/") && !mime.essence().ends_with("+xml"))
            || mime.essence().starts_with("audio/")
            || mime.essence().starts_with("video/")
    })
}
/// Compute only the document-consumer type; callers retain original headers for
/// Fetch, CSP, cache, resource timing and integrity provenance.
pub fn document_type(supplied: Option<&str>, no_sniff: bool, prefix: &[u8]) -> String {
    let bytes = &prefix[..prefix.len().min(PREFIX_LIMIT)];
    let mime = supplied.and_then(MimeType::parse);
    if no_sniff {
        return mime
            .map(|mime| mime.serialize())
            .unwrap_or_else(|| "application/octet-stream".into());
    }
    if let Some(mime) = mime.as_ref().filter(|mime| {
        !matches!(
            mime.essence(),
            "unknown/unknown" | "application/unknown" | "*/*"
        )
    }) {
        let essence = mime.essence();
        if essence == "text/plain" && apache_plain(supplied) {
            return if text_or_binary(bytes) == "application/octet-stream" {
                "application/octet-stream".into()
            } else {
                mime.serialize()
            };
        }
        if essence.ends_with("+xml") || matches!(essence, "text/xml" | "application/xml") {
            return mime.serialize();
        }
        if essence.starts_with("image/") {
            return image(bytes)
                .map(str::to_owned)
                .unwrap_or_else(|| mime.serialize());
        }
        if essence.starts_with("audio/") || essence.starts_with("video/") {
            return audio_video(bytes)
                .map(str::to_owned)
                .unwrap_or_else(|| mime.serialize());
        }
        return mime.serialize();
    }
    if html(bytes) {
        return "text/html".into();
    }
    if trimmed(bytes).starts_with(b"<?xml") {
        return "text/xml".into();
    }
    if bytes.starts_with(b"%PDF-") {
        return "application/pdf".into();
    }
    if bytes.starts_with(b"%!PS-Adobe-") {
        return "application/postscript".into();
    }
    if let Some(mime) = image(bytes).or_else(|| audio_video(bytes)) {
        return mime.into();
    }
    if bytes.starts_with(b"\x1f\x8b\x08") {
        return "application/x-gzip".into();
    }
    if bytes.starts_with(b"PK\x03\x04") {
        return "application/zip".into();
    }
    if bytes.starts_with(b"Rar!\x1a\x07\x00") {
        return "application/x-rar-compressed".into();
    }
    text_or_binary(bytes).into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_type_never_escalates_to_scriptable_or_image() {
        assert_eq!(
            document_type(Some("text/plain"), false, b"<html><script>bad()</script>"),
            "text/plain"
        );
        for bytes in [&b"PK\x03\x04\x00"[..], &b"\x89PNG\r\n\x1a\n"[..]] {
            assert_eq!(
                document_type(Some("text/plain"), false, bytes),
                "application/octet-stream"
            );
        }
        assert_eq!(
            document_type(Some("text/plain;charset=windows-1251"), false, b"\xc0"),
            "text/plain;charset=windows-1251"
        );
        assert_eq!(
            document_type(Some("text/plain"), false, b"\xff\xfe\x00\x00"),
            "text/plain"
        );
        assert_eq!(
            document_type(Some("text/plain;charset=windows-1251"), false, b"\x00"),
            "text/plain;charset=windows-1251"
        );
        assert_eq!(
            document_type(Some("text/plain;format=flowed"), false, b"\x00"),
            "text/plain;format=flowed"
        );
    }
    #[test]
    fn unknown_type_signatures_are_bounded_and_html_delimiters_exact() {
        assert_eq!(document_type(None, false, b"<!--comment"), "text/html");
        assert_eq!(document_type(None, false, b" \r\n<html>"), "text/html");
        assert_eq!(document_type(None, false, b"<htmlish>"), "text/plain");
        assert_eq!(document_type(None, false, b"ordinary text"), "text/plain");
        let mut bytes = vec![b'x'; PREFIX_LIMIT];
        bytes.push(0);
        assert_eq!(document_type(None, false, &bytes), "text/plain");
        assert_eq!(
            document_type(Some("text/html"), false, b"PK\x03\x04\x00"),
            "text/html"
        );
        assert_eq!(
            document_type(Some("application/atom+xml"), false, b"<html>"),
            "application/atom+xml"
        );
    }
    #[test]
    fn unknown_media_signatures_require_valid_native_headers() {
        assert_eq!(
            document_type(None, false, b"\x1a\x45\xdf\xa3\x87\x42\x82\x84webm"),
            "video/webm"
        );
        assert!(!webm(b"\x1a\x45\xdf\xa3\xff\x42\x82\x84webm"));
        let mut frames = vec![0; 421];
        frames[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
        frames[417..421].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
        assert!(mp3(&frames));
        assert_eq!(document_type(None, false, &frames), "audio/mpeg");
        frames[419] = 0xff;
        assert!(!mp3(&frames));
    }
    #[test]
    fn no_sniff_and_mp4_bounds() {
        assert_eq!(
            document_type(Some("text/plain"), true, b"\x00"),
            "text/plain"
        );
        assert_eq!(
            document_type(None, true, b"<html>"),
            "application/octet-stream"
        );
        assert!(!mp4(b"\xff\xff\xff\xffftypmp41"));
        assert!(mp4(b"\x00\x00\x00\x14ftypisom\x00\x00\x00\x00mp42"));
    }
}
