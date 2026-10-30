//! Bounded directional source windows for the native URL decoders.
//! Plan before decoding or mutating processing history; physical sink admission
//! remains a separate operation and an unaccepted segment retains its cursor.
const SEGMENTS: usize = 8;
const BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Segment {
    pub start: f64,
    pub end: f64,
    pub reverse: bool,
    pub maximum_bytes: usize,
}
#[derive(Debug, PartialEq)]
pub(super) struct Plan {
    segments: [Segment; SEGMENTS],
    len: usize,
}
impl Plan {
    pub(super) fn segments(&self) -> &[Segment] {
        &self.segments[..self.len]
    }
}
#[derive(Debug, PartialEq)]
pub(super) enum Error {
    InvalidInput,
    Capacity,
}

pub(super) fn plan(
    position: f64,
    duration: f64,
    playback_rate: f64,
    wall_seconds: f64,
    sample_rate: u32,
    channels: u32,
    looping: bool,
) -> Result<Plan, Error> {
    if !position.is_finite()
        || !duration.is_finite()
        || duration < 0.
        || !(0. ..=duration).contains(&position)
        || !playback_rate.is_finite()
        || !wall_seconds.is_finite()
        || wall_seconds < 0.
        || !(8000..=192000).contains(&sample_rate)
        || !(1..=8).contains(&channels)
    {
        return Err(Error::InvalidInput);
    }
    let mut remaining = playback_rate.abs() * wall_seconds;
    if !remaining.is_finite() {
        return Err(Error::Capacity);
    }
    let mut output = Plan {
        segments: [Segment::default(); SEGMENTS],
        len: 0,
    };
    if remaining == 0. || duration == 0. {
        return Ok(output);
    }
    let reverse = playback_rate < 0.;
    let bytes_per_frame = channels as usize * 4;
    // Two frames cover independent decoder endpoint rounding. Keep each call
    // below the existing 0.3-second decoder bound and one-MiB sink window.
    // One additional frame is slack for floating endpoint subtraction: a
    // mathematically exact boundary can round slightly above its frame count.
    let maximum_seconds = 0.25_f64.min((BYTES / bytes_per_frame - 3) as f64 / sample_rate as f64);
    let mut cursor = position;
    while remaining > 0. {
        let to_boundary = if reverse { cursor } else { duration - cursor };
        if to_boundary == 0. {
            if !looping {
                break;
            }
            cursor = if reverse { duration } else { 0. };
            continue;
        }
        if output.len == SEGMENTS {
            return Err(Error::Capacity);
        }
        let movement = remaining.min(to_boundary).min(maximum_seconds);
        let endpoint = if reverse {
            cursor - movement
        } else {
            cursor + movement
        };
        let start = cursor.min(endpoint);
        let end = cursor.max(endpoint);
        let actual = end - start;
        if actual <= 0. || actual > 0.3 {
            return Err(Error::Capacity);
        }
        let frames = (actual * sample_rate as f64).ceil() + 2.;
        if !frames.is_finite() || frames > (BYTES / bytes_per_frame) as f64 {
            return Err(Error::Capacity);
        }
        let maximum_bytes = (frames as usize)
            .checked_mul(bytes_per_frame)
            .ok_or(Error::Capacity)?;
        output.segments[output.len] = Segment {
            start,
            end,
            reverse,
            maximum_bytes,
        };
        output.len += 1;
        cursor = endpoint;
        remaining = if movement >= remaining {
            0.
        } else {
            remaining - movement
        };
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn high_rate_splits_within_existing_decoder_and_sink_bounds() {
        let value = plan(0., 10., 32., 0.05, 48000, 2, false).unwrap();
        assert_eq!(value.segments().len(), 7);
        assert!(
            value
                .segments()
                .iter()
                .all(|s| s.end - s.start <= 0.3 && s.maximum_bytes <= BYTES && !s.reverse)
        );
        assert!((value.segments().last().unwrap().end - 1.6).abs() < 1e-12);
    }
    #[test]
    fn loop_windows_split_at_boundaries_and_preserve_direction() {
        let forward = plan(9.9, 10., 4., 0.05, 48000, 2, true).unwrap();
        assert_eq!(forward.segments().len(), 2);
        assert_eq!(forward.segments()[0].end, 10.);
        assert_eq!(forward.segments()[1].start, 0.);
        let reverse = plan(0.1, 10., -4., 0.05, 48000, 2, true).unwrap();
        assert_eq!(reverse.segments().len(), 2);
        assert_eq!(reverse.segments()[0].start, 0.);
        assert_eq!(reverse.segments()[1].end, 10.);
        assert!(reverse.segments().iter().all(|s| s.reverse));
    }
    #[test]
    fn zero_and_nonlooping_endpoints_do_not_request_pcm() {
        for (position, rate) in [(0.5, 0.), (1., 1.), (0., -1.)] {
            assert!(
                plan(position, 1., rate, 0.05, 48000, 2, false)
                    .unwrap()
                    .segments()
                    .is_empty()
            );
        }
    }
    #[test]
    fn invalid_and_overbudget_windows_fail_before_any_decoder_call() {
        assert_eq!(
            plan(0., 10., f64::MAX, 2., 48000, 2, false),
            Err(Error::Capacity)
        );
        assert_eq!(
            plan(0., 10., 32., 0.05, 192000, 8, false),
            Err(Error::Capacity)
        );
        assert_eq!(
            plan(0., 1., 1., 0.05, 48000, 0, false),
            Err(Error::InvalidInput)
        );
        assert_eq!(
            plan(0., 1., f64::NAN, 0.05, 48000, 2, false),
            Err(Error::InvalidInput)
        );
    }
}
