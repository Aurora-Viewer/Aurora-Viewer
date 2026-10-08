//! Shoutcast / Icecast in-band ("ICY") metadata.
//!
//! When a client sends `Icy-MetaData: 1`, the server answers with an
//! `icy-metaint: N` header and inserts a metadata block after every `N` bytes
//! of audio: one length byte `L`, then `L * 16` bytes of text such as
//! `StreamTitle='Artist - Song';StreamUrl='';`, NUL padded.

use std::io::{self, Read};
use std::sync::Arc;

use parking_lot::Mutex;

/// Latest stream title (`None` = no title / cleared by the server).
pub(crate) type TitleSlot = Arc<Mutex<Option<String>>>;

/// A reader that strips ICY metadata blocks from the audio byte stream and
/// publishes `StreamTitle` updates into a [`TitleSlot`].
pub(crate) struct IcyReader<R> {
    inner: R,
    metaint: usize,
    remaining: usize,
    title: TitleSlot,
    meta_buf: Vec<u8>,
}

impl<R: Read> IcyReader<R> {
    pub(crate) fn new(inner: R, metaint: usize, title: TitleSlot) -> Self {
        Self {
            inner,
            metaint,
            remaining: metaint,
            title,
            meta_buf: Vec::new(),
        }
    }

    /// Reads one metadata block. Returns `false` on a clean end of stream.
    fn read_metadata(&mut self) -> io::Result<bool> {
        let mut len = [0u8; 1];
        loop {
            match self.inner.read(&mut len) {
                Ok(0) => return Ok(false),
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let size = usize::from(len[0]) * 16;
        if size > 0 {
            self.meta_buf.resize(size, 0);
            self.inner.read_exact(&mut self.meta_buf)?;
            if let Some(title) = parse_stream_title(&self.meta_buf) {
                *self.title.lock() = if title.is_empty() { None } else { Some(title) };
            }
        }
        Ok(true)
    }
}

impl<R: Read> Read for IcyReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.metaint == 0 {
            return self.inner.read(buf);
        }
        if self.remaining == 0 {
            if !self.read_metadata()? {
                return Ok(0);
            }
            self.remaining = self.metaint;
        }
        let n = buf.len().min(self.remaining);
        let got = self.inner.read(&mut buf[..n])?;
        self.remaining -= got;
        Ok(got)
    }
}

/// Extracts `StreamTitle` from a metadata block. Returns `None` if the block
/// has no `StreamTitle` key, `Some("")` if the title is empty.
pub(crate) fn parse_stream_title(block: &[u8]) -> Option<String> {
    let end = block.iter().rposition(|&b| b != 0).map(|p| p + 1).unwrap_or(0);
    let text = decode_text(&block[..end]);
    const KEY: &str = "StreamTitle='";
    let start = text.find(KEY)? + KEY.len();
    let rest = &text[start..];
    // Titles may contain apostrophes; the value ends at "';".
    let stop = rest.find("';").or_else(|| rest.rfind('\'')).unwrap_or(rest.len());
    Some(rest[..stop].trim().to_string())
}

/// Metadata is usually UTF-8, but many older servers send Latin-1.
fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_block(text: &str) -> Vec<u8> {
        let mut v = text.as_bytes().to_vec();
        let padded = v.len().div_ceil(16) * 16;
        v.resize(padded, 0);
        let mut out = vec![(padded / 16) as u8];
        out.extend(v);
        out
    }

    #[test]
    fn parses_titles() {
        assert_eq!(
            parse_stream_title(b"StreamTitle='Artist - Song';StreamUrl='';\0\0\0"),
            Some("Artist - Song".to_string())
        );
        assert_eq!(parse_stream_title(b"StreamTitle='Don't Stop';"), Some("Don't Stop".to_string()));
        assert_eq!(parse_stream_title(b"StreamTitle='';"), Some(String::new()));
        assert_eq!(parse_stream_title(b"StreamUrl='x';"), None);
        // Unterminated value.
        assert_eq!(parse_stream_title(b"StreamTitle='Live'"), Some("Live".to_string()));
        // Latin-1 fallback.
        assert_eq!(parse_stream_title(b"StreamTitle='Caf\xe9';"), Some("Café".to_string()));
    }

    #[test]
    fn strips_metadata_from_audio() {
        let metaint = 8;
        let mut stream = Vec::new();
        stream.extend(b"AAAAAAAA");
        stream.extend(meta_block("StreamTitle='One';"));
        stream.extend(b"BBBBBBBB");
        stream.push(0); // empty metadata block
        stream.extend(b"CCCCCCCC");
        stream.extend(meta_block("StreamTitle='Two';StreamUrl='http://x';"));
        stream.extend(b"DDD");

        let title: TitleSlot = Arc::new(Mutex::new(None));
        let mut r = IcyReader::new(&stream[..], metaint, title.clone());
        let mut audio = Vec::new();
        let mut buf = [0u8; 5];
        let mut seen = Vec::new();
        loop {
            let n = r.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            audio.extend_from_slice(&buf[..n]);
            seen.push(title.lock().clone());
        }
        assert_eq!(audio, b"AAAAAAAABBBBBBBBCCCCCCCCDDD");
        assert!(seen.contains(&Some("One".to_string())));
        assert_eq!(*title.lock(), Some("Two".to_string()));
    }

    #[test]
    fn empty_title_clears() {
        let mut stream = b"AAAA".to_vec();
        stream.extend(meta_block("StreamTitle='';"));
        stream.extend(b"BBBB");
        let title: TitleSlot = Arc::new(Mutex::new(Some("old".into())));
        let mut r = IcyReader::new(&stream[..], 4, title.clone());
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"AAAABBBB");
        assert_eq!(*title.lock(), None);
    }
}
