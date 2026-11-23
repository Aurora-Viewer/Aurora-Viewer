//! Detached Ed25519 signatures for `.ext` package contents.
//!
//! The signature covers a domain-separated package identity and the canonical
//! content digest. The signature entry itself is excluded from that digest to
//! avoid recursion; all other archive members, including the manifest, are
//! covered by the digest computed by the package validator.

use crate::Manifest;
use ring::{
    rand::SystemRandom,
    signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Optional root-level archive member containing an Ed25519 signature.
pub const SIGNATURE_MEMBER: &str = "signature.json";
/// Maximum encoded signature metadata size.
pub const MAX_SIGNATURE_BYTES: usize = 4096;
const SIGNATURE_DOMAIN: &[u8] = b"AURORA-EXTENSION-PACKAGE\0V1\0";

/// Versioned, self-contained signature envelope. `key_id` is the SHA-256
/// fingerprint of `public_key`; it is not a publisher's real-world identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureEnvelope {
    pub schema_version: u32,
    pub algorithm: String,
    pub key_id: String,
    /// Lowercase hex-encoded Ed25519 public key.
    pub public_key: String,
    /// Lowercase hex-encoded Ed25519 signature.
    pub signature: String,
}

/// A cryptographically verified signer. Trust is a separate user decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedSignature {
    pub key_id: String,
    pub public_key: [u8; 32],
}

/// Portable public-key file that users can import into their local trust store.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherKeyFile {
    pub schema_version: u32,
    pub algorithm: String,
    pub key_id: String,
    /// Lowercase hex-encoded Ed25519 public key.
    pub public_key: String,
}

/// Create a PKCS#8 Ed25519 private key. The caller is responsible for writing
/// it only to an appropriately protected local file or secret store.
pub fn generate_pkcs8() -> Result<Vec<u8>, SignatureError> {
    let document =
        Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| SignatureError("Ed25519 key generation failed".into()))?;
    Ok(document.as_ref().to_vec())
}

/// Return the public key and its stable lowercase SHA-256 fingerprint.
pub fn public_key_from_pkcs8(pkcs8: &[u8]) -> Result<VerifiedSignature, SignatureError> {
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| SignatureError("invalid Ed25519 PKCS#8 private key".into()))?;
    let public_key: [u8; 32] = pair
        .public_key()
        .as_ref()
        .try_into()
        .map_err(|_| SignatureError("invalid Ed25519 public key length".into()))?;
    Ok(VerifiedSignature {
        key_id: fingerprint(&public_key),
        public_key,
    })
}

/// Produce a deterministic JSON envelope for this validated package identity
/// and content digest. The private key never enters the package.
pub fn sign_manifest(manifest: &Manifest, content_sha256: &[u8; 32], pkcs8: &[u8]) -> Result<SignatureEnvelope, SignatureError> {
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| SignatureError("invalid Ed25519 PKCS#8 private key".into()))?;
    let public_key: [u8; 32] = pair
        .public_key()
        .as_ref()
        .try_into()
        .map_err(|_| SignatureError("invalid Ed25519 public key length".into()))?;
    let payload = signing_payload(manifest);
    let mut payload = payload;
    payload.extend_from_slice(content_sha256);
    let signature = pair.sign(&payload);
    Ok(SignatureEnvelope {
        schema_version: 1,
        algorithm: "ed25519".into(),
        key_id: fingerprint(&public_key),
        public_key: hex(&public_key),
        signature: hex(signature.as_ref()),
    })
}

/// Serialize an envelope as bounded UTF-8 JSON with a final newline.
pub fn encode_envelope(envelope: &SignatureEnvelope) -> Result<Vec<u8>, SignatureError> {
    let mut bytes = serde_json::to_vec_pretty(envelope).map_err(|_| SignatureError("signature envelope serialization failed".into()))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(SignatureError("signature envelope exceeds byte limit".into()));
    }
    Ok(bytes)
}

/// Parse and cryptographically verify an envelope. A valid key is not trusted
/// until the user explicitly trusts its fingerprint in the viewer.
pub fn verify_envelope(manifest: &Manifest, content_sha256: &[u8; 32], bytes: &[u8]) -> Result<VerifiedSignature, SignatureError> {
    if bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(SignatureError("signature envelope exceeds byte limit".into()));
    }
    let envelope: SignatureEnvelope =
        serde_json::from_slice(bytes).map_err(|_| SignatureError("signature envelope is malformed or has unknown fields".into()))?;
    if envelope.schema_version != 1 || envelope.algorithm != "ed25519" {
        return Err(SignatureError("unsupported package signature version or algorithm".into()));
    }
    let public_key = decode_hex::<32>(&envelope.public_key)?;
    let expected_key_id = fingerprint(&public_key);
    if envelope.key_id != expected_key_id {
        return Err(SignatureError("signature key fingerprint does not match its public key".into()));
    }
    let signature = decode_hex::<64>(&envelope.signature)?;
    let mut payload = signing_payload(manifest);
    payload.extend_from_slice(content_sha256);
    UnparsedPublicKey::new(&ED25519, &public_key)
        .verify(&payload, &signature)
        .map_err(|_| SignatureError("package signature verification failed".into()))?;
    Ok(VerifiedSignature {
        key_id: expected_key_id,
        public_key,
    })
}

/// Convert a validated public key to its versioned portable trust-file format.
pub fn public_key_file(public_key: &[u8; 32]) -> PublisherKeyFile {
    PublisherKeyFile {
        schema_version: 1,
        algorithm: "ed25519".into(),
        key_id: fingerprint(public_key),
        public_key: hex(public_key),
    }
}

/// Parse a portable public-key file and verify its fingerprint before use.
pub fn parse_public_key_file(bytes: &[u8]) -> Result<VerifiedSignature, SignatureError> {
    if bytes.len() > MAX_SIGNATURE_BYTES {
        return Err(SignatureError("publisher key file exceeds byte limit".into()));
    }
    let file: PublisherKeyFile =
        serde_json::from_slice(bytes).map_err(|_| SignatureError("publisher key file is malformed or has unknown fields".into()))?;
    if file.schema_version != 1 || file.algorithm != "ed25519" {
        return Err(SignatureError("unsupported publisher key version or algorithm".into()));
    }
    let public_key = decode_hex::<32>(&file.public_key)?;
    let key_id = fingerprint(&public_key);
    if file.key_id != key_id {
        return Err(SignatureError("publisher key fingerprint does not match its public key".into()));
    }
    Ok(VerifiedSignature { key_id, public_key })
}

fn signing_payload(manifest: &Manifest) -> Vec<u8> {
    let mut payload = SIGNATURE_DOMAIN.to_vec();
    append_field(&mut payload, manifest.id.as_bytes());
    append_field(&mut payload, manifest.version.as_bytes());
    append_field(&mut payload, manifest.api.as_bytes());
    payload
}

fn append_field(payload: &mut Vec<u8>, field: &[u8]) {
    payload.extend_from_slice(&(field.len() as u64).to_le_bytes());
    payload.extend_from_slice(field);
}

fn fingerprint(public_key: &[u8; 32]) -> String {
    hex(&Sha256::digest(public_key))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], SignatureError> {
    if value.len() != N * 2 || value.bytes().any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase()) {
        return Err(SignatureError("signature key or value is not canonical lowercase hex".into()));
    }
    let mut output = [0u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
            .map_err(|_| SignatureError("signature key or value is not valid hex".into()))?;
    }
    Ok(output)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignatureError(pub String);

impl std::fmt::Display for SignatureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SignatureError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            manifest_version: 1,
            id: "dev.example.fixture".into(),
            name: "Fixture".into(),
            version: "1.2.3".into(),
            api: ">=1.0.0, <2.0.0".into(),
            entry: "module.wasm".into(),
            permissions: vec!["ui.panel.register".into()],
            network_domains: Vec::new(),
            publisher: "Example publisher".into(),
        }
    }

    #[test]
    fn a_signature_binds_extension_identity_version_manifest_and_content_digest() {
        let manifest = manifest();
        let content: [u8; 32] = Sha256::digest(b"fixture package bytes").into();
        let private = generate_pkcs8().expect("private key");
        let signer = public_key_from_pkcs8(&private).expect("public key");
        let envelope = sign_manifest(&manifest, &content, &private).expect("signature");
        let encoded = encode_envelope(&envelope).expect("encoded envelope");
        let verified = verify_envelope(&manifest, &content, &encoded).expect("verify signature");
        assert_eq!(verified, signer);

        let mut changed_manifest = manifest.clone();
        changed_manifest.id = "dev.example.other".into();
        assert!(verify_envelope(&changed_manifest, &content, &encoded).is_err());
        let changed_content: [u8; 32] = Sha256::digest(b"tampered bytes").into();
        assert!(verify_envelope(&manifest, &changed_content, &encoded).is_err());
    }

    #[test]
    fn malformed_or_noncanonical_signature_envelopes_are_rejected() {
        let manifest = manifest();
        let content: [u8; 32] = Sha256::digest(b"fixture package bytes").into();
        let private = generate_pkcs8().expect("private key");
        let envelope = sign_manifest(&manifest, &content, &private).expect("signature");
        let encoded = encode_envelope(&envelope).expect("encoded envelope");
        let mut parsed: serde_json::Value = serde_json::from_slice(&encoded).expect("JSON envelope");
        parsed["signature"] = "zz".into();
        let malformed = serde_json::to_vec(&parsed).expect("JSON bytes");
        assert!(verify_envelope(&manifest, &content, &malformed).is_err());
        assert!(verify_envelope(&manifest, &content, b"{}\n").is_err());
        assert!(verify_envelope(&manifest, &content, &vec![b'x'; MAX_SIGNATURE_BYTES + 1]).is_err());
    }

    #[test]
    fn key_files_are_bound_to_their_public_key_fingerprint() {
        let private = generate_pkcs8().expect("private key");
        let signer = public_key_from_pkcs8(&private).expect("public key");
        let bytes = serde_json::to_vec(&public_key_file(&signer.public_key)).expect("key JSON");
        assert_eq!(parse_public_key_file(&bytes).expect("valid publisher key"), signer);
        let mut changed = public_key_file(&signer.public_key);
        changed.key_id = "00".repeat(32);
        let bytes = serde_json::to_vec(&changed).expect("key JSON");
        assert!(parse_public_key_file(&bytes).is_err());
    }
}
