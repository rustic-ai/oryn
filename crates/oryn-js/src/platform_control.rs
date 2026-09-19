//! Native browser-owned permission, location and window state protocol.
//! Page bindings may query state and request protocol registration; only the
//! authenticated automation parent can change permission or location overrides.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sysex: Option<bool>,
    #[serde(
        default,
        rename = "panTiltZoom",
        skip_serializing_if = "Option::is_none"
    )]
    pub pan_tilt_zoom: Option<bool>,
    #[serde(
        default,
        rename = "allowWithoutGesture",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_without_gesture: Option<bool>,
}
impl PermissionDescriptor {
    pub fn key(&self) -> Result<String, String> {
        if !matches!(
            self.name.as_str(),
            "geolocation"
                | "camera"
                | "microphone"
                | "background-sync"
                | "persistent-storage"
                | "storage-access"
                | "top-level-storage-access"
                | "clipboard-read"
                | "clipboard-write"
                | "notifications"
                | "midi"
                | "push"
                | "display-capture"
                | "screen-wake-lock"
                | "local-fonts"
                | "window-management"
                | "accelerometer"
                | "gyroscope"
                | "magnetometer"
        ) {
            return Err("TypeError: unknown permission name".into());
        }
        let mut normalized = self.clone();
        if normalized.name != "midi" {
            normalized.sysex = None;
        } else {
            normalized.sysex = Some(normalized.sysex.unwrap_or(false));
        }
        if normalized.name != "camera" {
            normalized.pan_tilt_zoom = None;
        } else {
            normalized.pan_tilt_zoom = Some(normalized.pan_tilt_zoom.unwrap_or(false));
        }
        if !matches!(
            normalized.name.as_str(),
            "clipboard-read" | "clipboard-write"
        ) {
            normalized.allow_without_gesture = None;
        } else {
            normalized.allow_without_gesture =
                Some(normalized.allow_without_gesture.unwrap_or(false));
        }
        serde_json::to_string(&normalized).map_err(|error| error.to_string())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionState {
    Granted,
    Denied,
    Prompt,
}
impl PermissionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Prompt => "prompt",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default = "default_accuracy")]
    pub accuracy: f64,
    #[serde(default)]
    pub altitude: Option<f64>,
    #[serde(default, rename = "altitudeAccuracy")]
    pub altitude_accuracy: Option<f64>,
    #[serde(default)]
    pub heading: Option<f64>,
    #[serde(default)]
    pub speed: Option<f64>,
}
fn default_accuracy() -> f64 {
    1.0
}
impl Coordinates {
    pub fn validate(&self) -> Result<(), String> {
        if !self.latitude.is_finite()
            || self.latitude.abs() > 90.
            || !self.longitude.is_finite()
            || self.longitude.abs() > 180.
            || !self.accuracy.is_finite()
            || self.accuracy < 0.
            || self.altitude.is_some_and(|v| !v.is_finite())
            || self
                .altitude_accuracy
                .is_some_and(|v| !v.is_finite() || v < 0.)
            || self
                .heading
                .is_some_and(|v| !v.is_finite() || !(0.0..360.0).contains(&v))
            || self.speed.is_some_and(|v| !v.is_finite() || v < 0.)
        {
            return Err("invalid geolocation coordinates".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionError {
    pub code: u16,
    pub message: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationMode {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "autoAccept")]
    Accept,
    #[serde(rename = "autoReject")]
    Reject,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptKind {
    Geolocation,
    UserMedia,
    Camera,
    Microphone,
    Protocol,
}
impl PromptKind {
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Self::Protocol => &[],
            Self::Geolocation => &["geolocation"],
            Self::Camera => &["camera"],
            Self::Microphone => &["microphone"],
            Self::UserMedia => &["camera", "microphone"],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptChoice {
    Allow,
    Deny,
    Dismiss,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionPrompt {
    pub id: String,
    pub origin: String,
    pub kind: PromptKind,
    pub state: PermissionState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    StorageAccess {
        request: bool,
    },
    Storage {
        local: bool,
        operation: WindowStorageOperation,
    },
    StorageEvents {
        after: u64,
    },
    CspEvents {
        after: u64,
    },
    BluetoothDevices,
    BluetoothTestingState,
    InstallEligibility,
    UiLocaleData {
        locale: String,
    },
    UiLocale,
    PromptBegin {
        kind: PromptKind,
    },
    PromptAnswer {
        id: String,
        choice: PromptChoice,
    },
    PromptCancel {
        id: String,
    },
    Media {
        request: crate::media_capture::Request,
    },
    Permission {
        descriptor: PermissionDescriptor,
    },
    Geolocation,
    WindowState,
    RegisterProtocol {
        scheme: String,
        url: String,
    },
    UnregisterProtocol {
        scheme: String,
        url: String,
    },
    WindowPosition,
    PermissionsPolicySnapshot,
    PromptResolve {
        id: String,
        choice: PromptChoice,
        receipt: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    StorageAccess {
        allowed: bool,
    },
    Storage {
        response: serde_json::Value,
    },
    StorageEvents {
        events: Vec<StorageEventRecord>,
    },
    CspEvents {
        events: Vec<CspEvent>,
    },
    Bluetooth {
        devices: Vec<String>,
        testing_enabled: bool,
        pending_responses: usize,
        allowed: bool,
        reports: Vec<String>,
    },
    InstallEligibility {
        reason: String,
    },
    UiLocaleData {
        profile: Option<oryn_dom::ControlLocale>,
    },
    UiLocale {
        locale: String,
    },
    Prompt(PermissionPrompt),
    PromptCanceled,
    Media {
        response: crate::media_capture::Response,
    },
    Permission {
        state: PermissionState,
        revision: u64,
    },
    Geolocation {
        position: Result<Coordinates, PositionError>,
        revision: u64,
    },
    WindowState {
        hidden: bool,
        revision: u64,
    },
    WindowSuspended,
    WindowRetired,
    ProtocolRegistration {
        accepted: bool,
    },
    WindowPosition {
        x: i32,
        y: i32,
    },
    PermissionsPolicySnapshot(crate::permissions_policy::Snapshot),
}
pub trait Backend: Send + Sync {
    fn request(
        &self,
        creator: crate::worker_wire::Creator,
        request: Request,
    ) -> Result<Response, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn omitted_accuracy_uses_bidi_default_but_null_is_invalid() {
        let value: Coordinates = serde_json::from_str(r#"{"latitude":4,"longitude":2}"#).unwrap();
        assert_eq!(value.accuracy, 1.);
        assert!(
            serde_json::from_str::<Coordinates>(r#"{"latitude":4,"longitude":2,"accuracy":null}"#)
                .is_err()
        );
    }
    #[test]
    fn descriptor_defaults_do_not_split_equivalent_permissions() {
        let a = PermissionDescriptor {
            name: "camera".into(),
            sysex: None,
            pan_tilt_zoom: None,
            allow_without_gesture: None,
        };
        let mut b = a.clone();
        b.pan_tilt_zoom = Some(false);
        assert_eq!(a.key(), b.key());
        b.pan_tilt_zoom = Some(true);
        assert_ne!(a.key(), b.key());
        b.name = "invented".into();
        assert!(b.key().is_err());
    }
    #[test]
    fn coordinates_reject_nonfinite_and_out_of_range_values() {
        let mut value = Coordinates {
            latitude: 50.,
            longitude: 0.,
            accuracy: 100.,
            altitude: None,
            altitude_accuracy: None,
            heading: None,
            speed: None,
        };
        assert!(value.validate().is_ok());
        value.latitude = 91.;
        assert!(value.validate().is_err());
        value.latitude = 50.;
        value.heading = Some(360.);
        assert!(value.validate().is_err());
    }
    #[test]
    fn clipboard_descriptor_omission_equals_explicit_false() {
        for name in ["clipboard-read", "clipboard-write"] {
            let omitted = PermissionDescriptor {
                name: name.into(),
                sysex: None,
                pan_tilt_zoom: None,
                allow_without_gesture: None,
            };
            let mut explicit = omitted.clone();
            explicit.allow_without_gesture = Some(false);
            assert_eq!(omitted.key().unwrap(), explicit.key().unwrap());
            explicit.allow_without_gesture = Some(true);
            assert_ne!(omitted.key().unwrap(), explicit.key().unwrap());
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CspEvent {
    pub id: u64,
    pub document_uri: String,
    pub referrer: String,
    pub blocked_uri: String,
    pub effective_directive: String,
    pub original_policy: String,
    pub source_file: String,
    pub sample: String,
    pub disposition: String,
    pub status_code: u16,
    pub line_number: u32,
    pub column_number: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StorageEventRecord {
    pub id: u64,
    pub local: bool,
    pub key: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum WindowStorageOperation {
    Access,
    Get { key: String },
    Set { key: String, value: String },
    Remove { key: String },
    Clear,
    Entries,
}

/// Shared HTML handler scheme validation, before parsing the handler URL.
pub fn protocol_scheme(value: &str) -> Result<String, String> {
    if value.len() > 64 {
        return Err("SecurityError: protocol scheme limit".into());
    }
    let scheme = value.to_ascii_lowercase();
    let custom = scheme
        .strip_prefix("web+")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_lowercase()));
    if !custom
        && !matches!(
            scheme.as_str(),
            "bitcoin"
                | "ftp"
                | "ftps"
                | "geo"
                | "im"
                | "irc"
                | "ircs"
                | "magnet"
                | "mailto"
                | "matrix"
                | "mms"
                | "news"
                | "nntp"
                | "openpgp4fpr"
                | "sftp"
                | "sip"
                | "sms"
                | "smsto"
                | "ssh"
                | "tel"
                | "urn"
                | "webcal"
                | "wtai"
                | "xmpp"
        )
    {
        return Err("SecurityError: scheme is not registerable".into());
    }
    Ok(scheme)
}
