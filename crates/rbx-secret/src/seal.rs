//! Sealing a value for one universe's public key.
//!
//! Roblox will not accept a secret in the clear. A write carries the content
//! already encrypted, as a **LibSodium sealed box** under the universe's
//! X25519 public key, base64-encoded, so this module is not a convenience,
//! it is the difference between the command existing and not.
//!
//! ## What a sealed box is, and why it is the right shape here
//!
//! An ordinary NaCl `box` authenticates *both* ends: the sender needs a
//! long-term keypair the receiver already knows. That is not this situation.
//! Whoever runs `rbx secret set` has no identity registered with Roblox
//! beyond the API key in the header, and Roblox has no interest in one: the
//! key already said who is calling.
//!
//! A sealed box drops the sender's half. It generates a throwaway keypair per
//! call, does the Diffie-Hellman against the recipient's public key, prepends
//! the ephemeral public key to the ciphertext, and forgets the private half.
//! The recipient can open it; nobody else can, **including the process that
//! sealed it**. That last property is the one that makes this safe to run in
//! CI: a sealed value in a build log is not a secret leak.
//!
//! The output is exactly `ephemeral_public_key (32) || ciphertext || tag (16)`,
//! so a 7-byte secret seals to 55 bytes. The length is not hidden, which is
//! worth knowing but rarely worth acting on.
//!
//! ## Why the key id travels with it
//!
//! [`Sealed`] carries the `key_id` from the same response the public key came
//! from, rather than letting the caller pair them up. A value sealed under a
//! rotated key and submitted with the current key's id is a secret Roblox
//! stores and can never decrypt: a failure that surfaces months later, in
//! production, as `GetSecret` returning something unusable. Keeping the two
//! in one value means they cannot drift apart in a call site.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use crypto_box::aead::OsRng;
use crypto_box::PublicKey;

use crate::model::Secret;

/// X25519 public keys are 32 bytes. Named because the error when they are not
/// is worth writing out.
const KEY_LEN: usize = 32;

/// A universe's public key, ready to seal against.
#[derive(Debug, Clone)]
pub struct UniverseKey {
    key: PublicKey,
    key_id: String,
    /// Kept for `rbx secret public-key`, which prints what Roblox sent rather
    /// than a re-encoding of what we parsed.
    encoded: String,
}

/// A sealed value and the key id it must be submitted with.
#[derive(Debug, Clone)]
pub struct Sealed {
    /// Base64 of `ephemeral_pk || ciphertext || tag`.
    pub content: String,
    pub key_id: String,
}

impl UniverseKey {
    /// Read the key out of a `secrets/public-key` response.
    ///
    /// Every failure here is a malformed response rather than user error,
    /// which is why they all name the endpoint: someone reading
    /// "the public key is 20 bytes" needs to know it was not their input.
    pub fn from_response(response: &Secret) -> Result<Self> {
        let encoded = response
            .secret
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow!("the public-key response carried no key; nothing can be sealed for this universe")
            })?;
        let key_id = response
            .key_id
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "the public-key response carried no key_id; a write without one is rejected"
                )
            })?;

        let raw = STANDARD
            .decode(encoded)
            .context("the public-key response is not valid base64")?;
        let len = raw.len();
        let bytes: [u8; KEY_LEN] = raw
            .try_into()
            .map_err(|_| anyhow!("the public key is {len} bytes; an X25519 key is {KEY_LEN}"))?;

        Ok(Self {
            key: PublicKey::from(bytes),
            key_id: key_id.to_string(),
            encoded: encoded.to_string(),
        })
    }

    /// The key id Roblox will match a write against.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The key as Roblox sent it, base64.
    pub fn encoded(&self) -> &str {
        &self.encoded
    }

    /// Seal one value.
    ///
    /// Takes bytes, not a `str`: the documented content includes private keys
    /// and other binary material, and a signature that only accepted UTF-8
    /// would quietly rule that out.
    ///
    /// `OsRng` and not a seeded generator anywhere in reach: the ephemeral
    /// keypair is the whole confidentiality of the scheme, and a reproducible
    /// one would make two seals of the same value byte-identical, which is
    /// itself a leak.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Sealed> {
        if plaintext.is_empty() {
            bail!("refusing to store an empty secret; pass a value, or delete the secret instead");
        }
        let sealed = self
            .key
            .seal(&mut OsRng, plaintext)
            .map_err(|_| anyhow!("sealing the value failed"))?;

        Ok(Sealed {
            content: STANDARD.encode(&sealed),
            key_id: self.key_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto_box::SecretKey;

    /// Stand in for Roblox: a universe keypair whose private half a test can
    /// use to prove the ciphertext is a real sealed box rather than something
    /// that merely looks like one.
    fn universe() -> (SecretKey, Secret) {
        let private = SecretKey::generate(&mut OsRng);
        let response = Secret {
            id: Some("public-key".into()),
            secret: Some(STANDARD.encode(private.public_key().as_bytes())),
            key_id: Some("key-2026-08".into()),
            ..Secret::default()
        };
        (private, response)
    }

    /// The test that actually matters: what we send is something Roblox can
    /// open. Asserting the length or the base64 shape would pass just as
    /// happily on a value encrypted for the wrong key.
    #[test]
    fn a_sealed_value_opens_with_the_universes_private_key() {
        let (private, response) = universe();
        let key = UniverseKey::from_response(&response).expect("parse");

        let sealed = key.seal(b"hunter2").expect("seal");
        assert_eq!(sealed.key_id, "key-2026-08");

        let ciphertext = STANDARD.decode(&sealed.content).expect("base64");
        let opened = private.unseal(&ciphertext).expect("unseal");
        assert_eq!(opened, b"hunter2");
    }

    /// Binary content is in the documented use ("private keys"), and a value
    /// that is not UTF-8 must survive the round trip untouched.
    #[test]
    fn binary_content_survives() {
        let (private, response) = universe();
        let key = UniverseKey::from_response(&response).expect("parse");
        let raw: Vec<u8> = (0u8..=255).collect();

        let sealed = key.seal(&raw).expect("seal");
        let opened = private
            .unseal(&STANDARD.decode(&sealed.content).expect("base64"))
            .expect("unseal");
        assert_eq!(opened, raw);
    }

    /// Two seals of one value must differ. Equal ciphertexts would mean the
    /// ephemeral keypair is not ephemeral, and an observer could tell that a
    /// rotation put the old value back.
    #[test]
    fn sealing_twice_does_not_produce_the_same_ciphertext() {
        let (_, response) = universe();
        let key = UniverseKey::from_response(&response).expect("parse");

        let first = key.seal(b"same value").expect("seal");
        let second = key.seal(b"same value").expect("seal");
        assert_ne!(first.content, second.content);
    }

    /// 32 bytes of ephemeral public key and a 16-byte tag on top of the
    /// plaintext. Asserted so that a dependency bump to a construction with a
    /// different overhead is a test failure rather than a `400` from Roblox.
    #[test]
    fn the_overhead_is_the_libsodium_sealed_box_overhead() {
        let (_, response) = universe();
        let key = UniverseKey::from_response(&response).expect("parse");

        let sealed = key.seal(b"1234567").expect("seal");
        let ciphertext = STANDARD.decode(&sealed.content).expect("base64");
        assert_eq!(ciphertext.len(), 32 + 7 + 16);
    }

    #[test]
    fn an_empty_value_is_refused_before_it_reaches_roblox() {
        let (_, response) = universe();
        let key = UniverseKey::from_response(&response).expect("parse");
        assert!(key.seal(b"").is_err());
    }

    #[test]
    fn a_malformed_public_key_response_says_what_was_wrong_with_it() {
        let missing_key = Secret {
            key_id: Some("k1".into()),
            ..Secret::default()
        };
        let error = UniverseKey::from_response(&missing_key).expect_err("no key");
        assert!(error.to_string().contains("no key"), "{error}");

        let missing_id = Secret {
            secret: Some(STANDARD.encode([0u8; KEY_LEN])),
            ..Secret::default()
        };
        let error = UniverseKey::from_response(&missing_id).expect_err("no key_id");
        assert!(error.to_string().contains("key_id"), "{error}");

        let short = Secret {
            secret: Some(STANDARD.encode([0u8; 20])),
            key_id: Some("k1".into()),
            ..Secret::default()
        };
        let error = UniverseKey::from_response(&short).expect_err("short key");
        assert!(error.to_string().contains("20 bytes"), "{error}");

        let not_base64 = Secret {
            secret: Some("not base64!!".into()),
            key_id: Some("k1".into()),
            ..Secret::default()
        };
        let error = UniverseKey::from_response(&not_base64).expect_err("bad base64");
        assert!(error.to_string().contains("base64"), "{error}");
    }
}
