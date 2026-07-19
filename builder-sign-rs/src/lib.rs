//! builder-sign-rs — open-ot builder signing library.
//!
//! Per SPEC §9 build → sign → pin pipeline. Replaces the
//! `scripts/builder-sign.sh` stub referenced in `cells/CLAUDE.md`.
//!
//! Crypto:
//! - Hash: BLAKE3 (b3sum-compatible, 256-bit)
//! - Signature: Ed25519
//! - CID encoding: base58btc (compact, URL-safe-ish)
//! - Key encoding: hex

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH};
use std::fs;
use std::io::Read;
use std::path::Path;

/// 32-byte BLAKE3 content hash, encoded as base58btc for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct B3Cid {
    bytes: [u8; 32],
}

impl B3Cid {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// base58btc string with a leading 'z' indicator (multibase-compatible prefix).
    pub fn to_string(&self) -> String {
        let mut s = String::with_capacity(46);
        s.push('z');
        s.push_str(&bs58::encode(self.bytes).into_string());
        s
    }

    pub fn parse(s: &str) -> Result<Self> {
        let body = s
            .strip_prefix('z')
            .ok_or_else(|| anyhow!("CID must start with 'z' (base58btc multibase prefix)"))?;
        let raw = bs58::decode(body)
            .into_vec()
            .context("base58btc decode CID body")?;
        if raw.len() != 32 {
            bail!("CID must decode to 32 bytes, got {}", raw.len());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&raw);
        Ok(Self { bytes: arr })
    }
}

/// Hash a file's contents with BLAKE3.
pub fn hash_file(path: &Path) -> Result<B3Cid> {
    let mut hasher = blake3::Hasher::new();
    let mut f = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let h = hasher.finalize();
    Ok(B3Cid::from_bytes(*h.as_bytes()))
}

/// Hash an in-memory byte slice with BLAKE3.
pub fn hash_bytes(data: &[u8]) -> B3Cid {
    let h = blake3::hash(data);
    B3Cid::from_bytes(*h.as_bytes())
}

/// Ed25519 keypair wrapper. Use [`Keypair::generate`] for a fresh key, or
/// [`Keypair::from_secret_hex`] to load one from a hex-encoded file/string.
pub struct Keypair {
    signing: SigningKey,
}

impl Keypair {
    /// Generate a fresh Ed25519 keypair using OS RNG.
    pub fn generate() -> Self {
        use rand::rngs::OsRng;
        let signing = SigningKey::generate(&mut OsRng);
        Self { signing }
    }

    /// Load a keypair from a 32-byte secret key (hex-encoded).
    pub fn from_secret_hex(hex_str: &str) -> Result<Self> {
        let bytes = hex::decode(hex_str.trim()).context("decode secret key hex")?;
        if bytes.len() != SECRET_KEY_LENGTH {
            bail!(
                "secret key must be {} bytes, got {}",
                SECRET_KEY_LENGTH,
                bytes.len()
            );
        }
        let mut arr = [0u8; SECRET_KEY_LENGTH];
        arr.copy_from_slice(&bytes);
        let signing = SigningKey::from_bytes(&arr);
        Ok(Self { signing })
    }

    /// Hex-encode the 32-byte private key (for file storage; mode 0600).
    pub fn secret_hex(&self) -> String {
        hex::encode(self.signing.to_bytes())
    }

    /// Hex-encode the 32-byte public key.
    pub fn public_hex(&self) -> String {
        hex::encode(self.public_key().to_bytes())
    }

    pub fn public_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }
}

/// 64-byte Ed25519 signature.
pub struct Signature {
    sig: ed25519_dalek::Signature,
}

impl Signature {
    pub fn to_hex(&self) -> String {
        hex::encode(self.sig.to_bytes())
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s.trim()).context("decode signature hex")?;
        if bytes.len() != 64 {
            bail!("signature must be 64 bytes, got {}", bytes.len());
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&bytes);
        Ok(Self {
            sig: ed25519_dalek::Signature::from_bytes(&arr),
        })
    }
}

/// Sign a BLAKE3 hash with the builder's Ed25519 key. The on-chain
/// `pinModule` record stores both the hash and this signature; on-device
/// verification needs only the hash (plus the builder DID's pubkey from
/// atproto).
pub fn sign_hash(cid: &B3Cid, keypair: &Keypair) -> Signature {
    let sig = keypair.signing.sign(cid.as_bytes());
    Signature { sig }
}

/// Verify a signature against a hash and public key.
pub fn verify_hash(cid: &B3Cid, signature: &Signature, public_key: &VerifyingKey) -> Result<()> {
    public_key
        .verify(cid.as_bytes(), &signature.sig)
        .map_err(|e| anyhow!("Ed25519 verify failed: {}", e))
}

/// Parse a public key from hex.
pub fn parse_public_key_hex(s: &str) -> Result<VerifyingKey> {
    let bytes = hex::decode(s.trim()).context("decode public key hex")?;
    if bytes.len() != 32 {
        bail!("public key must be 32 bytes, got {}", bytes.len());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    VerifyingKey::from_bytes(&arr).context("parse Ed25519 public key")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_file(content: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn keygen_roundtrip() {
        let kp = Keypair::generate();
        let hex = kp.secret_hex();
        let kp2 = Keypair::from_secret_hex(&hex).unwrap();
        assert_eq!(kp.public_hex(), kp2.public_hex());
    }

    #[test]
    fn sign_verify_roundtrip() {
        let kp = Keypair::generate();
        let cid = hash_bytes(b"the quick brown fox");
        let sig = sign_hash(&cid, &kp);
        verify_hash(&cid, &sig, &kp.public_key()).unwrap();
    }

    #[test]
    fn sign_verify_tampered_payload() {
        let kp = Keypair::generate();
        let cid_orig = hash_bytes(b"the quick brown fox");
        let cid_tampered = hash_bytes(b"the quick brown FOX");
        let sig = sign_hash(&cid_orig, &kp);
        assert!(verify_hash(&cid_tampered, &sig, &kp.public_key()).is_err());
    }

    #[test]
    fn sign_verify_tampered_signature() {
        let kp = Keypair::generate();
        let cid = hash_bytes(b"the quick brown fox");
        let sig = sign_hash(&cid, &kp);
        let mut hex = sig.to_hex();
        // Flip a single byte in the middle of the signature.
        let mid = hex.len() / 2;
        let b = hex.as_bytes()[mid];
        let new = if b == b'0' { b'1' } else { b'0' };
        unsafe {
            hex.as_bytes_mut()[mid] = new;
        }
        let tampered = Signature::from_hex(&hex).unwrap();
        assert!(verify_hash(&cid, &tampered, &kp.public_key()).is_err());
    }

    #[test]
    fn sign_verify_wrong_pubkey() {
        let kp_a = Keypair::generate();
        let kp_b = Keypair::generate();
        let cid = hash_bytes(b"the quick brown fox");
        let sig = sign_hash(&cid, &kp_a);
        assert!(verify_hash(&cid, &sig, &kp_b.public_key()).is_err());
    }

    #[test]
    fn cid_known_vector() {
        // BLAKE3 of empty input — well-known reference value.
        let cid = hash_bytes(b"");
        assert_eq!(
            hex::encode(cid.as_bytes()),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn cid_deterministic() {
        let f = tmp_file(b"deterministic content");
        let a = hash_file(f.path()).unwrap();
        let b = hash_file(f.path()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn cid_distinct() {
        let f1 = tmp_file(b"content A");
        let f2 = tmp_file(b"content B");
        let a = hash_file(f1.path()).unwrap();
        let b = hash_file(f2.path()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn cid_base58_roundtrip() {
        let cid_a = hash_bytes(b"some content");
        let s = cid_a.to_string();
        assert!(s.starts_with('z'));
        let cid_b = B3Cid::parse(&s).unwrap();
        assert_eq!(cid_a, cid_b);
    }

    #[test]
    fn signature_serialise_roundtrip() {
        let kp = Keypair::generate();
        let cid = hash_bytes(b"payload");
        let sig = sign_hash(&cid, &kp);
        let hex = sig.to_hex();
        let sig2 = Signature::from_hex(&hex).unwrap();
        verify_hash(&cid, &sig2, &kp.public_key()).unwrap();
    }

    #[test]
    fn public_key_serialise_roundtrip() {
        let kp = Keypair::generate();
        let hex = kp.public_hex();
        let pk = parse_public_key_hex(&hex).unwrap();
        assert_eq!(pk.to_bytes(), kp.public_key().to_bytes());
    }
}
