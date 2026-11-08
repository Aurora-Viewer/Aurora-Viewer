//! The file format of the disk cache store (`cache/store`): the assets the
//! viewer keeps on disk (textures, meshes, materials, animations, sounds,
//! objects, inventory), AES-256-GCM under a key of the installation, which
//! the viewer draws at random once and keeps in the system's credential
//! store (`aurora-viewer/src/cache_key.rs`).
//!
//! Layout of a file: [`MAGIC`], 16 random bytes of salt and the plain
//! length (u64, little endian) make the header, which every chunk
//! authenticates; then the plain bytes in chunks of [`CHUNK`], each followed
//! by its 16-byte tag (at least one chunk: an empty file still has its tag).
//! Each file has its own key, HKDF-SHA256 of the installation key and its
//! salt, so a chunk's nonce can simply be its index without ever repeating
//! under a key. The chunks let a reader decrypt only what it reads
//! ([`Reader`]: the transcoded texture cache reads the coarse levels at the
//! end of a file).
//!
//! The store reads only its own format: [`Key::open`] and [`Reader::new`]
//! give an error for a file without the magic, a cache miss like a damaged
//! one. The `_or_plain` readers take such a file as it is, for the callers
//! whose sources are plain files (the demo's, the files tests write by
//! hand).
//!
//! `ring` does the AES (hardware AES and carry-less multiply): about 13 GB/s
//! on one core, some 12 µs for a texture of the cache.
//!
//! The store's file names derive from the installation key too
//! ([`Names`]): an HMAC-SHA256 of the asset id, under a key of its own
//! (HKDF of the installation key), so that a name says nothing of the asset
//! without the key.

use ring::aead::{self, Aad, LessSafeKey, Nonce};
use ring::rand::{SecureRandom, SystemRandom};
use ring::{hkdf, hmac};
use std::io::{self, Read, Seek, SeekFrom};

pub const MAGIC: &[u8; 4] = b"AUSL";
const SALT: usize = 16;
/// Magic, salt, plain length.
pub const HEADER: usize = 4 + SALT + 8;
/// Plain bytes per chunk.
pub const CHUNK: usize = 64 << 10;
const TAG: usize = 16;
const INFO: &[u8] = b"aurora cache file v1";
const NAMES_SALT: &[u8] = b"aurora cache names";
const NAMES_INFO: &[u8] = b"aurora cache names v1";
/// Hex digits of a store file name (128 bits of the HMAC).
pub const NAME_LEN: usize = 32;

/// The installation key. Its bytes never reach a log (no `Debug` of them).
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("sealed cache file: {what}"))
}

fn chunks(len: u64) -> u64 {
    len.div_ceil(CHUNK as u64).max(1)
}

/// The size on disk of a sealed file of `len` plain bytes.
pub fn sealed_len(len: u64) -> u64 {
    HEADER as u64 + len + chunks(len) * TAG as u64
}

fn nonce(chunk: u64) -> Nonce {
    let mut n = [0u8; aead::NONCE_LEN];
    n[4..].copy_from_slice(&chunk.to_be_bytes());
    Nonce::assume_unique_for_key(n)
}

/// Whether these bytes start like a sealed file.
pub fn is_sealed(head: &[u8]) -> bool {
    head.len() >= HEADER && head.starts_with(MAGIC)
}

fn plain_len(header: &[u8; HEADER]) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&header[4 + SALT..]);
    u64::from_le_bytes(b)
}

impl Key {
    pub fn from_bytes(bytes: [u8; 32]) -> Key {
        Key(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// A new key from the system's random source (None if it fails).
    pub fn random() -> Option<Key> {
        let mut k = [0u8; 32];
        SystemRandom::new().fill(&mut k).ok()?;
        Some(Key(k))
    }

    fn file_key(&self, header: &[u8; HEADER]) -> io::Result<LessSafeKey> {
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &header[4..4 + SALT]).extract(&self.0);
        let okm = prk
            .expand(&[INFO], &aead::AES_256_GCM)
            .map_err(|_| io::Error::other("sealed cache file: key derivation"))?;
        Ok(LessSafeKey::new(aead::UnboundKey::from(okm)))
    }

    /// The key of the store's file names.
    pub fn names(&self) -> Names {
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, NAMES_SALT).extract(&self.0);
        // (HKDF refuses only an output longer than 255 hashes: this one is one)
        let okm = prk.expand(&[NAMES_INFO], hmac::HMAC_SHA256).expect("one SHA-256 output");
        Names(hmac::Key::from(okm))
    }

    /// The sealed file of `plain`.
    pub fn seal(&self, plain: &[u8]) -> io::Result<Vec<u8>> {
        let mut header = [0u8; HEADER];
        header[..4].copy_from_slice(MAGIC);
        SystemRandom::new()
            .fill(&mut header[4..4 + SALT])
            .map_err(|_| io::Error::other("sealed cache file: no random source"))?;
        header[4 + SALT..].copy_from_slice(&(plain.len() as u64).to_le_bytes());
        let key = self.file_key(&header)?;
        let mut out = Vec::with_capacity(sealed_len(plain.len() as u64) as usize);
        out.extend_from_slice(&header);
        let mut parts: Vec<&[u8]> = plain.chunks(CHUNK).collect();
        if parts.is_empty() {
            parts.push(&[]);
        }
        for (i, part) in parts.into_iter().enumerate() {
            let start = out.len();
            out.extend_from_slice(part);
            let tag = key
                .seal_in_place_separate_tag(nonce(i as u64), Aad::from(&header), &mut out[start..])
                .map_err(|_| io::Error::other("sealed cache file: encryption"))?;
            out.extend_from_slice(tag.as_ref());
        }
        Ok(out)
    }

    /// The plain bytes of a sealed file read whole, decrypted in place. An
    /// error for a file not in this format, a damaged one, or one sealed
    /// under another key (the store was reset): a cache miss.
    pub fn open(&self, file: Vec<u8>) -> io::Result<Vec<u8>> {
        if !is_sealed(&file) {
            return Err(invalid("not in the store format"));
        }
        self.open_sealed(file)
    }

    /// Like [`Key::open`], but a file without the magic is read as it is.
    pub fn open_or_plain(&self, file: Vec<u8>) -> io::Result<Vec<u8>> {
        if !is_sealed(&file) {
            return Ok(file);
        }
        self.open_sealed(file)
    }

    fn open_sealed(&self, mut file: Vec<u8>) -> io::Result<Vec<u8>> {
        let mut header = [0u8; HEADER];
        header.copy_from_slice(&file[..HEADER]);
        let len = plain_len(&header);
        if file.len() as u64 != sealed_len(len) {
            return Err(invalid("size"));
        }
        let key = self.file_key(&header)?;
        let (mut read, mut write) = (HEADER, 0);
        for i in 0..chunks(len) {
            let n = (len - i * CHUNK as u64).min(CHUNK as u64) as usize;
            key.open_in_place(nonce(i), Aad::from(&header), &mut file[read..read + n + TAG])
                .map_err(|_| invalid("authentication"))?;
            // (the plain bytes move down over the header and the tags)
            file.copy_within(read..read + n, write);
            read += n + TAG;
            write += n;
        }
        file.truncate(write);
        Ok(file)
    }
}

/// The file names of the store ([`Key::names`]).
pub struct Names(hmac::Key);

impl std::fmt::Debug for Names {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Names(..)")
    }
}

impl Names {
    /// The stem of the file of `id`: [`NAME_LEN`] lowercase hex digits.
    pub fn name(&self, id: &uuid::Uuid) -> String {
        let tag = hmac::sign(&self.0, id.as_bytes());
        let mut out = String::with_capacity(NAME_LEN);
        for b in &tag.as_ref()[..NAME_LEN / 2] {
            out.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(b & 15), 16).unwrap_or('0'));
        }
        out
    }
}

/// Random access to the plain bytes of a cache file: only the chunks read
/// are decrypted.
pub struct Reader<R> {
    inner: R,
    sealed: Option<Sealed>,
    len: u64,
    pos: u64,
}

struct Sealed {
    key: LessSafeKey,
    header: [u8; HEADER],
    /// The chunk held in `plain`.
    held: Option<u64>,
    plain: Vec<u8>,
}

impl<R: Read + Seek> Reader<R> {
    /// A reader of a sealed file; an error for a file not in this format
    /// (as [`Key::open`]).
    pub fn new(inner: R, key: &Key) -> io::Result<Self> {
        Self::with(inner, key, false)
    }

    /// Like [`Reader::new`], but a file without the magic is read as it is.
    pub fn new_or_plain(inner: R, key: &Key) -> io::Result<Self> {
        Self::with(inner, key, true)
    }

    fn with(mut inner: R, key: &Key, plain: bool) -> io::Result<Self> {
        let total = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(0))?;
        let mut sealed = None;
        let mut len = total;
        if total >= HEADER as u64 {
            let mut header = [0u8; HEADER];
            inner.read_exact(&mut header)?;
            if is_sealed(&header) {
                len = plain_len(&header);
                if total != sealed_len(len) {
                    return Err(invalid("size"));
                }
                let key = key.file_key(&header)?;
                sealed = Some(Sealed {
                    key,
                    header,
                    held: None,
                    plain: Vec::new(),
                });
            } else {
                inner.seek(SeekFrom::Start(0))?;
            }
        }
        if sealed.is_none() && !plain {
            return Err(invalid("not in the store format"));
        }
        Ok(Reader {
            inner,
            sealed,
            len,
            pos: 0,
        })
    }

    /// Plain length.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<R: Read + Seek> Read for Reader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let Some(s) = &mut self.sealed else {
            let n = self.inner.read(out)?;
            self.pos += n as u64;
            return Ok(n);
        };
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        let chunk = self.pos / CHUNK as u64;
        if s.held != Some(chunk) {
            let n = (self.len - chunk * CHUNK as u64).min(CHUNK as u64) as usize;
            s.plain.resize(n + TAG, 0);
            self.inner.seek(SeekFrom::Start(HEADER as u64 + chunk * (CHUNK + TAG) as u64))?;
            self.inner.read_exact(&mut s.plain)?;
            s.held = None;
            s.key
                .open_in_place(nonce(chunk), Aad::from(&s.header), &mut s.plain)
                .map_err(|_| invalid("authentication"))?;
            s.plain.truncate(n);
            s.held = Some(chunk);
        }
        let at = (self.pos - chunk * CHUNK as u64) as usize;
        let n = (s.plain.len() - at).min(out.len());
        out[..n].copy_from_slice(&s.plain[at..at + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Reader<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        if self.sealed.is_none() {
            self.inner.seek(SeekFrom::Start(pos))?;
        }
        self.pos = pos;
        Ok(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 + i / 7) as u8).collect()
    }

    #[test]
    fn round_trip_at_chunk_edges() {
        let key = Key::random().expect("random");
        for n in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK + 17] {
            let plain = bytes(n);
            let sealed = key.seal(&plain).expect("seal");
            assert_eq!(sealed.len() as u64, sealed_len(n as u64));
            assert!(is_sealed(&sealed));
            assert_eq!(key.open(sealed).expect("open"), plain, "{n} bytes");
        }
    }

    #[test]
    fn hides_the_plain_bytes() {
        let key = Key::random().expect("random");
        // an Ogg page, a JPEG 2000 codestream start: none survive sealing
        let plain = [b"OggS".as_slice(), &[0xff, 0x4f, 0xff, 0x51], &bytes(1000)].concat();
        let sealed = key.seal(&plain).expect("seal");
        assert!(!sealed.windows(4).any(|w| w == b"OggS" || w == [0xff, 0x4f, 0xff, 0x51]));
        // two seals of the same bytes differ (each file has its salt)
        assert_ne!(key.seal(&plain).expect("seal"), sealed);
    }

    #[test]
    fn another_key_or_a_changed_byte_is_refused() {
        let key = Key::random().expect("random");
        let sealed = key.seal(&bytes(CHUNK + 5)).expect("seal");
        assert!(Key::random().expect("random").open(sealed.clone()).is_err());
        for at in [5, HEADER + 3, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[at] ^= 1;
            assert!(key.open(bad).is_err(), "byte {at} changed");
        }
        let mut cut = sealed.clone();
        cut.pop();
        assert!(key.open(cut).is_err(), "truncated");
    }

    #[test]
    fn the_store_reads_only_its_own_format() {
        let key = Key::random().expect("random");
        // short, long, and long with a magic of another format
        for file in [bytes(10), bytes(200), [b"AUSX".as_slice(), &bytes(200)].concat()] {
            assert!(key.open(file.clone()).is_err(), "{} bytes", file.len());
            assert!(Reader::new(Cursor::new(file.clone()), &key).is_err(), "{} bytes", file.len());
        }
        // a sealed file still reads through both
        let sealed = key.seal(&bytes(200)).expect("seal");
        assert_eq!(key.open(sealed.clone()).expect("open"), bytes(200));
        assert_eq!(Reader::new(Cursor::new(sealed), &key).expect("reader").len(), 200);
    }

    #[test]
    fn plain_files_read_as_they_are_when_asked() {
        let key = Key::random().expect("random");
        let plain = bytes(200);
        assert_eq!(key.open_or_plain(plain.clone()).expect("plain"), plain);
        let sealed = key.seal(&plain).expect("seal");
        assert_eq!(key.open_or_plain(sealed).expect("sealed"), plain);
        let mut r = Reader::new_or_plain(Cursor::new(plain.clone()), &key).expect("reader");
        assert_eq!(r.len(), 200);
        r.seek(SeekFrom::Start(150)).expect("seek");
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).expect("read");
        assert_eq!(tail, plain[150..]);
    }

    #[test]
    fn names_are_stable_per_key_and_say_nothing_of_the_id() {
        let key = Key::random().expect("random");
        let names = key.names();
        let id = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_0fed_cba9_8765_4321);
        let name = names.name(&id);
        assert_eq!(name.len(), NAME_LEN);
        assert!(name.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(key.names().name(&id), name, "same key, same name");
        assert_ne!(Key::random().expect("random").names().name(&id), name, "another key");
        assert_ne!(names.name(&uuid::Uuid::from_u128(1)), name, "another id");
        assert!(!name.contains(&id.simple().to_string()[..8]));
    }

    #[test]
    fn reader_seeks_across_chunks() {
        let key = Key::random().expect("random");
        let plain = bytes(3 * CHUNK + 100);
        let mut r = Reader::new(Cursor::new(key.seal(&plain).expect("seal")), &key).expect("reader");
        assert_eq!(r.len(), plain.len() as u64);
        let mut head = [0u8; 10];
        r.read_exact(&mut head).expect("head");
        assert_eq!(head, plain[..10]);
        // a span over a chunk boundary, then the end, then back
        for start in [CHUNK - 5, 3 * CHUNK + 50, 7] {
            r.seek(SeekFrom::Start(start as u64)).expect("seek");
            let mut got = vec![0u8; 50.min(plain.len() - start)];
            r.read_exact(&mut got).expect("span");
            assert_eq!(got, plain[start..start + got.len()]);
        }
        r.seek(SeekFrom::End(0)).expect("end");
        assert_eq!(r.read(&mut head).expect("eof"), 0);
    }
}
