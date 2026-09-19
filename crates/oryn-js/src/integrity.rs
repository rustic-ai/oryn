//! Subresource Integrity over fetched representation bytes, before character
//! decoding. URL resolution never substitutes for response verification.
use base64::Engine;
use sha2::{Digest, Sha256, Sha384, Sha512};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrityError {
    IneligibleResponse,
    DigestMismatch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Algorithm {
    Sha256,
    Sha384,
    Sha512,
}
fn token(value: &str) -> Option<(Algorithm, Option<Vec<u8>>)> {
    let (algorithm, value) = value.split_once('-')?;
    let algorithm = match algorithm {
        "sha256" => Algorithm::Sha256,
        "sha384" => Algorithm::Sha384,
        "sha512" => Algorithm::Sha512,
        _ => return None,
    };
    let digest = value.split_once('?').map_or(value, |(digest, _)| digest);
    // Metadata grammar and digest decoding are separate: a supported hash
    // with base64-value syntax must still fail verification when its bytes
    // cannot be decoded. Dropping it would disable SRI or select a weaker hash.
    let value = digest.trim_end_matches('=');
    if value.is_empty()
        || digest.len() - value.len() > 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'-' | b'_'))
    {
        return None;
    }
    let decoded = [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ]
    .iter()
    .find_map(|engine| engine.decode(digest).ok());
    Some((algorithm, decoded))
}
/// `eligible` is derived from the native response/CORS policy, never an author
/// flag. All supported metadata at the strongest algorithm participates: any
/// matching digest at that strength passes; a weaker match cannot override it.
pub fn verify(metadata: &str, bytes: &[u8], eligible: bool) -> Result<(), IntegrityError> {
    if metadata
        .trim_matches(|value: char| value.is_ascii_whitespace())
        .is_empty()
    {
        return Ok(());
    }
    let parsed = metadata
        .split_ascii_whitespace()
        .filter_map(token)
        .collect::<Vec<_>>();
    let Some(strongest) = parsed.iter().map(|entry| entry.0).max() else {
        return Ok(());
    };
    if !eligible {
        return Err(IntegrityError::IneligibleResponse);
    }
    let actual = match strongest {
        Algorithm::Sha256 => Sha256::digest(bytes).to_vec(),
        Algorithm::Sha384 => Sha384::digest(bytes).to_vec(),
        Algorithm::Sha512 => Sha512::digest(bytes).to_vec(),
    };
    if parsed
        .iter()
        .any(|(algorithm, digest)| *algorithm == strongest && digest.as_ref() == Some(&actual))
    {
        Ok(())
    } else {
        Err(IntegrityError::DigestMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_hash_checks_exact_representation_bytes() {
        let metadata = "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
        assert_eq!(verify(metadata, b"hello", true), Ok(()));
        assert_eq!(
            verify(metadata, b"hello\n", true),
            Err(IntegrityError::DigestMismatch)
        );
        assert_eq!(
            verify(metadata, b"hello", false),
            Err(IntegrityError::IneligibleResponse)
        );
        assert_eq!(verify("", b"hello", false), Ok(()));
    }
    #[test]
    fn strongest_algorithm_and_multiple_alternatives() {
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let weak = format!("sha256-{}", encode(&Sha256::digest(b"hello")));
        let wrong = format!("sha512-{}", encode(&[0u8; 64]));
        let strong = format!("sha512-{}", encode(&Sha512::digest(b"hello")));
        assert_eq!(
            verify(&format!("{weak} {wrong}"), b"hello", true),
            Err(IntegrityError::DigestMismatch)
        );
        assert_eq!(
            verify(&format!("{weak} {wrong} {strong}"), b"hello", true),
            Ok(())
        );
        assert_eq!(
            verify(
                &format!("sha384-{}", encode(&Sha384::digest(b"hello"))),
                b"hello",
                true
            ),
            Ok(())
        );
        // A syntactically valid strongest digest of the wrong length fails; it
        // does not allow the weaker matching metadata to take over.
        assert_eq!(
            verify(&format!("{weak} sha512-AA=="), b"hello", true),
            Err(IntegrityError::DigestMismatch)
        );
    }
    #[test]
    fn undecodable_supported_metadata_cannot_disable_or_downgrade_integrity() {
        let good = "sha256-LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
        for wrong in ["sha256-foobar", "sha256-A"] {
            assert_eq!(
                verify(wrong, b"hello", true),
                Err(IntegrityError::DigestMismatch),
                "{wrong}"
            );
        }
        assert_eq!(verify("sha256-A===?option", b"hello", true), Ok(()));
        assert_eq!(
            verify("sha256-foobar?option", b"hello", false),
            Err(IntegrityError::IneligibleResponse)
        );
        assert_eq!(
            verify(&format!("{good} sha512-foobar"), b"hello", true),
            Err(IntegrityError::DigestMismatch)
        );
        assert_eq!(
            verify(&format!("sha256-foobar {good}"), b"hello", true),
            Ok(())
        );
        assert_eq!(verify(&format!("{good} sha512-!"), b"hello", true), Ok(()));
    }
    #[test]
    fn malformed_unsupported_and_optional_metadata() {
        assert_eq!(
            verify("sha1-unknown nonsense sha256-!", b"hello", true),
            Ok(())
        );
        assert_eq!(
            verify("sha1-unknown nonsense sha256-!", b"hello", false),
            Ok(())
        );
        let unpadded =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(b"hello"));
        assert_eq!(
            verify(&format!("sha256-{unpadded}?option"), b"hello", true),
            Ok(())
        );
        assert_eq!(
            verify("sha256-AA==", b"hello", true),
            Err(IntegrityError::DigestMismatch)
        );
    }
}
