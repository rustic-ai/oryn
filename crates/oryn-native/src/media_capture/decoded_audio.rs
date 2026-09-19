//! Container probing and PCM decoding are independent of response MIME labels.
//! Encoded and decoded allocations have separate budgets.
use symphonia::core::{audio::SampleBuffer, errors::Error, io::MediaSourceStream, probe::Hint};
pub(super) struct Audio {
    pub rate: u32,
    pub channels: u32,
    pub samples: Vec<f32>,
}
impl Audio {
    pub fn duration(&self) -> f64 {
        self.samples.len() as f64 / (self.rate as f64 * self.channels as f64)
    }
    pub fn bytes(&self) -> usize {
        self.samples.len() * 4
    }
}
pub(super) fn decode(bytes: &[u8]) -> Result<Audio, String> {
    let source = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    let mut format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            source,
            &Default::default(),
            &Default::default(),
        )
        .map_err(|_| "NotSupportedError: unrecognized audio container")?
        .format;
    let track = format
        .default_track()
        .ok_or("NotSupportedError: audio track unavailable")?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &Default::default())
        .map_err(|_| "NotSupportedError: audio codec unavailable")?;
    let mut audio = Audio {
        rate: 0,
        channels: 0,
        samples: Vec::new(),
    };
    let mut packets = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(_) => return Err("NotReadableError: malformed audio container".into()),
        };
        packets += 1;
        if packets > 1_000_000 {
            return Err("QuotaExceededError: audio packet count".into());
        }
        if packet.track_id() != id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|_| "NotReadableError: audio decode failure")?;
        let spec = *decoded.spec();
        let channels = spec.channels.count() as u32;
        if channels == 0 || channels > 8 || !(8000..=192000).contains(&spec.rate) {
            return Err("NotSupportedError: audio format bounds".into());
        }
        if audio.rate != 0 && (audio.rate != spec.rate || audio.channels != channels) {
            return Err("NotSupportedError: discontinuous audio format".into());
        }
        audio.rate = spec.rate;
        audio.channels = channels;
        let capacity = decoded.capacity();
        if capacity > 4 * 1024 * 1024 {
            return Err("QuotaExceededError: audio packet size".into());
        }
        let mut samples = SampleBuffer::<f32>::new(capacity as u64, spec);
        samples.copy_interleaved_ref(decoded);
        if audio.samples.len().saturating_add(samples.len()) > 16 * 1024 * 1024 {
            return Err("QuotaExceededError: decoded audio bytes".into());
        }
        if samples.samples().iter().any(|sample| !sample.is_finite()) {
            return Err("NotReadableError: non-finite audio sample".into());
        }
        audio.samples.extend_from_slice(samples.samples());
    }
    if audio.samples.is_empty() {
        return Err("NotSupportedError: empty audio stream".into());
    }
    Ok(audio)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_container_does_not_create_synthetic_audio() {
        for bytes in [&b"not audio"[..], &b"OggS"[..], &b"RIFF\0\0\0\0WAVE"[..]] {
            assert!(decode(bytes).is_err());
        }
    }
}
