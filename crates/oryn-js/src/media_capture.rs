//! Native capture wire values. Device configuration is parent-only and is never
//! accepted as a page request or inferred from a permission grant.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceConfiguration {
    #[default]
    None,
    SyntheticFixtureV1,
    System,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Audio,
    Video,
}
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub kind: Kind,
    pub label: String,
    pub enabled: bool,
    pub ended: bool,
    pub settings: serde_json::Value,
    pub constraints: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stream {
    pub id: String,
    pub tracks: Vec<Track>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    Capture {
        constraints: serde_json::Value,
    },
    PollCapture {
        id: String,
    },
    CancelCapture {
        id: String,
    },
    Sample {
        id: String,
        after: u64,
    },
    AudioRender {
        id: String,
        consumer: u64,
        after: u64,
        paused: bool,
        volume: f32,
    },
    Inspect {
        id: String,
    },
    Stop {
        id: String,
    },
    Release {
        id: String,
    },
    Clone {
        id: String,
    },
    Enable {
        id: String,
        enabled: bool,
    },
    Capabilities {
        id: String,
    },
    ApplyConstraints {
        id: String,
        constraints: serde_json::Value,
    },
    UrlOpen {
        bytes_base64: String,
    },
    UrlSample {
        id: String,
        time: f64,
        paused: bool,
        volume: f32,
        seek: bool,
        playback_rate: f64,
        preserves_pitch: bool,
        looping: bool,
    },
    UrlRelease {
        id: String,
    },
    UrlCanPlay {
        mime: String,
    },
    UrlPictureInPicture {
        id: String,
        enabled: bool,
    },
    UrlPictureInPictureState {
        id: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    Pending { id: String },
    Captured(Stream),
    Track(Track),
    Capabilities(serde_json::Value),
    Sample(serde_json::Value),
    Stopped,
    UrlAsset(serde_json::Value),
    UrlFrame(serde_json::Value),
    UrlSupport(bool),
    UrlPictureInPicture(Option<(u32, u32)>),
}
#[cfg(test)]
mod rate_wire_tests {
    use super::*;
    #[test]
    fn url_samples_carry_explicit_rate_pitch_and_loop_without_legacy_defaults() {
        for rate in [-2., 0., 2.] {
            for pitch in [false, true] {
                for looping in [false, true] {
                    let request = Request::UrlSample {
                        id: "owned".into(),
                        time: 0.5,
                        paused: false,
                        volume: 0.5,
                        seek: false,
                        playback_rate: rate,
                        preserves_pitch: pitch,
                        looping,
                    };
                    let row = serde_json::to_value(request).unwrap();
                    let Request::UrlSample {
                        playback_rate,
                        preserves_pitch,
                        looping: decoded_loop,
                        ..
                    } = serde_json::from_value::<Request>(row.clone()).unwrap()
                    else {
                        panic!("wrong variant")
                    };
                    assert_eq!(
                        (playback_rate, preserves_pitch, decoded_loop),
                        (rate, pitch, looping)
                    );
                    for field in ["playback_rate", "preserves_pitch", "looping"] {
                        let mut old = row.clone();
                        old["UrlSample"].as_object_mut().unwrap().remove(field);
                        assert!(
                            serde_json::from_value::<Request>(old).is_err(),
                            "missing {field} must fail closed"
                        );
                    }
                }
            }
        }
    }
}
