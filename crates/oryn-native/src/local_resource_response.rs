//! Fetch's single-range response for an already authenticated native Blob.
//! Resolution and CSP remain at the calling destination's admission boundary.
pub(crate) fn blob(
    mut bytes: Vec<u8>,
    media_type: String,
    range: Option<&str>,
) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
    let length = bytes.len();
    let mut headers = vec![("content-type".into(), media_type)];
    let Some(range) = range else {
        headers.push(("content-length".into(), length.to_string()));
        return Ok((200, headers, bytes));
    };
    let (start, end) = single_range(range, length)?;
    let part = bytes.drain(start..end).collect::<Vec<_>>();
    headers.push((
        "content-range".into(),
        format!("bytes {}-{}/{}", start, end - 1, length),
    ));
    headers.push(("content-length".into(), part.len().to_string()));
    Ok((206, headers, part))
}
fn single_range(value: &str, length: usize) -> Result<(usize, usize), String> {
    let fail = || "NetworkError: invalid or unsatisfiable Blob range".to_string();
    let value = value.trim_matches([' ', '\t']);
    let prefix = value
        .get(..6)
        .filter(|s| s.eq_ignore_ascii_case("bytes="))
        .ok_or_else(fail)?;
    let _ = prefix;
    let (start, end) = value[6..].split_once('-').ok_or_else(fail)?;
    if length == 0
        || (!start.is_empty() && !start.bytes().all(|b| b.is_ascii_digit()))
        || (!end.is_empty() && !end.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(fail());
    }
    if start.is_empty() {
        let count = end.parse::<u64>().map_err(|_| fail())?;
        if count == 0 {
            return Err(fail());
        }
        return Ok((
            length.saturating_sub(usize::try_from(count).unwrap_or(usize::MAX)),
            length,
        ));
    }
    let start = start.parse::<u64>().map_err(|_| fail())?;
    if start >= length as u64 {
        return Err(fail());
    }
    let last = if end.is_empty() {
        length as u64 - 1
    } else {
        end.parse::<u64>()
            .map_err(|_| fail())?
            .min(length as u64 - 1)
    };
    if last < start {
        return Err(fail());
    }
    Ok((start as usize, last as usize + 1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_are_sliced_from_owned_bytes_with_exact_response_metadata() {
        for (range, wanted) in [
            ("bytes=1-2", vec![1, 2]),
            ("bytes=2-", vec![2, 3]),
            ("bytes=-2", vec![2, 3]),
            ("bytes=0-99", vec![0, 1, 2, 3]),
        ] {
            let (status, headers, bytes) =
                blob(vec![0, 1, 2, 3], "audio/wav".into(), Some(range)).unwrap();
            assert_eq!(status, 206);
            assert_eq!(bytes, wanted);
            assert!(headers.contains(&("content-length".into(), wanted.len().to_string())));
        }
        assert_eq!(blob(vec![], String::new(), None).unwrap().0, 200);
    }
    #[test]
    fn invalid_multi_overflow_and_unsatisfiable_ranges_fail_closed() {
        for value in [
            "bytes=4-",
            "bytes=2-1",
            "bytes=-0",
            "bytes=0-1,2-3",
            "bytes=+1-2",
            "bytes=18446744073709551616-",
            "items=0-1",
            "bytes=-",
            "bytes= 1-2",
        ] {
            assert!(
                blob(vec![0, 1, 2, 3], String::new(), Some(value)).is_err(),
                "{value}"
            );
        }
        assert!(blob(vec![], String::new(), Some("bytes=0-")).is_err());
    }
}
