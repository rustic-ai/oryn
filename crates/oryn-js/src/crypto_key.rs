//! Validated native AES-CBC/Ed25519 key transport and cryptographic primitives.
//! Key bytes are native-only and erased on release; Debug never emits material.
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use aes::{Aes128, Aes192, Aes256};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const MAX_DATA: usize = 32 * 1024 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Usage {
    Encrypt,
    Decrypt,
    WrapKey,
    UnwrapKey,
    Sign,
    Verify,
}
impl Usage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Encrypt => "encrypt",
            Self::Decrypt => "decrypt",
            Self::WrapKey => "wrapKey",
            Self::UnwrapKey => "unwrapKey",
            Self::Sign => "sign",
            Self::Verify => "verify",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    version: u8,
    algorithm: String,
    private: Option<bool>,
    extractable: bool,
    usages: Vec<Usage>,
    material: Zeroizing<Vec<u8>>,
}
impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CryptoKey")
            .field("algorithm", &self.algorithm)
            .field("extractable", &self.extractable)
            .field("usages", &self.usages)
            .field("bits", &self.bits())
            .finish_non_exhaustive()
    }
}
impl Key {
    pub fn new(
        material: Vec<u8>,
        extractable: bool,
        usages: Vec<Usage>,
    ) -> Result<Self, &'static str> {
        let key = Self {
            version: 1,
            algorithm: "AES-CBC".into(),
            private: None,
            extractable,
            usages,
            material: Zeroizing::new(material),
        };
        key.validate()?;
        Ok(key)
    }
    pub fn generate(
        bits: usize,
        extractable: bool,
        usages: Vec<Usage>,
    ) -> Result<Self, &'static str> {
        if !matches!(bits, 128 | 192 | 256) {
            return Err("OperationError");
        }
        let mut bytes = Zeroizing::new(vec![0; bits / 8]);
        getrandom::fill(&mut bytes).map_err(|_| "OperationError")?;
        Self::new(std::mem::take(&mut *bytes), extractable, usages)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 1 || self.usages.len() > 4 {
            return Err("DataError");
        }
        let valid = match (self.algorithm.as_str(), self.private) {
            ("AES-CBC", None) => {
                matches!(self.material.len(), 16 | 24 | 32)
                    && !self.usages.is_empty()
                    && self.usages.iter().all(|usage| {
                        matches!(
                            usage,
                            Usage::Encrypt | Usage::Decrypt | Usage::WrapKey | Usage::UnwrapKey
                        )
                    })
            }
            ("Ed25519", Some(private)) => {
                self.material.len() == 32
                    && (!private || !self.usages.is_empty())
                    && self
                        .usages
                        .iter()
                        .all(|usage| *usage == if private { Usage::Sign } else { Usage::Verify })
            }
            _ => false,
        };
        if !valid {
            return Err("DataError");
        }
        for (index, usage) in self.usages.iter().enumerate() {
            if self.usages[..index].contains(usage) {
                return Err("DataError");
            }
        }
        if self.private == Some(false) {
            ed25519_dalek::VerifyingKey::from_bytes(
                self.material
                    .as_slice()
                    .try_into()
                    .map_err(|_| "DataError")?,
            )
            .map_err(|_| "DataError")?;
        }
        Ok(())
    }
    pub fn kind(&self) -> &'static str {
        match self.private {
            None => "secret",
            Some(true) => "private",
            Some(false) => "public",
        }
    }
    pub fn ed25519(
        material: Vec<u8>,
        private: bool,
        extractable: bool,
        usages: Vec<Usage>,
    ) -> Result<Self, &'static str> {
        let key = Self {
            version: 1,
            algorithm: "Ed25519".into(),
            private: Some(private),
            extractable,
            usages,
            material: Zeroizing::new(material),
        };
        key.validate()?;
        Ok(key)
    }
    pub fn generate_ed25519(
        extractable: bool,
        usages: Vec<Usage>,
    ) -> Result<(Self, Self), &'static str> {
        if !usages.contains(&Usage::Sign)
            || usages
                .iter()
                .any(|usage| !matches!(usage, Usage::Sign | Usage::Verify))
        {
            return Err("SyntaxError");
        }
        let mut seed = Zeroizing::new(vec![0; 32]);
        getrandom::fill(&mut seed).map_err(|_| "OperationError")?;
        let signing = ed25519_dalek::SigningKey::from_bytes(seed.as_slice().try_into().unwrap());
        let public = Self::ed25519(
            signing.verifying_key().to_bytes().to_vec(),
            false,
            true,
            usages
                .iter()
                .copied()
                .filter(|usage| *usage == Usage::Verify)
                .collect(),
        )?;
        let private = Self::ed25519(
            std::mem::take(&mut *seed),
            true,
            extractable,
            vec![Usage::Sign],
        )?;
        Ok((public, private))
    }
    pub fn public_bytes(&self) -> Result<[u8; 32], &'static str> {
        if self.algorithm != "Ed25519" {
            return Err("InvalidAccessError");
        }
        let bytes: &[u8; 32] = self
            .material
            .as_slice()
            .try_into()
            .map_err(|_| "DataError")?;
        Ok(if self.private == Some(true) {
            ed25519_dalek::SigningKey::from_bytes(bytes)
                .verifying_key()
                .to_bytes()
        } else {
            *bytes
        })
    }
    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>, &'static str> {
        use ed25519_dalek::Signer;
        if self.algorithm != "Ed25519" || self.private != Some(true) || !self.permits(Usage::Sign) {
            return Err("InvalidAccessError");
        }
        if data.len() > MAX_DATA {
            return Err("OperationError");
        }
        Ok(ed25519_dalek::SigningKey::from_bytes(
            self.material
                .as_slice()
                .try_into()
                .map_err(|_| "DataError")?,
        )
        .sign(data)
        .to_bytes()
        .to_vec())
    }
    pub fn verify(&self, signature: &[u8], data: &[u8]) -> Result<bool, &'static str> {
        if self.algorithm != "Ed25519"
            || self.private != Some(false)
            || !self.permits(Usage::Verify)
        {
            return Err("InvalidAccessError");
        }
        if data.len() > MAX_DATA {
            return Err("OperationError");
        }
        let Ok(signature) = ed25519_dalek::Signature::from_slice(signature) else {
            return Ok(false);
        };
        let public = ed25519_dalek::VerifyingKey::from_bytes(
            self.material
                .as_slice()
                .try_into()
                .map_err(|_| "DataError")?,
        )
        .map_err(|_| "DataError")?;
        Ok(public.verify_strict(data, &signature).is_ok())
    }
    pub fn import_der(
        format: &str,
        data: &[u8],
        extractable: bool,
        usages: Vec<Usage>,
    ) -> Result<Self, &'static str> {
        use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey};
        match format {
            "pkcs8" => {
                let key =
                    ed25519_dalek::SigningKey::from_pkcs8_der(data).map_err(|_| "DataError")?;
                Self::ed25519(key.to_bytes().to_vec(), true, extractable, usages)
            }
            "spki" => {
                let key = ed25519_dalek::VerifyingKey::from_public_key_der(data)
                    .map_err(|_| "DataError")?;
                Self::ed25519(key.to_bytes().to_vec(), false, extractable, usages)
            }
            _ => Err("NotSupportedError"),
        }
    }
    pub fn export_der(&self, format: &str) -> Result<Zeroizing<Vec<u8>>, &'static str> {
        use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
        if !self.extractable {
            return Err("InvalidAccessError");
        }
        if self.algorithm != "Ed25519" {
            return Err("NotSupportedError");
        }
        match (format, self.private) {
            ("pkcs8", Some(true)) => {
                let key = ed25519_dalek::SigningKey::from_bytes(
                    self.material
                        .as_slice()
                        .try_into()
                        .map_err(|_| "DataError")?,
                );
                Ok(Zeroizing::new(
                    key.to_pkcs8_der()
                        .map_err(|_| "OperationError")?
                        .as_bytes()
                        .to_vec(),
                ))
            }
            ("spki", Some(false)) => {
                let key = ed25519_dalek::VerifyingKey::from_bytes(
                    self.material
                        .as_slice()
                        .try_into()
                        .map_err(|_| "DataError")?,
                )
                .map_err(|_| "DataError")?;
                Ok(Zeroizing::new(
                    key.to_public_key_der()
                        .map_err(|_| "OperationError")?
                        .as_bytes()
                        .to_vec(),
                ))
            }
            _ => Err("InvalidAccessError"),
        }
    }

    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }
    pub fn bits(&self) -> usize {
        self.material.len() * 8
    }
    pub fn extractable(&self) -> bool {
        self.extractable
    }
    pub fn usages(&self) -> &[Usage] {
        &self.usages
    }
    pub fn permits(&self, usage: Usage) -> bool {
        self.usages.contains(&usage)
    }
    pub fn retained_bytes(&self) -> usize {
        self.material
            .len()
            .saturating_add(self.algorithm.len())
            .saturating_add(self.usages.len())
            .saturating_add(8)
    }
    pub fn export(&self) -> Result<Zeroizing<Vec<u8>>, &'static str> {
        if !self.extractable {
            return Err("InvalidAccessError");
        }
        Ok(self.material.clone())
    }
    pub fn crypt(
        &self,
        iv: &[u8],
        input: &[u8],
        decrypt: bool,
        usage: Usage,
    ) -> Result<Vec<u8>, &'static str> {
        self.validate()?;
        if self.algorithm != "AES-CBC" {
            return Err("InvalidAccessError");
        }
        if !self.permits(usage) {
            return Err("InvalidAccessError");
        }
        if iv.len() != 16 || input.len() > MAX_DATA {
            return Err("OperationError");
        }
        if decrypt && (input.is_empty() || input.len() % 16 != 0) {
            return Err("OperationError");
        }
        if !decrypt
            && input
                .len()
                .checked_add(16)
                .is_none_or(|length| length > MAX_DATA)
        {
            return Err("OperationError");
        }
        macro_rules! run {
            ($cipher:ty) => {
                if decrypt {
                    cbc::Decryptor::<$cipher>::new_from_slices(&self.material, iv)
                        .map_err(|_| "OperationError")?
                        .decrypt_padded_vec_mut::<Pkcs7>(input)
                        .map_err(|_| "OperationError")
                } else {
                    Ok(
                        cbc::Encryptor::<$cipher>::new_from_slices(&self.material, iv)
                            .map_err(|_| "OperationError")?
                            .encrypt_padded_vec_mut::<Pkcs7>(input),
                    )
                }
            };
        }
        match self.material.len() {
            16 => run!(Aes128),
            24 => run!(Aes192),
            32 => run!(Aes256),
            _ => Err("OperationError"),
        }
    }
}
pub fn digest(name: &str, data: &[u8]) -> Result<Vec<u8>, &'static str> {
    use sha2::Digest;
    if data.len() > MAX_DATA {
        return Err("OperationError");
    }
    Ok(match name.to_ascii_uppercase().as_str() {
        "SHA-1" => sha1::Sha1::digest(data).to_vec(),
        "SHA-256" => sha2::Sha256::digest(data).to_vec(),
        "SHA-384" => sha2::Sha384::digest(data).to_vec(),
        "SHA-512" => sha2::Sha512::digest(data).to_vec(),
        _ => return Err("NotSupportedError"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aes_cbc_known_answer_padding_and_nonextractable_custody() {
        let key = Key::new(vec![0; 16], false, vec![Usage::Encrypt, Usage::Decrypt]).unwrap();
        let output = key
            .crypt(&[0; 16], &[0; 16], false, Usage::Encrypt)
            .unwrap();
        assert_eq!(
            &output[..16],
            &[
                0x66, 0xe9, 0x4b, 0xd4, 0xef, 0x8a, 0x2c, 0x3b, 0x88, 0x4c, 0xfa, 0x59, 0xca, 0x34,
                0x2b, 0x2e
            ]
        );
        assert_eq!(
            key.crypt(&[0; 16], &output, true, Usage::Decrypt).unwrap(),
            vec![0; 16]
        );
        assert_eq!(key.export().unwrap_err(), "InvalidAccessError");
        assert_eq!(
            key.crypt(&[0; 16], &output[..16], true, Usage::Decrypt)
                .unwrap_err(),
            "OperationError"
        );
        assert_eq!(
            key.crypt(&[0; 15], &output, true, Usage::Decrypt)
                .unwrap_err(),
            "OperationError"
        );
        let serialized = serde_json::to_value(&key).unwrap();
        let restored: Key = serde_json::from_value(serialized.clone()).unwrap();
        restored.validate().unwrap();
        assert!(!restored.extractable());
        assert_eq!(
            restored
                .crypt(&[0; 16], &output, true, Usage::Decrypt)
                .unwrap(),
            vec![0; 16]
        );
        let mut corrupt = serialized;
        corrupt["algorithm"] = serde_json::json!("RSA-OAEP");
        assert!(
            serde_json::from_value::<Key>(corrupt)
                .unwrap()
                .validate()
                .is_err()
        );
        assert!(!format!("{key:?}").contains("material"));
    }
    #[test]
    fn ed25519_rfc8032_vector_nonextractable_clone_and_der() {
        fn hex(text: &str) -> Vec<u8> {
            text.as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect()
        }
        let seed = hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let public = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let signature = hex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        let private = Key::ed25519(seed.clone(), true, false, vec![Usage::Sign]).unwrap();
        let verify = Key::ed25519(public.clone(), false, true, vec![Usage::Verify]).unwrap();
        assert_eq!(private.public_bytes().unwrap().as_slice(), public);
        assert_eq!(private.sign(b"").unwrap(), signature);
        assert!(verify.verify(&signature, b"").unwrap());
        assert!(!verify.verify(&signature, b"changed").unwrap());
        assert!(private.export().is_err());
        let cloned: Key = serde_json::from_value(serde_json::to_value(&private).unwrap()).unwrap();
        cloned.validate().unwrap();
        assert_eq!(cloned.sign(b"").unwrap(), signature);
        assert!(!cloned.extractable());
        let exportable = Key::ed25519(seed, true, true, vec![Usage::Sign]).unwrap();
        let encoded = exportable.export_der("pkcs8").unwrap();
        assert_eq!(
            Key::import_der("pkcs8", &encoded, false, vec![Usage::Sign])
                .unwrap()
                .sign(b"")
                .unwrap(),
            signature
        );
        let encoded = verify.export_der("spki").unwrap();
        assert!(
            Key::import_der("spki", &encoded, true, vec![Usage::Verify])
                .unwrap()
                .verify(&signature, b"")
                .unwrap()
        );
    }
    #[test]
    fn typed_wire_validation_rejects_key_metadata_and_material_tampering() {
        let key = Key::new(vec![0; 16], false, vec![Usage::Encrypt]).unwrap();
        let wire = crate::message_wire::WireValue {
            shared: vec![],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: vec![],
            version: 1,
            bytes: vec![],
            hosts: vec![crate::message_wire::WireHost::CryptoKey(key)],
            buffers: vec![],
            ports: vec![],
        };
        wire.validate().unwrap();
        let encoded = serde_json::to_value(&wire).unwrap();
        for (field, value) in [
            ("version", serde_json::json!(2)),
            ("material", serde_json::json!([0, 1, 2])),
            ("usages", serde_json::json!(["Sign"])),
            ("private", serde_json::json!(true)),
        ] {
            let mut tampered = encoded.clone();
            tampered["hosts"][0]["CryptoKey"][field] = value;
            let decoded: crate::message_wire::WireValue = serde_json::from_value(tampered).unwrap();
            assert!(decoded.validate().is_err());
        }
    }
    #[test]
    fn digest_vectors_and_key_usage_validation() {
        assert_eq!(
            digest("SHA-256", b"abc").unwrap(),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
        assert!(Key::generate(128, false, vec![]).is_err());
        assert!(Key::new(vec![0; 15], true, vec![Usage::Encrypt]).is_err());
    }
}
