//! Ed25519 signatures of the release files. Anyone with write access on the
//! repository can create or edit a GitHub release by hand (GitHub has no
//! separate right for it), so the `.sha256` file of a release proves nothing
//! on its own. The publish jobs of the Release workflow sign each package's
//! `.sha256` file (it names the package, and so its version) and the update
//! policy, with a key that only the workflows of `main` and of the stable
//! tags can read (the `release` environment, docs/UPDATES.md, "Signed
//! updates"). The viewer announces a release only once these signatures
//! check out against a key compiled in here (`feed::Source::check`).
//!
//! The Legacy Viewer has no counterpart: its updates come from its own
//! server, over HTTPS.
//!
//! A signature is the 64 raw bytes of Ed25519 over the whole file (what
//! `openssl pkeyutl -sign -rawin` writes, `scripts/sign-release.ps1`). The
//! file `<file>.sig` beside it holds one per signing key, one after the
//! other: while the key changes, a release is signed with the old and the
//! new one, and each viewer checks the one it knows.

use anyhow::{Result, anyhow, ensure};
use ring::signature::{ED25519, UnparsedPublicKey};

/// An Ed25519 public key.
pub type Key = [u8; 32];

/// The keys a published release may be signed with, in hex (the last 32
/// bytes of the key's DER form), each with the secret that holds it.
/// `scripts/sign-release.ps1` refuses a signing key that is not listed here
/// before anything is published, rather than publish a release no viewer
/// accepts; `scripts/setup-release-signing.ps1` adds a new one
/// (docs/UPDATES.md, "Changing the key").
pub const RELEASE_KEYS: &[&str] = &[
    // release-keys-begin
    // UPDATE_SIGNING_KEY_NEXT, 2026-11-23
    "4eaa4034d0643e00a0d9daeddfb6f1b6f683a509a03c526c3673e71ee5c0034d",
    // release-keys-end
];

/// What a signature file's name adds to the signed file's.
pub const SUFFIX: &str = ".sig";

/// The size of an Ed25519 signature.
pub const LEN: usize = 64;

/// The most signatures a `.sig` file holds: two while the key changes.
pub const MAX: usize = 4;

/// A key written in hex: 64 characters.
pub fn parse_key(hex: &str) -> Result<Key> {
    crate::package::parse_sha256(hex.trim()).map_err(|_| anyhow!("invalid public key"))
}

/// The keys of [`RELEASE_KEYS`]. One that does not parse is left out (a
/// test refuses it before any build ships).
pub fn release_keys() -> Vec<Key> {
    RELEASE_KEYS.iter().filter_map(|hex| parse_key(hex).ok()).collect()
}

/// Whether one of the signatures of the `.sig` file `signatures` is one of
/// `keys`' over the whole of `message`.
pub fn verify(keys: &[Key], message: &[u8], signatures: &[u8]) -> Result<()> {
    let (each, rest) = signatures.as_chunks::<LEN>();
    ensure!(
        rest.is_empty() && (1..=MAX).contains(&each.len()),
        "signature file of {} bytes",
        signatures.len()
    );
    let signed_by = |key: &Key, signature: &[u8; LEN]| UnparsedPublicKey::new(&ED25519, key).verify(message, signature).is_ok();
    ensure!(
        each.iter().any(|signature| keys.iter().any(|key| signed_by(key, signature))),
        "signature matches no release key"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn pair(seed: u8) -> (Ed25519KeyPair, Key) {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).expect("seed");
        let key = pair.public_key().as_ref().try_into().expect("32 bytes");
        (pair, key)
    }

    #[test]
    fn the_release_keys_are_valid_and_distinct() {
        assert!(!RELEASE_KEYS.is_empty(), "a release needs a key to be signed with");
        let keys = release_keys();
        assert_eq!(keys.len(), RELEASE_KEYS.len(), "a release key does not parse");
        for (i, key) in keys.iter().enumerate() {
            assert!(!keys[..i].contains(key), "release key listed twice");
        }
    }

    #[test]
    fn only_a_listed_key_over_the_whole_file_verifies() {
        let (signer, key) = pair(1);
        let (other, other_key) = pair(2);
        let file = b"0123  Aurora-Viewer-v0.9.0-windows-x64.zip\n";
        let signature = signer.sign(file);
        assert!(verify(&[key], file, signature.as_ref()).is_ok());
        // any of the listed keys: the old and the new one while it changes
        assert!(verify(&[other_key, key], file, signature.as_ref()).is_ok());
        // another key, no key, another file, a changed byte
        assert!(verify(&[other_key], file, signature.as_ref()).is_err());
        assert!(verify(&[], file, signature.as_ref()).is_err());
        assert!(verify(&[key], b"0123  Aurora-Viewer-v0.9.1-windows-x64.zip\n", signature.as_ref()).is_err());
        let mut changed = signature.as_ref().to_vec();
        changed[10] ^= 1;
        assert!(verify(&[key], file, &changed).is_err());
        // a signature by the other key, a truncated or padded one, nothing
        assert!(verify(&[key], file, other.sign(file).as_ref()).is_err());
        assert!(verify(&[key], file, &signature.as_ref()[..63]).is_err());
        assert!(verify(&[key], file, &[signature.as_ref(), b"\n"].concat()).is_err());
        assert!(verify(&[key], file, b"").is_err());
        // signed with the old and the new key: each viewer knows one of them
        let both = [other.sign(file).as_ref(), signature.as_ref()].concat();
        assert!(verify(&[key], file, &both).is_ok());
        assert!(verify(&[other_key], file, &both).is_ok());
        assert!(verify(&[key], file, &[other.sign(file).as_ref(), &[0; LEN]].concat()).is_err());
        // no more than `MAX` of them
        let padded = |zeros: usize| [vec![0; LEN * zeros], signature.as_ref().to_vec()].concat();
        assert!(verify(&[key], file, &padded(MAX - 1)).is_ok());
        assert!(verify(&[key], file, &padded(MAX)).is_err());
    }

    #[test]
    fn a_key_is_written_in_hex() {
        let (_, key) = pair(3);
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(parse_key(&hex).expect("key"), key);
        assert_eq!(parse_key(&format!("{}\n", hex.to_uppercase())).expect("key"), key);
        for bad in ["", "00", &hex[..62], &format!("{hex}00"), &hex.replace(&hex[..2], "zz")] {
            assert!(parse_key(bad).is_err(), "{bad}");
        }
    }
}
