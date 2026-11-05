//! Signed update manifest. Each release carries `aurora-update-manifest.json`
//! (version, then the name, size and SHA-256 of the files the updater reads:
//! the Windows package and the update policy) and its Ed25519 signature,
//! `aurora-update-manifest.json.sig`. The Release workflow signs it with a
//! key that only its protected environment holds (docs/UPDATES.md, "Update
//! signature"); the viewer installs a package, and applies a policy, only
//! when the manifest verifies against one of the keys compiled in below and
//! names those exact bytes. The `.sha256` files beside the archives stay for
//! people checking a download by hand: the updater no longer reads them.

use anyhow::{Context, Result, ensure};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Release asset holding the manifest.
pub const MANIFEST: &str = "aurora-update-manifest.json";
/// Release asset holding the signature of the manifest's bytes, in hex.
pub const SIGNATURE: &str = "aurora-update-manifest.json.sig";
/// Environment variable the signing tool reads the private key from.
pub const KEY_VARIABLE: &str = "UPDATE_SIGNING_KEY";

/// The public keys a release may be signed with. Any one of them is enough,
/// so a new key is added in a release still signed with the old one, which
/// is removed later.
///
/// TODO(maintainer): empty until the release key exists. Run
/// `cargo run -p aurora-updater --bin aurora-update-sign -- keygen --out <file>`
/// and paste the `key("…")` line it prints here (docs/UPDATES.md, "Update
/// signature"). While the list is empty, every update is refused and a
/// build with `AURORA_VERSION_TAG` (the Release workflow) does not compile.
pub const UPDATE_KEYS: &[[u8; 32]] = &[];

// A published build without a key could install no update at all: the
// Release workflow (which sets AURORA_VERSION_TAG) stops here instead.
// Development builds and CI, without a tag, compile and refuse updates.
const _: () = assert!(
    option_env!("AURORA_VERSION_TAG").is_none() || !UPDATE_KEYS.is_empty(),
    "aurora-updater: UPDATE_KEYS is empty; add the release public key (docs/UPDATES.md, \"Update signature\")"
);

/// A public key written as 64 hex digits; a typo fails the build.
pub const fn key(hex: &str) -> [u8; 32] {
    const fn digit(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => panic!("update key: not a hex digit"),
        }
    }
    let bytes = hex.as_bytes();
    assert!(bytes.len() == 64, "update key: 64 hex digits expected");
    let mut out = [0_u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (digit(bytes[2 * i]) << 4) | digit(bytes[2 * i + 1]);
        i += 1;
    }
    out
}

/// An update whose signature or contents do not hold. The viewer reports it
/// as an integrity failure, not as a network one.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "update refused: {}", self.0)
    }
}

impl std::error::Error for Refused {}

/// Logged by the caller with the rest of the check's error.
pub(crate) fn refused(reason: impl Into<String>) -> anyhow::Error {
    Refused(reason.into()).into()
}

/// Whether an error, or one of its causes, is a refusal.
pub fn is_refused(error: &anyhow::Error) -> bool {
    error.chain().any(|e| e.is::<Refused>())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// The release version without its `v`.
    pub version: String,
    pub assets: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

impl Entry {
    pub fn of(name: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
            size: bytes.len() as u64,
            sha256: hex(&Sha256::digest(bytes)),
        }
    }
    pub fn digest(&self) -> Result<[u8; 32]> {
        crate::package::parse_sha256(&self.sha256)
    }
    /// The bytes are exactly the ones this entry names.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        self.size == bytes.len() as u64 && self.digest().is_ok_and(|d| d == Sha256::digest(bytes).as_slice())
    }
}

impl Manifest {
    pub fn new(version: &str, assets: Vec<Entry>) -> Self {
        Self {
            schema: 1,
            version: version.into(),
            assets,
        }
    }
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.assets.iter().find(|e| e.name == name)
    }
    /// The bytes that are signed and published.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).expect("manifest serializes");
        bytes.push(b'\n');
        bytes
    }
}

/// Checks the signature over the manifest's exact bytes against `keys`,
/// then reads it.
pub fn verify(manifest: &[u8], signature: &[u8], keys: &[[u8; 32]]) -> Result<Manifest> {
    if keys.is_empty() {
        return Err(refused("this build holds no update key"));
    }
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(|text| unhex(text.trim()).ok())
        .filter(|s| s.len() == 64)
        .ok_or_else(|| refused("unreadable manifest signature"))?;
    if !keys
        .iter()
        .any(|key| UnparsedPublicKey::new(&ED25519, key).verify(manifest, &signature).is_ok())
    {
        return Err(refused("the manifest signature does not match any update key"));
    }
    let manifest: Manifest = serde_json::from_slice(manifest).map_err(|e| refused(format!("unreadable manifest: {e}")))?;
    if manifest.schema != 1 {
        return Err(refused(format!("manifest schema {}", manifest.schema)));
    }
    Ok(manifest)
}

/// A new private key, as the PKCS#8 document stored in the release secret.
pub fn generate() -> Result<Vec<u8>> {
    let rng = ring::rand::SystemRandom::new();
    let document = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| anyhow::anyhow!("cannot generate a key"))?;
    Ok(document.as_ref().to_vec())
}

fn pair(pkcs8: &[u8]) -> Result<Ed25519KeyPair> {
    Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|e| anyhow::anyhow!("invalid signing key: {e}"))
}

pub fn public_key(pkcs8: &[u8]) -> Result<[u8; 32]> {
    let pair = pair(pkcs8)?;
    pair.public_key().as_ref().try_into().context("Ed25519 public key size")
}

/// The signature file's contents for `manifest`.
pub fn sign(manifest: &[u8], pkcs8: &[u8]) -> Result<String> {
    Ok(format!("{}\n", hex(pair(pkcs8)?.sign(manifest).as_ref())))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Result<Vec<u8>> {
    ensure!(
        text.len().is_multiple_of(2) && text.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid hex"
    );
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).context("invalid hex"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(manifest: &Manifest) -> (Vec<u8>, Vec<u8>, [u8; 32]) {
        let pkcs8 = generate().expect("test key");
        let bytes = manifest.to_bytes();
        let signature = sign(&bytes, &pkcs8).expect("signature").into_bytes();
        (bytes, signature, public_key(&pkcs8).expect("public key"))
    }

    #[test]
    fn a_signed_manifest_reads_back_and_names_its_bytes() {
        let manifest = Manifest::new("0.3.0", vec![Entry::of("a.zip", b"package"), Entry::of("p.json", b"{}")]);
        let (bytes, signature, public) = signed(&manifest);
        let other = public_key(&generate().expect("key")).expect("public key");
        let read = verify(&bytes, &signature, &[other, public]).expect("any listed key verifies");
        assert_eq!(read, manifest);
        let entry = read.entry("a.zip").expect("entry");
        assert!(entry.matches(b"package"));
        assert!(!entry.matches(b"packagf") && !entry.matches(b"package!"));
    }

    #[test]
    fn a_wrong_key_an_altered_manifest_or_a_bad_signature_is_refused() {
        let manifest = Manifest::new("0.3.0", vec![Entry::of("a.zip", b"package")]);
        let (bytes, signature, public) = signed(&manifest);
        let stranger = public_key(&generate().expect("key")).expect("public key");
        let refusal = verify(&bytes, &signature, &[stranger]).expect_err("wrong key");
        assert!(is_refused(&refusal), "{refusal:#}");
        let mut altered = bytes.clone();
        let at = altered.iter().position(|&b| b == b'3').expect("version digit");
        altered[at] = b'4';
        assert!(verify(&altered, &signature, &[public]).is_err());
        for bad in [&b""[..], b"zz", &signature[..100]] {
            assert!(verify(&bytes, bad, &[public]).is_err());
        }
        assert!(verify(&bytes, &signature, &[]).is_err(), "no key, no update");
    }

    #[test]
    fn keys_are_written_as_hex_and_the_tool_line_round_trips() {
        let pkcs8 = generate().expect("key");
        let public = public_key(&pkcs8).expect("public key");
        assert_eq!(unhex(&hex(&public)).expect("hex"), public);
        assert_eq!(
            key("00ff10AB00000000000000000000000000000000000000000000000000000001")[..4],
            [0, 255, 16, 171]
        );
        assert!(unhex("abc").is_err() && unhex("zz").is_err());
        // The private key round-trips through its hex secret.
        assert_eq!(public_key(&unhex(&hex(&pkcs8)).expect("hex")).expect("key"), public);
    }
}
