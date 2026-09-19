//! Bounded WebM demux. Offsets refer only to the admitted, immutable response.
//! The platform decoder never opens a URL or follows a container reference.
use serde::Serialize;
#[derive(Debug, Serialize)]
pub(super) struct Packet {
    pub offset: usize,
    pub length: usize,
    pub time: f64,
    pub key: bool,
    pub discard: f64,
}
#[derive(Debug, Serialize)]
pub(super) struct Track {
    pub number: u64,
    pub codec: String,
    pub width: u64,
    pub height: u64,
    pub channels: u64,
    pub delay: f64,
    pub private: Vec<u8>,
    pub packets: Vec<Packet>,
}
#[derive(Debug, Serialize)]
pub(super) struct Movie {
    pub duration: f64,
    pub tracks: Vec<Track>,
}
type Result<T> = std::result::Result<T, &'static str>;
const BAD: &str = "NotSupportedError: malformed or unsupported WebM";
fn vint(b: &[u8], at: &mut usize, id: bool) -> Result<(u64, bool)> {
    let first = *b.get(*at).ok_or(BAD)?;
    let n = first.leading_zeros() as usize + 1;
    if n > 8 || (id && n > 4) || b.len().saturating_sub(*at) < n {
        return Err(BAD);
    }
    let mut v = if id {
        first as u64
    } else {
        (first & (0xffu16 >> n) as u8) as u64
    };
    for x in &b[*at + 1..*at + n] {
        v = (v << 8) | *x as u64;
    }
    *at += n;
    Ok((v, !id && v == (1u64 << (7 * n)) - 1))
}
fn elements(b: &[u8], mut at: usize, end: usize) -> Result<Vec<(u64, usize, usize)>> {
    let mut out = Vec::new();
    while at < end {
        if out.len() >= 100_000 {
            return Err(BAD);
        }
        let (id, _) = vint(&b[..end], &mut at, true)?;
        let (size, unknown) = vint(&b[..end], &mut at, false)?;
        // Streaming segments/clusters can omit their final size. Walk actual
        // child headers to find a sibling; never scan compressed payload bytes
        // for apparent EBML signatures.
        let stop = if unknown && id == 0x18538067 {
            end
        } else if unknown && id == 0x1f43b675 {
            let mut next = at;
            let mut children = 0;
            while next < end {
                children += 1;
                if children > 100_000 {
                    return Err(BAD);
                }
                let begin = next;
                let (child, _) = vint(&b[..end], &mut next, true)?;
                if matches!(
                    child,
                    0x1f43b675
                        | 0x1549a966
                        | 0x1654ae6b
                        | 0x1c53bb6b
                        | 0x114d9b74
                        | 0x1254c367
                        | 0x1043a770
                        | 0x1941a469
                ) {
                    next = begin;
                    break;
                }
                let (length, unknown_length) = vint(&b[..end], &mut next, false)?;
                if unknown_length {
                    return Err(BAD);
                }
                next = next
                    .checked_add(usize::try_from(length).map_err(|_| BAD)?)
                    .filter(|x| *x <= end)
                    .ok_or(BAD)?;
            }
            next
        } else if unknown {
            return Err(BAD);
        } else {
            at.checked_add(usize::try_from(size).map_err(|_| BAD)?)
                .filter(|x| *x <= end)
                .ok_or(BAD)?
        };
        out.push((id, at, stop));
        at = stop;
    }
    Ok(out)
}
fn uint(b: &[u8]) -> Result<u64> {
    if b.is_empty() || b.len() > 8 {
        return Err(BAD);
    }
    Ok(b.iter().fold(0, |a, x| (a << 8) | *x as u64))
}
fn sint(b: &[u8]) -> Result<i64> {
    let v = uint(b)?;
    Ok(if b.len() < 8 && b[0] & 128 != 0 {
        (v | (!0u64 << (b.len() * 8))) as i64
    } else {
        v as i64
    })
}
fn float(b: &[u8]) -> Result<f64> {
    let f = match b.len() {
        4 => f32::from_be_bytes(b.try_into().unwrap()) as f64,
        8 => f64::from_be_bytes(b.try_into().unwrap()),
        _ => return Err(BAD),
    };
    if f.is_finite() && f >= 0. {
        Ok(f)
    } else {
        Err(BAD)
    }
}
fn opus_duration(b: &[u8]) -> Result<f64> {
    let toc = *b.first().ok_or(BAD)?;
    let config = toc >> 3;
    let ms = if config >= 16 {
        2.5 * (1u32 << (config & 3)) as f64
    } else if config >= 12 {
        10. * (1u32 << (config & 1)) as f64
    } else {
        [10., 20., 40., 60.][(config & 3) as usize]
    };
    let count = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => (*b.get(1).ok_or(BAD)? & 63) as u32,
    };
    let seconds = ms * count as f64 / 1000.;
    if count == 0 || seconds > 0.12 {
        Err(BAD)
    } else {
        Ok(seconds)
    }
}
fn block(
    b: &[u8],
    start: usize,
    end: usize,
    cluster: u64,
    scale: f64,
    key: bool,
    discard: f64,
    tracks: &mut [Track],
) -> Result<()> {
    let mut at = start;
    let (number, _) = vint(&b[..end], &mut at, false)?;
    if end.saturating_sub(at) < 3 {
        return Err(BAD);
    }
    let relative = i16::from_be_bytes([b[at], b[at + 1]]) as f64;
    let flags = b[at + 2];
    at += 3;
    let Some(track) = tracks.iter_mut().find(|t| t.number == number) else {
        return Ok(());
    };
    let lace = (flags >> 1) & 3;
    let count = if lace == 0 {
        1
    } else {
        let n = *b.get(at).filter(|_| at < end).ok_or(BAD)? as usize + 1;
        at += 1;
        n
    };
    let mut lengths = Vec::with_capacity(count);
    if lace == 1 {
        for _ in 1..count {
            let mut n = 0usize;
            loop {
                let x = *b.get(at).filter(|_| at < end).ok_or(BAD)?;
                at += 1;
                n = n.checked_add(x as usize).ok_or(BAD)?;
                if x != 255 {
                    break;
                }
            }
            lengths.push(n)
        }
    } else if lace == 2 {
        if (end - at) % count != 0 {
            return Err(BAD);
        }
        lengths.resize(count - 1, (end - at) / count);
    } else if lace == 3 && count > 1 {
        let (first, _) = vint(&b[..end], &mut at, false)?;
        let mut previous = i64::try_from(first).map_err(|_| BAD)?;
        lengths.push(previous as usize);
        for _ in 2..count {
            let before = at;
            let (delta, _) = vint(&b[..end], &mut at, false)?;
            let bias = (1i64 << (7 * (at - before) - 1)) - 1;
            previous = previous
                .checked_add(delta as i64 - bias)
                .filter(|n| *n >= 0)
                .ok_or(BAD)?;
            lengths.push(previous as usize)
        }
    }
    let used = lengths
        .iter()
        .try_fold(0usize, |a, x| a.checked_add(*x))
        .ok_or(BAD)?;
    lengths.push((end - at).checked_sub(used).ok_or(BAD)?);
    let mut time = (cluster as f64 + relative) * scale - track.delay;
    for (i, length) in lengths.into_iter().enumerate() {
        if length == 0 || track.packets.len() >= 100_000 {
            return Err(BAD);
        }
        let stop = at.checked_add(length).filter(|x| *x <= end).ok_or(BAD)?;
        track.packets.push(Packet {
            offset: at,
            length,
            time,
            key: key && (flags & 128 != 0 || lace == 0),
            discard: if i + 1 == count { discard } else { 0. },
        });
        if track.codec == "A_OPUS" {
            time += opus_duration(&b[at..stop])?;
        } else if count != 1 {
            return Err(BAD);
        }
        at = stop;
    }
    Ok(())
}
pub(super) fn parse(b: &[u8]) -> Result<Movie> {
    if b.len() > 16 * 1024 * 1024 || !b.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Err(BAD);
    }
    let top = elements(b, 0, b.len())?;
    let (_, hs, he) = top.iter().find(|x| x.0 == 0x1a45dfa3).copied().ok_or(BAD)?;
    if !elements(b, hs, he)?
        .iter()
        .any(|(id, s, e)| *id == 0x4282 && &b[*s..*e] == b"webm")
    {
        return Err(BAD);
    }
    let (_, s, e) = top.iter().find(|x| x.0 == 0x18538067).copied().ok_or(BAD)?;
    let entries = elements(b, s, e)?;
    let mut scale = 0.001;
    let mut duration = 0.;
    let mut tracks = Vec::new();
    for &(id, s, e) in &entries {
        if id == 0x1549a966 {
            for (id, s, e) in elements(b, s, e)? {
                match id {
                    0x2ad7b1 => {
                        let n = uint(&b[s..e])?;
                        if n == 0 {
                            return Err(BAD);
                        }
                        scale = n as f64 / 1e9
                    }
                    0x4489 => duration = float(&b[s..e])?,
                    _ => {}
                }
            }
        }
    }
    duration *= scale;
    for &(id, s, e) in &entries {
        if id != 0x1654ae6b {
            continue;
        }
        for (id, s, e) in elements(b, s, e)? {
            if id != 0xae {
                continue;
            }
            if tracks.len() >= 8 {
                return Err(BAD);
            }
            let mut t = Track {
                number: 0,
                codec: String::new(),
                width: 0,
                height: 0,
                channels: 1,
                delay: 0.,
                private: Vec::new(),
                packets: Vec::new(),
            };
            for (id, s, e) in elements(b, s, e)? {
                match id {
                    0x6d80 => return Err(BAD),
                    0xd7 => t.number = uint(&b[s..e])?,
                    0x86 => t.codec = std::str::from_utf8(&b[s..e]).map_err(|_| BAD)?.into(),
                    0x63a2 => {
                        if e - s > 4096 {
                            return Err(BAD);
                        }
                        t.private = b[s..e].to_vec()
                    }
                    0x56aa => t.delay = uint(&b[s..e])? as f64 / 1e9,
                    0xe0 => {
                        for (id, s, e) in elements(b, s, e)? {
                            match id {
                                0xb0 => t.width = uint(&b[s..e])?,
                                0xba => t.height = uint(&b[s..e])?,
                                _ => {}
                            }
                        }
                    }
                    0xe1 => {
                        for (id, s, e) in elements(b, s, e)? {
                            if id == 0x9f {
                                t.channels = uint(&b[s..e])?
                            }
                        }
                    }
                    _ => {}
                }
            }
            if t.number == 0 || tracks.iter().any(|x: &Track| x.number == t.number) {
                return Err(BAD);
            }
            match t.codec.as_str() {
                "V_VP9" | "V_VP8" => {
                    if t.width == 0
                        || t.height == 0
                        || t.width > 8192
                        || t.height > 8192
                        || t.width * t.height * 4 > 16 * 1024 * 1024
                    {
                        return Err(BAD);
                    }
                }
                "A_OPUS" => {
                    if t.channels == 0
                        || t.channels > 2
                        || t.private.len() < 19
                        || &t.private[..8] != b"OpusHead"
                        || t.private[8] > 15
                        || t.private[9] as u64 != t.channels
                        || t.private[18] != 0
                    {
                        return Err(BAD);
                    }
                    if t.delay == 0. {
                        t.delay = u16::from_le_bytes([t.private[10], t.private[11]]) as f64 / 48000.
                    }
                }
                _ => return Err(BAD),
            }
            tracks.push(t);
        }
    }
    if tracks.is_empty()
        || tracks
            .iter()
            .filter(|t| matches!(t.codec.as_str(), "V_VP9" | "V_VP8"))
            .count()
            > 1
        || tracks.iter().filter(|t| t.codec == "A_OPUS").count() > 1
    {
        return Err(BAD);
    }
    for &(id, s, e) in &entries {
        if id != 0x1f43b675 {
            continue;
        }
        let members = elements(b, s, e)?;
        let cluster = members
            .iter()
            .find(|x| x.0 == 0xe7)
            .map(|x| uint(&b[x.1..x.2]))
            .transpose()?
            .unwrap_or(0);
        for (id, s, e) in members {
            if id == 0xa3 {
                let mut p = s;
                vint(&b[..e], &mut p, false)?;
                let key = *b.get(p + 2).filter(|_| p + 2 < e).ok_or(BAD)? & 128 != 0;
                block(b, s, e, cluster, scale, key, 0., &mut tracks)?;
            } else if id == 0xa0 {
                let group = elements(b, s, e)?;
                let key = !group.iter().any(|x| x.0 == 0xfb);
                let discard = group
                    .iter()
                    .find(|x| x.0 == 0x75a2)
                    .map(|x| sint(&b[x.1..x.2]))
                    .transpose()?
                    .unwrap_or(0) as f64
                    / 1e9;
                for (id, s, e) in group {
                    if id == 0xa1 {
                        block(b, s, e, cluster, scale, key, discard, &mut tracks)?
                    }
                }
            }
        }
    }
    let declared_duration = duration > 0.;
    for t in &tracks {
        if t.packets.is_empty() || t.packets.windows(2).any(|p| p[1].time < p[0].time) {
            return Err(BAD);
        }
        for p in &t.packets {
            let end = p.time
                + if t.codec == "A_OPUS" {
                    opus_duration(&b[p.offset..p.offset + p.length])?
                } else {
                    1. / 30.
                };
            if !declared_duration {
                duration = duration.max(end - p.discard.max(0.));
            }
        }
    }
    if !duration.is_finite() || duration <= 0. || duration > 24. * 3600. {
        return Err(BAD);
    }
    Ok(Movie { duration, tracks })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lacing_signed_timestamps_and_discard_are_preserved() {
        let mut tracks = vec![Track {
            number: 1,
            codec: "A_OPUS".into(),
            width: 0,
            height: 0,
            channels: 1,
            delay: 0.0065,
            private: vec![],
            packets: vec![],
        }];
        let b = [0x81, 0xff, 0xfe, 0x04, 1, 0x80, 0x80];
        block(&b, 0, b.len(), 2, 0.001, true, 0.001, &mut tracks).unwrap();
        assert_eq!(tracks[0].packets.len(), 2);
        assert_eq!(tracks[0].packets[0].time, -0.0065);
        assert_eq!(tracks[0].packets[1].discard, 0.001);
        assert_eq!(tracks[0].packets[1].offset, 6);
    }
    #[test]
    fn malformed_lengths_and_packets_fail_closed() {
        assert!(parse(b"not webm").is_err());
        assert!(elements(&[0x81, 0x85, 0], 0, 3).is_err());
        assert!(vint(&[0], &mut 0, false).is_err());
        assert!(opus_duration(&[3, 0]).is_err());
        assert!(opus_duration(&[0x1b, 63]).is_err());
    }
    #[test]
    fn eight_byte_sizes_and_unknown_sentinels_do_not_shift_a_byte_by_eight() {
        let mut at = 0;
        assert_eq!(
            vint(&[1, 0, 0, 0, 0, 0, 1, 0], &mut at, false).unwrap(),
            (256, false)
        );
        assert_eq!(at, 8);
        let mut at = 0;
        assert_eq!(
            vint(&[1, 255, 255, 255, 255, 255, 255, 255], &mut at, false).unwrap(),
            ((1u64 << 56) - 1, true)
        );
    }
    fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 16383);
        let mut b = id.to_vec();
        b.extend_from_slice(&((body.len() as u16) | 0x4000).to_be_bytes());
        b.extend_from_slice(body);
        b
    }
    fn join(parts: Vec<Vec<u8>>) -> Vec<u8> {
        parts.concat()
    }
    fn opus_webm(unknown_cluster: bool) -> Vec<u8> {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 1, 0x38, 1, 0x80, 0xbb, 0, 0, 0, 0, 0]);
        let track = element(
            &[0xae],
            &join(vec![
                element(&[0xd7], &[1]),
                element(&[0x86], b"A_OPUS"),
                element(&[0x63, 0xa2], &head),
                element(&[0xe1], &element(&[0x9f], &[1])),
            ]),
        );
        let members = join(vec![
            element(&[0xe7], &[0]),
            element(&[0xa3], &[0x81, 0, 0, 0x80, 0x98]),
        ]);
        let cluster = if unknown_cluster {
            let mut c = vec![0x1f, 0x43, 0xb6, 0x75, 0xff];
            c.extend(members);
            c
        } else {
            element(&[0x1f, 0x43, 0xb6, 0x75], &members)
        };
        join(vec![
            element(&[0x1a, 0x45, 0xdf, 0xa3], &element(&[0x42, 0x82], b"webm")),
            element(
                &[0x18, 0x53, 0x80, 0x67],
                &join(vec![
                    element(&[0x16, 0x54, 0xae, 0x6b], &track),
                    cluster,
                    element(&[0x1c, 0x53, 0xbb, 0x6b], &[]),
                ]),
            ),
        ])
    }
    #[test]
    fn complete_container_tracks_preskip_and_unknown_cluster_boundaries() {
        for unknown in [false, true] {
            let b = opus_webm(unknown);
            let movie = parse(&b).unwrap();
            assert_eq!(movie.tracks.len(), 1);
            assert_eq!(movie.tracks[0].codec, "A_OPUS");
            assert_eq!(movie.tracks[0].delay, 0.0065);
            let p = &movie.tracks[0].packets[0];
            assert_eq!(&b[p.offset..p.offset + p.length], &[0x98]);
            assert_eq!(p.time, -0.0065);
        }
    }
    #[test]
    fn ebml_and_xiph_lacing_reject_overrun_and_preserve_packets() {
        let make = || {
            vec![Track {
                number: 1,
                codec: "A_OPUS".into(),
                width: 0,
                height: 0,
                channels: 1,
                delay: 0.,
                private: vec![],
                packets: vec![],
            }]
        };
        for block_bytes in [
            &[0x81, 0, 0, 0x02, 1, 1, 0x80, 0x80][..],
            &[0x81, 0, 0, 0x06, 1, 0x81, 0x80, 0x80][..],
        ] {
            let mut t = make();
            block(
                block_bytes,
                0,
                block_bytes.len(),
                0,
                0.001,
                true,
                0.,
                &mut t,
            )
            .unwrap();
            assert_eq!(t[0].packets.len(), 2);
            assert_eq!(t[0].packets[1].time, 0.0025);
        }
        let b = [0x81, 0, 0, 0x02, 1, 255];
        assert!(block(&b, 0, b.len(), 0, 0.001, true, 0., &mut make()).is_err());
    }
    #[test]
    fn opus_private_mapping_version_and_codec_mismatch_fail_before_decode() {
        let b = opus_webm(false);
        let head = b.windows(8).position(|x| x == b"OpusHead").unwrap();
        for (index, value) in [(8, 16), (9, 3), (18, 1)] {
            let mut bad = b.clone();
            bad[head + index] = value;
            assert!(parse(&bad).is_err());
        }
        let mut bad = b.clone();
        let at = bad.windows(6).position(|x| x == b"A_OPUS").unwrap();
        bad[at..at + 6].copy_from_slice(b"A_VORB");
        assert!(parse(&bad).is_err());
        for end in [1, 4, head + 8, b.len() - 1] {
            assert!(parse(&b[..end]).is_err());
        }
    }
    #[test]
    fn compressed_payload_signatures_cannot_end_unknown_clusters() {
        let payload = [0x81, 0, 0, 0x80, 0x98, 0x1f, 0x43, 0xb6, 0x75];
        let block = element(&[0xa3], &payload);
        let mut bytes = vec![0x1f, 0x43, 0xb6, 0x75, 0xff];
        bytes.extend_from_slice(&block);
        let boundary = bytes.len();
        bytes.extend(element(&[0x1c, 0x53, 0xbb, 0x6b], &[]));
        let entries = elements(&bytes, 0, bytes.len()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].2, boundary);
    }
    #[test]
    fn oversized_lengths_empty_laces_and_negative_ebml_deltas_are_rejected() {
        let b = [0x81, 1, 255, 255, 255, 255, 255, 255, 254];
        assert!(elements(&b, 0, b.len()).is_err());
        let make = || {
            vec![Track {
                number: 1,
                codec: "A_OPUS".into(),
                width: 0,
                height: 0,
                channels: 1,
                delay: 0.,
                private: vec![],
                packets: vec![],
            }]
        };
        for b in [
            &[0x81, 0, 0, 0x04, 255][..],
            &[0x81, 0, 0, 0x06, 2, 0x81, 0x80, 0x98, 0x98][..],
        ] {
            assert!(block(b, 0, b.len(), 0, 0.001, true, 0., &mut make()).is_err());
        }
    }
}
