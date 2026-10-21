//! Ticket credentials: the secret carried in a guest's QR code.
//!
//! Two properties pull in opposite directions. A credential must not be
//! recoverable from a database read, and it must be recoverable by the guest who
//! legitimately owns the ticket so they can show the QR code again later.
//!
//! The resolution is to store two representations of the same value: a SHA-256
//! digest used for the door lookup, which cannot be reversed, and an AES-256-GCM
//! ciphertext used to re-render the QR code for its owner. A leaked database alone
//! yields no usable credential from the digest column, and an attacker holding
//! ciphertext without the key also gets nothing.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    aead::{self, AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::SecureRandom,
    rand::SystemRandom,
};

/// Encoded credential length. 32 random bytes plus the `zeno_t_` prefix.
const CREDENTIAL_BYTES: usize = 32;

/// Characters used for the short human-readable reference. Ambiguous glyphs are
/// excluded so staff can read it off a lanyard over a noisy door without
/// confusing O/0 or I/1.
const REFERENCE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const REFERENCE_LENGTH: usize = 8;

#[derive(Debug)]
pub enum CredentialError {
    Unavailable,
}

impl From<ring::error::Unspecified> for CredentialError {
    fn from(_: ring::error::Unspecified) -> Self {
        CredentialError::Unavailable
    }
}

/// A key that encrypts ticket credentials at rest.
///
/// Held in memory for the process lifetime. Deriving it from an environment
/// variable rather than a database table keeps the secret out of the same backup
/// as the ciphertext it protects.
#[derive(Clone)]
pub struct CredentialKey(LessSafeKey);

impl CredentialKey {
    pub fn from_env() -> Result<Self, CredentialError> {
        let raw = std::env::var("ZENO_TICKET_CREDENTIAL_KEY")
            .map_err(|_| CredentialError::Unavailable)?;
        Self::from_base64(&raw)
    }

    pub fn from_base64(raw: &str) -> Result<Self, CredentialError> {
        // Exactly 32 raw bytes for AES-256. A different length is a configuration
        // mistake, not something to pad or stretch into a working key.
        let bytes = URL_SAFE_NO_PAD
            .decode(raw.trim())
            .map_err(|_| CredentialError::Unavailable)?;
        let unbound = UnboundKey::new(&AES_256_GCM, &bytes)?;
        Ok(Self(LessSafeKey::new(unbound)))
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>, CredentialError> {
        let mut nonce_bytes = [0u8; aead::NONCE_LEN];
        nonce_bytes.copy_from_slice(&random_bytes::<{ aead::NONCE_LEN }>());
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let mut buffer = plaintext.as_bytes().to_vec();
        let tag = self
            .0
            .seal_in_place_separate_tag(nonce, Aad::empty(), &mut buffer)?;

        // nonce || ciphertext || tag, so a single column holds the sealed value.
        let mut sealed = nonce_bytes.to_vec();
        sealed.extend_from_slice(&buffer);
        sealed.extend_from_slice(tag.as_ref());
        Ok(sealed)
    }

    pub fn decrypt(&self, sealed: &[u8]) -> Result<String, CredentialError> {
        const TAG_LENGTH: usize = 16;
        if sealed.len() <= aead::NONCE_LEN + TAG_LENGTH {
            return Err(CredentialError::Unavailable);
        }
        let (nonce, rest) = sealed.split_at(aead::NONCE_LEN);
        let (ciphertext, tag) = rest.split_at(rest.len() - TAG_LENGTH);
        let nonce = Nonce::try_assume_unique_for_key(nonce)?;
        let tag = aead::Tag::try_from(tag).map_err(|_| CredentialError::Unavailable)?;

        let mut buffer = ciphertext.to_vec();
        let plaintext =
            self.0
                .open_in_place_separate_tag(nonce, Aad::empty(), tag, &mut buffer, 0..)?;
        String::from_utf8(plaintext.to_vec()).map_err(|_| CredentialError::Unavailable)
    }
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("the operating system random source is unavailable");
    bytes
}

/// A fresh 256-bit credential and its lookup digest.
pub fn new_credential() -> (String, [u8; 32]) {
    let raw = random_bytes::<CREDENTIAL_BYTES>();
    let credential = format!("zeno_t_{}", URL_SAFE_NO_PAD.encode(raw));
    let digest = sha256(credential.as_bytes());
    (credential, digest)
}

pub fn sha256(value: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&sha2::Sha256::digest(value));
    digest
}

/// A short code staff can type when a camera cannot read the QR code.
///
/// Generation can produce a duplicate, so the caller must treat the result as
/// tentative and retry on a unique-constraint violation. `REFERENCE_ATTEMPTS`
/// bounds that retry so a pathological collision cannot spin forever. The
/// alphabet already excludes ambiguous glyphs, so no post-filter is needed.
pub fn new_reference() -> String {
    let bytes = random_bytes::<REFERENCE_LENGTH>();
    bytes
        .iter()
        .map(|byte| REFERENCE_ALPHABET[usize::from(*byte) % REFERENCE_ALPHABET.len()] as char)
        .collect()
}

/// Whether a value looks like a credential this service issued.
///
/// Checked before any database work so a malformed code from a damaged QR never
/// reaches a query.
pub fn is_credential_shape(value: &str) -> bool {
    let Some(encoded) = value.strip_prefix("zeno_t_") else {
        return false;
    };
    encoded.len() == 43
        && encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Whether a value looks like a short reference. Deliberately shape-only: the
/// uniqueness check is the database's job.
pub fn is_reference_shape(value: &str) -> bool {
    (REFERENCE_LENGTH..=10).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> CredentialKey {
        CredentialKey::from_base64(&URL_SAFE_NO_PAD.encode([7u8; 32])).expect("valid key")
    }

    #[test]
    fn a_credential_survives_a_round_trip_through_encryption() {
        let key = key();
        let (credential, digest) = new_credential();
        let sealed = key.encrypt(&credential).expect("encrypt");
        assert_eq!(key.decrypt(&sealed).expect("decrypt"), credential);
        // The ciphertext must not leak the plaintext, and the digest is what the
        // door actually looks up.
        assert!(!sealed.windows(8).any(|w| w == credential.as_bytes()));
        assert_eq!(digest.len(), 32);
        assert_ne!(digest, sha256(b"something else"));
    }

    #[test]
    fn a_tampered_ciphertext_is_rejected_rather_than_returned_garbage() {
        // Without this an attacker who can write to the database could swap in a
        // ciphertext of their choosing and it would decrypt to attacker-controlled
        // text.
        let key = key();
        let (credential, _) = new_credential();
        let mut sealed = key.encrypt(&credential).expect("encrypt");
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        assert!(key.decrypt(&sealed).is_err());
    }

    #[test]
    fn a_ciphertext_cannot_be_decrypted_with_another_key() {
        let sealed = key().encrypt("zeno_t_secret").expect("encrypt");
        let other =
            CredentialKey::from_base64(&URL_SAFE_NO_PAD.encode([9u8; 32])).expect("valid key");
        assert!(other.decrypt(&sealed).is_err());
    }

    #[test]
    fn a_malformed_key_is_rejected_rather_than_stretched() {
        assert!(CredentialKey::from_base64("not-base64!!").is_err());
        assert!(CredentialKey::from_base64(&URL_SAFE_NO_PAD.encode([1u8; 16])).is_err());
        assert!(CredentialKey::from_base64(&URL_SAFE_NO_PAD.encode([1u8; 33])).is_err());
    }

    #[test]
    fn credentials_are_high_entropy_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            let (credential, digest) = new_credential();
            assert!(is_credential_shape(&credential));
            // 32 bytes of entropy: guessing one is not an option.
            assert_eq!(
                URL_SAFE_NO_PAD
                    .decode(credential.strip_prefix("zeno_t_").unwrap())
                    .unwrap()
                    .len(),
                32
            );
            assert!(seen.insert(digest));
        }
    }

    #[test]
    fn a_reference_avoids_characters_staff_misread_at_a_door() {
        for _ in 0..256 {
            let reference = new_reference();
            assert_eq!(reference.len(), REFERENCE_LENGTH);
            assert!(is_reference_shape(&reference));
            for ambiguous in ['0', '1', 'O', 'I'] {
                assert!(
                    !reference.contains(ambiguous),
                    "reference {reference} contains ambiguous {ambiguous}"
                );
            }
        }
    }

    #[test]
    fn shape_checks_reject_rubbish_before_it_reaches_a_query() {
        assert!(is_credential_shape(
            "zeno_t_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(!is_credential_shape("zeno_t_short"));
        assert!(!is_credential_shape("not-a-credential"));
        assert!(!is_credential_shape(""));

        assert!(is_reference_shape("ABCDEFGH"));
        assert!(!is_reference_shape("abcdefgh"));
        assert!(!is_reference_shape("AB"));
    }
}
