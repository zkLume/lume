//! Schnorr signing over the Cheetah curve — generic SDK implementation.
//!
//! Implements the same Schnorr signing algorithm as Hoon's
//! `sign:affine:belt-schnorr:cheetah` (three.hoon lines 1628-1661).
//!
//! The algorithm:
//! 1. Deterministic nonce = trunc_g_order(hash_varlen([pk.x, pk.y, msg, sk]))
//! 2. R = nonce * G
//! 3. Challenge = trunc_g_order(hash_varlen([R.x, R.y, pk.x, pk.y, msg]))
//! 4. Signature = (nonce + challenge * sk) mod g_order
//! 5. Return (challenge, signature) as [Belt; 8] each (8 x 32-bit chunks)

use std::fmt;

use ibig::UBig;
use zeroize::Zeroize;
use nockchain_math::belt::Belt;
use nockchain_math::crypto::cheetah::{ch_scal_big, trunc_g_order, A_GEN, G_ORDER};
use nockchain_math::tip5::hash::hash_varlen;
use nockchain_types::tx_engine::common::{Hash, SchnorrPubkey, SchnorrSignature};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SigningError {
    InvalidSecretKey,
    ZeroNonce,
    ZeroChallenge,
    ZeroSignature,
    ZeroSeedScalar,
}

impl fmt::Display for SigningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSecretKey => write!(f, "secret key must be in (0, g_order)"),
            Self::ZeroNonce => write!(f, "deterministic nonce was zero"),
            Self::ZeroChallenge => write!(f, "challenge was zero"),
            Self::ZeroSignature => write!(f, "signature was zero"),
            Self::ZeroSeedScalar => write!(f, "seed phrase produced zero scalar — use a different phrase"),
        }
    }
}

impl std::error::Error for SigningError {}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Derive the Schnorr public key from a secret key.
///
/// `sk` is 8 x 32-bit Belt chunks (little-endian order, matching Hoon's t8).
pub fn derive_pubkey(sk: &[Belt; 8]) -> SchnorrPubkey {
    let sk_big = belts8_to_secret(sk);
    let point = ch_scal_big(&sk_big, &A_GEN).expect("valid secret key");
    // sk_big zeroized on drop (C-002)
    SchnorrPubkey(point)
}

/// Compute the PKH (public-key hash) from a public key.
///
/// Matches Hoon's `hash:schnorr-pubkey` = `(hash-hashable:tip5 leaf+pk)`.
/// This hashes the **entire pubkey noun** (including inf flag and cell structure)
/// through `hash_noun_varlen_digest`, NOT just the coordinate belts.
pub fn pubkey_hash(pk: &SchnorrPubkey) -> Hash {
    use nockapp::noun::slab::NounSlab;
    use nockchain_math::tip5::hash::hash_noun_varlen_digest;
    use noun_serde::NounEncode;

    let mut slab: NounSlab = NounSlab::new();
    let noun = pk.to_noun(&mut slab);
    let digest = hash_noun_varlen_digest(&mut slab, noun)
        .expect("hash_noun_varlen_digest should not fail on a valid SchnorrPubkey noun");
    Hash::from_limbs(&digest)
}

/// Sign a message digest with a secret key.
///
/// `sk`: secret key as 8 x 32-bit Belt chunks.
/// `message`: tip5 noun-digest (5 x 64-bit limbs).
///
/// Returns a `SchnorrSignature` with challenge and signature components,
/// each stored as 8 x 32-bit Belt chunks.
///
/// Compatible with Hoon's `sign:affine:belt-schnorr:cheetah`.
///
/// # Deterministic nonce — contract (AUDIT 2026-04-17 M-06)
///
/// The nonce is derived as
/// `trunc_g_order(hash_varlen([pk.x, pk.y, message, sk]))`. This is
/// deterministic: the same `(sk, message)` pair always produces the
/// same nonce, therefore the same signature. Matches the Hoon spec
/// (`sign:affine:belt-schnorr:cheetah`, three.hoon lines 1628-1661).
///
/// Security is only preserved if every call for a given key uses a
/// **distinct** `message` value. Re-signing the same logical document
/// is safe (same signature = no new entropy leaked). Signing two
/// *different* logical documents that happen to hash to the same
/// `message` is not — and any caller that lets the message be chosen
/// (or reused) adversarially breaks the signature scheme.
///
/// Callers who don't fully control message entropy must include a
/// fresh nonce / counter in the message body before digesting. The
/// signing layer does not add randomness on behalf of the caller.
pub fn sign(sk: &[Belt; 8], message: &[Belt; 5]) -> Result<SchnorrSignature, SigningError> {
    let sk_big = belts8_to_secret(sk);
    if *sk_big == UBig::from(0u64) || *sk_big >= *G_ORDER {
        return Err(SigningError::InvalidSecretKey);
    }

    // 1. Derive public key: pk = sk * G
    let pubkey = ch_scal_big(&sk_big, &A_GEN).expect("valid scalar");

    // 2. Deterministic nonce: hash([pk.x, pk.y, message, sk])
    let mut nonce_input: Vec<Belt> = Vec::with_capacity(6 + 6 + 5 + 8);
    nonce_input.extend_from_slice(&pubkey.x.0);
    nonce_input.extend_from_slice(&pubkey.y.0);
    nonce_input.extend_from_slice(message);
    nonce_input.extend_from_slice(sk);
    let nonce_hash = hash_varlen(&mut nonce_input);
    // Zeroize: nonce_input contains secret key material (C-002)
    for b in nonce_input.iter_mut() { b.0.zeroize(); }
    let nonce = SecretScalar(trunc_g_order(&nonce_hash));
    if *nonce == UBig::from(0u64) {
        return Err(SigningError::ZeroNonce);
    }

    // 3. R = nonce * G
    let r_point = ch_scal_big(&nonce, &A_GEN).expect("valid nonce");

    // 4. Challenge: hash([R.x, R.y, pk.x, pk.y, message])
    let mut chal_input: Vec<Belt> = Vec::with_capacity(6 * 4 + 5);
    chal_input.extend_from_slice(&r_point.x.0);
    chal_input.extend_from_slice(&r_point.y.0);
    chal_input.extend_from_slice(&pubkey.x.0);
    chal_input.extend_from_slice(&pubkey.y.0);
    chal_input.extend_from_slice(message);
    let chal_hash = hash_varlen(&mut chal_input);
    let chal = trunc_g_order(&chal_hash);
    if chal == UBig::from(0u64) {
        return Err(SigningError::ZeroChallenge);
    }

    // 5. Signature: sig = (nonce + chal * sk) mod g_order
    // SecretScalar Derefs to UBig, so arithmetic works directly.
    let sig = (&*nonce + &chal * &*sk_big) % &*G_ORDER;
    if sig == UBig::from(0u64) {
        return Err(SigningError::ZeroSignature);
    }

    // 6. Encode as 8 x 32-bit Belt chunks (Hoon's t8 representation)
    let result = SchnorrSignature {
        chal: ubig_to_belts8(&chal),
        sig: ubig_to_belts8(&sig),
    };

    // sk_big and nonce are SecretScalar — zeroized on drop (C-002).
    drop(nonce);
    drop(sk_big);

    Ok(result)
}

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

/// Derive a signing key from a seed phrase.
///
/// Hashes the phrase bytes through tip5's `hash_varlen`, then truncates
/// to a valid scalar in (0, g_order) and packs into 8 x 32-bit Belts.
pub fn key_from_seed_phrase(phrase: &str) -> Result<[Belt; 8], SigningError> {
    let bytes = phrase.as_bytes();
    // Pack bytes into Belt values (8 bytes per Belt, little-endian)
    let mut belts: Vec<Belt> = Vec::with_capacity((bytes.len() + 7) / 8);
    for chunk in bytes.chunks(8) {
        let mut val: u64 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            val |= (b as u64) << (i * 8);
        }
        belts.push(Belt(val));
    }
    let hash = hash_varlen(&mut belts);
    // Zeroize: belts contains seed-derived key material (C-002)
    for b in belts.iter_mut() { b.0.zeroize(); }
    let scalar = SecretScalar(trunc_g_order(&hash));
    if *scalar == UBig::from(0u64) {
        return Err(SigningError::ZeroSeedScalar);
    }
    let result = ubig_to_belts8(&scalar);
    // scalar zeroized on drop (C-002)
    Ok(result)
}

// ---------------------------------------------------------------------------
// Sensitive scalar wrapper (C-002)
// ---------------------------------------------------------------------------

/// Wrapper for UBig values derived from secret key material.
///
/// Overwrites the value with zero on drop. UBig doesn't implement `Zeroize`
/// (orphan rule prevents external impl), so this is best-effort: it clears
/// the value but can't guarantee the allocator zeroes freed heap blocks.
/// Still better than the raw UBig pattern — at minimum prevents the value
/// from being readable after the wrapper is dropped.
///
/// # AUDIT 2026-04-17 L-06 — zeroize limitation
///
/// `self.0 = UBig::from(0u64)` clears the *logical* value, but:
///
/// - UBig's internal heap buffer is freed without zeroing (the
///   allocator may leave old key bits in memory that a later heap
///   allocation could read).
/// - If `UBig` has grown beyond its internal-small-int representation
///   and realloc'd during construction, the realloc source buffer is
///   also freed without zeroing.
/// - Stack-resident temporaries (e.g., during `trunc_g_order`) are
///   outside this wrapper and get cleaned up only by ordinary stack
///   reuse.
///
/// Production hardening would either store secrets in
/// `zeroize::Zeroizing<Vec<u8>>` (requires upstream UBig Zeroize
/// support) or swap to a constant-time bigint crate with explicit
/// zeroization. Until then, this wrapper is a nudge, not a guarantee.
struct SecretScalar(UBig);

impl Drop for SecretScalar {
    fn drop(&mut self) {
        // Overwrite with zero. UBig's internal buffer is freed on reassign.
        // The old heap allocation is released without zeroing (allocator limitation),
        // but the logical value is cleared.
        self.0 = UBig::from(0u64);
    }
}

impl std::ops::Deref for SecretScalar {
    type Target = UBig;
    fn deref(&self) -> &UBig { &self.0 }
}

impl std::ops::DerefMut for SecretScalar {
    fn deref_mut(&mut self) -> &mut UBig { &mut self.0 }
}

// ---------------------------------------------------------------------------
// Conversion helpers (UBig <-> [Belt; 8] in 32-bit chunks)
// ---------------------------------------------------------------------------

/// Reconstruct a UBig from 8 x 32-bit Belt chunks (little-endian).
///
/// Matches Hoon's `rep 5 sk-as-32-bit-belts`.
pub(crate) fn belts8_to_ubig(belts: &[Belt; 8]) -> UBig {
    // Build from raw bytes in one shot — no intermediate UBig allocations.
    let mut bytes = [0u8; 32];
    for (i, belt) in belts.iter().enumerate() {
        let chunk = (belt.0 as u32).to_le_bytes();
        bytes[i * 4..i * 4 + 4].copy_from_slice(&chunk);
    }
    let result = UBig::from_le_bytes(&bytes);
    bytes.zeroize();
    result
}

/// Like `belts8_to_ubig` but returns a SecretScalar that zeroizes on drop.
/// Use for secret key material only.
fn belts8_to_secret(belts: &[Belt; 8]) -> SecretScalar {
    SecretScalar(belts8_to_ubig(belts))
}

/// Split a UBig into 8 x 32-bit Belt chunks (little-endian).
///
/// Matches Hoon's `rip 5` with zero-padding to 8 elements.
pub(crate) fn ubig_to_belts8(val: &UBig) -> [Belt; 8] {
    let mut belts = [Belt(0); 8];
    let mut v = val.clone();
    let mask = UBig::from(0xFFFF_FFFFu64);
    for belt in &mut belts {
        let chunk = &v & &mask;
        // AUDIT 2026-04-19 L-18: chunk is `v & 0xFFFF_FFFF`, so it fits
        // in 32 bits (and therefore in u64) by construction. A silent
        // `unwrap_or(0)` would mask any invariant break and zero the
        // limb, producing invalid key material. Prefer a named
        // expect so the crash localizes the problem.
        *belt = Belt(u64::try_from(&chunk).expect("chunk is 32-bit by construction"));
        v >>= 32;
    }
    belts
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nockchain_math::crypto::cheetah::{ch_add, ch_neg, F6_ZERO};

    #[test]
    fn derive_pubkey_from_nonzero_key() {
        let mut sk = [Belt(0); 8];
        sk[0] = Belt(42);
        let pk = derive_pubkey(&sk);
        assert!(!pk.0.inf);
        assert_ne!(pk.0.x, F6_ZERO);
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let mut sk = [Belt(0); 8];
        sk[0] = Belt(12345);
        sk[1] = Belt(67890);

        let message = [Belt(1), Belt(2), Belt(3), Belt(4), Belt(5)];

        let sig = sign(&sk, &message).expect("signing should succeed");

        let pubkey = derive_pubkey(&sk);
        let chal_big = belts8_to_ubig(&sig.chal);
        let sig_big = belts8_to_ubig(&sig.sig);

        let left = ch_scal_big(&sig_big, &A_GEN).expect("valid sig scalar");
        let right = ch_neg(&ch_scal_big(&chal_big, &pubkey.0).expect("valid chal scalar"));
        let r_reconstructed = ch_add(&left, &right).expect("valid point add");
        assert_ne!(r_reconstructed.x, F6_ZERO, "R must not be at infinity");

        let mut hashable: Vec<Belt> = Vec::with_capacity(6 * 4 + 5);
        hashable.extend_from_slice(&r_reconstructed.x.0);
        hashable.extend_from_slice(&r_reconstructed.y.0);
        hashable.extend_from_slice(&pubkey.0.x.0);
        hashable.extend_from_slice(&pubkey.0.y.0);
        hashable.extend_from_slice(&message);
        let recomputed_hash = hash_varlen(&mut hashable);
        let recomputed_chal = trunc_g_order(&recomputed_hash);

        assert_eq!(
            recomputed_chal, chal_big,
            "recomputed challenge must match signed challenge"
        );
    }

    #[test]
    fn belts_roundtrip() {
        let original = UBig::from(0xDEADBEEF_CAFEBABE_u64);
        let belts = ubig_to_belts8(&original);
        let recovered = belts8_to_ubig(&belts);
        assert_eq!(original, recovered);
    }

    #[test]
    fn belts_roundtrip_large() {
        let mut val = UBig::from(1u64);
        for _ in 0..7 {
            val <<= 32;
            val += UBig::from(0xAAAA_BBBBu64);
        }
        let belts = ubig_to_belts8(&val);
        let recovered = belts8_to_ubig(&belts);
        assert_eq!(val, recovered);
    }

    #[test]
    fn pubkey_hash_produces_valid_hash() {
        let mut sk = [Belt(0); 8];
        sk[0] = Belt(999);
        let pk = derive_pubkey(&sk);
        let pkh = pubkey_hash(&pk);
        assert!(pkh.0.iter().any(|b| b.0 != 0));
    }

    #[test]
    fn different_messages_produce_different_signatures() {
        let mut sk = [Belt(0); 8];
        sk[0] = Belt(7777);

        let msg1 = [Belt(1), Belt(0), Belt(0), Belt(0), Belt(0)];
        let msg2 = [Belt(2), Belt(0), Belt(0), Belt(0), Belt(0)];

        let sig1 = sign(&sk, &msg1).expect("signing should succeed");
        let sig2 = sign(&sk, &msg2).expect("signing should succeed");

        assert_ne!(sig1.chal, sig2.chal);
        assert_ne!(sig1.sig, sig2.sig);
    }

    #[test]
    fn key_from_seed_phrase_produces_valid_key() {
        let sk = key_from_seed_phrase("test seed phrase for key derivation")
            .expect("key derivation should succeed");
        // Key should be non-zero
        assert!(sk.iter().any(|b| b.0 != 0));
        // Different phrases should produce different keys
        let sk2 = key_from_seed_phrase("a completely different seed phrase")
            .expect("key derivation should succeed");
        assert_ne!(sk, sk2);
    }
}
