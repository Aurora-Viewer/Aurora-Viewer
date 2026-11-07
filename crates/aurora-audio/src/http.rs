//! Minimal blocking HTTP/1.1 client for internet radio.
//!
//! A dedicated client is used instead of a general-purpose one because
//! SHOUTcast v1 servers answer with a non-HTTP status line (`ICY 200 OK`),
//! which strict HTTP parsers reject. Supports http/https, redirects and
//! chunked transfer encoding; every blocking read honours a stop flag.

use parking_lot::Mutex;
use std::collections::HashSet;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rustls_platform_verifier::BuilderVerifierExt;
use url::{Host, Url};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Socket read timeout; bounds how long a stop request can take to be seen.
const READ_POLL: Duration = Duration::from_millis(250);
/// No audio bytes for this long means the stream is dead.
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 5;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const USER_AGENT: &str = aurora_version::user_agent();

#[derive(Debug, thiserror::Error)]
pub(crate) enum HttpError {
    #[error("invalid stream URL: {0}")]
    Url(String),
    #[error("unsupported URL scheme `{0}`")]
    Scheme(String),
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("malformed HTTP response")]
    Malformed,
    #[error("HTTP status {0}")]
    Status(u16),
    #[error("too many redirects")]
    Redirects,
    #[error("server did not send a TLS reply")]
    NoTlsReply,
    #[error("stopped")]
    Stopped,
}

/// Body + interesting headers of a successful response.
pub(crate) struct HttpResponse {
    pub content_type: Option<String>,
    pub metaint: Option<usize>,
    pub body: Box<dyn Read + Send + Sync>,
}

/// Parsed status line + headers.
#[derive(Debug, PartialEq)]
pub(crate) struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Parses `HTTP/1.x 200 OK` or `ICY 200 OK`.
pub(crate) fn parse_status_line(line: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    let proto = parts.next()?;
    if !(proto.starts_with("HTTP/") || proto == "ICY") {
        return None;
    }
    let code = parts.next()?;
    if code.len() != 3 {
        return None;
    }
    code.parse().ok()
}

/// Parses response header lines (status line first, without the blank line).
pub(crate) fn parse_head(lines: &[String]) -> Option<Head> {
    let (first, rest) = lines.split_first()?;
    let status = parse_status_line(first)?;
    let headers = rest
        .iter()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    Some(Head { status, headers })
}

enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

/// Retries socket read timeouts while checking the stop flag, and fails if
/// the stream stalls.
struct StopAware<R> {
    inner: R,
    stop: Arc<AtomicBool>,
    last_data: Instant,
}

impl<R: Read> Read for StopAware<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(io::Error::other("stopped"));
            }
            match self.inner.read(buf) {
                Ok(n) => {
                    self.last_data = Instant::now();
                    return Ok(n);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                    ) =>
                {
                    if self.last_data.elapsed() > STALL_TIMEOUT {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "stream stalled"));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Decoder for `Transfer-Encoding: chunked`.
pub(crate) struct ChunkedReader<R> {
    inner: R,
    remaining: usize,
    done: bool,
}

impl<R: BufRead> ChunkedReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            remaining: 0,
            done: false,
        }
    }

    fn read_line(&mut self) -> io::Result<String> {
        let mut line = Vec::new();
        let n = (&mut self.inner).take(4096).read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(String::from_utf8_lossy(&line).trim().to_string())
    }
}

impl<R: BufRead> Read for ChunkedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut line = self.read_line()?;
            if line.is_empty() {
                // CRLF terminating the previous chunk.
                line = self.read_line()?;
            }
            let size_str = line.split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size_str, 16).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
            if size == 0 {
                self.done = true;
                return Ok(0);
            }
            self.remaining = size;
        }
        let n = buf.len().min(self.remaining);
        let got = self.inner.read(&mut buf[..n])?;
        if got == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= got;
        Ok(got)
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, HttpError> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            // Explicit provider: the workspace may enable several rustls
            // crypto backends, which makes the implicit default ambiguous.
            let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                // Some stations omit their intermediate certificate (#1628).
                // The Windows/macOS verifier can build the missing chain;
                // certificate trust and the server name are still checked.
                .and_then(|b| b.with_platform_verifier())
                .map(|b| Arc::new(b.with_no_client_auth()))
                .map_err(|e| e.to_string())
        })
        .clone()
        .map_err(HttpError::Tls)
}

fn connect(url: &Url, stop: &AtomicBool) -> Result<Conn, HttpError> {
    let https = match url.scheme() {
        "http" | "icy" => false,
        "https" => true,
        other => return Err(HttpError::Scheme(other.to_string())),
    };
    let port = url.port_or_known_default().unwrap_or(if https { 443 } else { 80 });
    let addrs: Vec<SocketAddr> = match url.host() {
        Some(Host::Domain(d)) => (d, port)
            .to_socket_addrs()
            .map_err(|e| HttpError::Connect(e.to_string()))?
            .collect(),
        Some(Host::Ipv4(ip)) => vec![SocketAddr::from((ip, port))],
        Some(Host::Ipv6(ip)) => vec![SocketAddr::from((ip, port))],
        None => return Err(HttpError::Url("missing host".into())),
    };
    let mut last_err = String::from("no address");
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let mut tcp = tcp.ok_or(HttpError::Connect(last_err))?;
    tcp.set_read_timeout(Some(READ_POLL))?;
    tcp.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let _ = tcp.set_nodelay(true);
    if !https {
        return Ok(Conn::Plain(tcp));
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let name = rustls::pki_types::ServerName::try_from(host).map_err(|e| HttpError::Tls(e.to_string()))?;
    let mut conn = rustls::ClientConnection::new(tls_config()?, name).map_err(|e| HttpError::Tls(e.to_string()))?;
    handshake(&mut conn, &mut tcp, stop)?;
    Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}

/// Observe the first response byte before rustls consumes it. Any TLS record
/// type is enough to forbid fallback, even if that record is incomplete.
struct HandshakeIo<'a> {
    socket: &'a mut TcpStream,
    first_byte: Option<u8>,
}

impl Read for HandshakeIo<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.socket.read(buf)?;
        if n > 0 && self.first_byte.is_none() {
            self.first_byte = Some(buf[0]);
        }
        Ok(n)
    }
}

impl Write for HandshakeIo<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.socket.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

fn no_tls_reply(error: &io::Error, first_byte: Option<u8>) -> bool {
    if first_byte.is_some_and(|byte| (20..=24).contains(&byte)) {
        return false;
    }
    if let Some(error) = error.get_ref().and_then(|cause| cause.downcast_ref::<rustls::Error>()) {
        return matches!(
            error,
            rustls::Error::InvalidMessage(
                rustls::InvalidMessage::InvalidContentType
                    | rustls::InvalidMessage::UnknownProtocolVersion
                    | rustls::InvalidMessage::MessageTooLarge
                    | rustls::InvalidMessage::MessageTooShort
            )
        );
    }
    // A server without TLS that drops the connection on our ClientHello
    // can surface as a reset, an abort or a broken pipe instead of an EOF
    // (Windows especially): the same case, no TLS record came back.
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
    )
}

fn handshake(conn: &mut rustls::ClientConnection, tcp: &mut TcpStream, stop: &AtomicBool) -> Result<(), HttpError> {
    let deadline = Instant::now() + STALL_TIMEOUT;
    let mut transport = HandshakeIo {
        socket: tcp,
        first_byte: None,
    };
    while conn.is_handshaking() {
        if stop.load(Ordering::Relaxed) {
            return Err(HttpError::Stopped);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "TLS handshake timed out").into());
        }
        match conn.complete_io(&mut transport) {
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => {
                if conn.is_handshaking()
                    && conn.protocol_version().is_none()
                    && conn.peer_certificates().is_none()
                    && no_tls_reply(&error, transport.first_byte)
                {
                    return Err(HttpError::NoTlsReply);
                }
                return Err(HttpError::Io(error));
            }
        }
    }
    Ok(())
}

fn plain_music_url(url: &Url) -> Option<Url> {
    let port = url.port()?;
    if url.scheme() != "https" || port == 443 {
        return None;
    }
    let mut plain = url.clone();
    plain.set_scheme("http").ok()?;
    plain.set_port(Some(port)).ok()?;
    Some(plain)
}

/// This client is used only for music. Remember exact station URLs, never a
/// blanket host policy, and never fall back after certificate/alert failures.
fn connect_music(url: &mut Url, stop: &AtomicBool) -> Result<Conn, HttpError> {
    static PLAIN_STATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let stations = PLAIN_STATIONS.get_or_init(|| Mutex::new(HashSet::new()));
    let original = url.as_str().to_owned();
    let plain = plain_music_url(url);
    let remembered = stations.lock().contains(&original);
    if let Some(plain) = plain.as_ref()
        && remembered
    {
        *url = plain.clone();
        return connect(url, stop);
    }
    match connect(url, stop) {
        Err(HttpError::NoTlsReply) if plain.is_some() && !stop.load(Ordering::Relaxed) => {
            let mut cache = stations.lock();
            // Keep memory and the once-per-URL log bounded for a long session.
            if cache.len() >= 256 && !cache.contains(&original) {
                return Err(HttpError::NoTlsReply);
            }
            if cache.insert(original) {
                log::warn!(
                    "music stream: {}:{} does not speak TLS, trying http",
                    url.host_str().unwrap_or("unknown"),
                    url.port().unwrap_or(443)
                );
            }
            drop(cache);
            if let Some(plain) = plain {
                *url = plain;
            }
            connect(url, stop)
        }
        result => result,
    }
}

fn request_target(url: &Url) -> String {
    let mut t = url.path().to_string();
    if t.is_empty() {
        t.push('/');
    }
    if let Some(q) = url.query() {
        t.push('?');
        t.push_str(q);
    }
    t
}

fn host_header(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    }
}

/// Reads the status line and headers, at most [`MAX_HEADER_BYTES`] in all:
/// each line read stops at what is left of that budget, so a line that
/// never ends is refused instead of being kept whole in memory.
fn read_head<R: BufRead>(r: &mut R) -> Result<Head, HttpError> {
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        // one byte over the budget tells a full budget from a longer line
        let left = (MAX_HEADER_BYTES - total) as u64 + 1;
        let n = (&mut *r).take(left).read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(HttpError::Malformed);
        }
        total += n;
        if total > MAX_HEADER_BYTES {
            return Err(HttpError::Malformed);
        }
        let text = String::from_utf8_lossy(&line).trim_end().to_string();
        if text.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(text);
    }
    parse_head(&lines).ok_or(HttpError::Malformed)
}

/// Opens `url` for streaming, following redirects. Sends `Icy-MetaData: 1`.
pub(crate) fn open(url: &str, stop: &Arc<AtomicBool>) -> Result<HttpResponse, HttpError> {
    let mut url = Url::parse(url.trim()).map_err(|e| HttpError::Url(e.to_string()))?;
    for _ in 0..=MAX_REDIRECTS {
        if stop.load(Ordering::Relaxed) {
            return Err(HttpError::Stopped);
        }
        let mut conn = connect_music(&mut url, stop)?;
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: */*\r\nIcy-MetaData: 1\r\nConnection: close\r\n\r\n",
            request_target(&url),
            host_header(&url),
            USER_AGENT
        );
        conn.write_all(req.as_bytes())?;
        conn.flush()?;
        let mut reader = BufReader::with_capacity(
            16 * 1024,
            StopAware {
                inner: conn,
                stop: stop.clone(),
                last_data: Instant::now(),
            },
        );
        let head = read_head(&mut reader)?;
        match head.status {
            200..=299 => {
                let content_type = head
                    .header("content-type")
                    .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
                let metaint = head.header("icy-metaint").and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0);
                let chunked = head
                    .header("transfer-encoding")
                    .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
                let body: Box<dyn Read + Send + Sync> = if chunked {
                    Box::new(ChunkedReader::new(reader))
                } else {
                    Box::new(reader)
                };
                return Ok(HttpResponse {
                    content_type,
                    metaint,
                    body,
                });
            }
            301 | 302 | 303 | 307 | 308 => {
                let loc = head.header("location").ok_or(HttpError::Malformed)?;
                url = url.join(loc).map_err(|e| HttpError::Url(e.to_string()))?;
                log::debug!("stream redirected to {url}");
            }
            other => return Err(HttpError::Status(other)),
        }
    }
    Err(HttpError::Redirects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parcel_music_plain_icy_https_fallback_preserves_request_and_is_remembered() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("https://{addr}/live?station=1");
        let server = std::thread::spawn(move || {
            for attempt in 0..3 {
                let deadline = Instant::now() + Duration::from_secs(10);
                let (mut sock, _) = loop {
                    match listener.accept() {
                        Ok(client) => break client,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "connection {attempt} missing");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                if attempt == 0 {
                    let mut hello = [0; 4096];
                    let n = sock.read(&mut hello).unwrap();
                    assert!(n > 0 && hello[0] == 22, "first request must try TLS");
                    sock.write_all(b"HTTP/1.0 400 Bad Request\r\n\r\n").unwrap();
                } else {
                    let mut reader = BufReader::new(sock);
                    let mut request = String::new();
                    loop {
                        let mut line = String::new();
                        let n = reader.read_line(&mut line).unwrap();
                        assert!(n > 0);
                        request.push_str(&line);
                        if line == "\r\n" {
                            break;
                        }
                    }
                    assert!(request.starts_with("GET /live?station=1 HTTP/1.1\r\n"), "{request}");
                    assert!(request.contains(&format!("Host: {addr}\r\n")), "{request}");
                    assert!(request.contains(&format!("User-Agent: {USER_AGENT}\r\n")), "{request}");
                    assert!(request.contains("Icy-MetaData: 1\r\n"));
                    reader
                        .get_mut()
                        .write_all(b"ICY 200 OK\r\nContent-Type: audio/mpeg\r\n\r\nMP3")
                        .unwrap();
                    reader.get_mut().shutdown(std::net::Shutdown::Write).unwrap();
                    let mut drain = Vec::new();
                    let _ = reader.read_to_end(&mut drain);
                }
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        for _ in 0..2 {
            let mut response = open(&url, &stop).unwrap();
            assert_eq!(response.content_type.as_deref(), Some("audio/mpeg"));
            let mut body = [0; 3];
            response.body.read_exact(&mut body).unwrap();
            assert_eq!(&body, b"MP3");
        }
        server.join().unwrap();
    }

    #[test]
    fn parcel_music_fallback_excludes_standard_tls_and_certificate_failures() {
        assert!(plain_music_url(&Url::parse("https://radio.example/live").unwrap()).is_none());
        assert!(plain_music_url(&Url::parse("https://radio.example:443/live").unwrap()).is_none());
        assert!(plain_music_url(&Url::parse("http://radio.example:8443/live").unwrap()).is_none());
        let plain = plain_music_url(&Url::parse("https://radio.example:1701/live?q=1").unwrap()).unwrap();
        assert_eq!(plain.as_str(), "http://radio.example:1701/live?q=1");
        assert!(no_tls_reply(&io::ErrorKind::UnexpectedEof.into(), None));
        assert!(!no_tls_reply(&io::ErrorKind::UnexpectedEof.into(), Some(22)));
        assert!(!no_tls_reply(&io::ErrorKind::TimedOut.into(), None));
        assert!(!no_tls_reply(&io::ErrorKind::ConnectionRefused.into(), None));
        for kind in [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
        ] {
            assert!(no_tls_reply(&kind.into(), None), "{kind:?} before any reply");
            assert!(!no_tls_reply(&kind.into(), Some(22)), "{kind:?} after a TLS handshake record");
            assert!(no_tls_reply(&kind.into(), Some(b'H')), "{kind:?} after a plain reply byte");
        }
        let record = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidMessage(rustls::InvalidMessage::InvalidContentType),
        );
        assert!(no_tls_reply(&record, Some(b'H')));
        let cert = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding),
        );
        assert!(!no_tls_reply(&cert, None));
        let alert = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(rustls::AlertDescription::HandshakeFailure),
        );
        assert!(!no_tls_reply(&alert, None));
    }

    #[test]
    fn https_rejects_an_untrusted_certificate() {
        let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://{}/stream", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            sock.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut tls = rustls::ServerConnection::new(Arc::new(config)).unwrap();
            let _ = tls.complete_io(&mut sock);
            drop(sock);
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut plain, _)) => {
                        let _ = plain.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                        return true;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                    Err(e) => panic!("{e}"),
                }
            }
            false
        });
        let stop = Arc::new(AtomicBool::new(false));
        let error = open(&url, &stop).err().expect("a self-signed station must be rejected");
        assert!(!server.join().unwrap(), "certificate failure must not make a plaintext request");
        let HttpError::Io(error) = error else {
            panic!("expected a TLS handshake error: {error}");
        };
        assert!(
            matches!(
                error.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>()),
                Some(rustls::Error::InvalidCertificate(_))
            ),
            "{error:?}"
        );
    }

    #[test]
    fn status_lines() {
        assert_eq!(parse_status_line("ICY 200 OK"), Some(200));
        assert_eq!(parse_status_line("HTTP/1.0 200 OK"), Some(200));
        assert_eq!(parse_status_line("HTTP/1.1 302 Found"), Some(302));
        assert_eq!(parse_status_line("HTTP/1.1 404"), Some(404));
        assert_eq!(parse_status_line("FOO 200 OK"), None);
        assert_eq!(parse_status_line("HTTP/1.1 2000 OK"), None);
        assert_eq!(parse_status_line(""), None);
    }

    #[test]
    fn heads() {
        let lines: Vec<String> = [
            "ICY 200 OK",
            "icy-name: Test Radio",
            "Content-Type: audio/mpeg",
            "icy-metaint:16000",
            "garbage line",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let head = parse_head(&lines).unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-type"), Some("audio/mpeg"));
        assert_eq!(head.header("ICY-METAINT"), Some("16000"));
        assert_eq!(head.header("icy-name"), Some("Test Radio"));
        assert_eq!(head.headers.len(), 3);
    }

    #[test]
    fn read_head_from_bytes() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: audio/ogg\r\n\r\nBODY";
        let mut r = BufReader::new(&raw[..]);
        let head = read_head(&mut r).unwrap();
        assert_eq!(head.status, 200);
        let mut body = String::new();
        r.read_to_string(&mut body).unwrap();
        assert_eq!(body, "BODY");
    }

    /// Endless bytes without a line end, counting what was read.
    struct Endless {
        byte: u8,
        read: usize,
    }

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(self.byte);
            self.read += buf.len();
            Ok(buf.len())
        }
    }

    #[test]
    fn read_head_stops_a_line_that_never_ends() {
        let mut r = BufReader::with_capacity(1024, Endless { byte: b'a', read: 0 });
        assert!(matches!(read_head(&mut r), Err(HttpError::Malformed)));
        assert!(r.get_ref().read <= MAX_HEADER_BYTES + 1024, "{}", r.get_ref().read);

        // the same once the status line is in
        let mut raw = b"ICY 200 OK\r\nicy-name: ".to_vec();
        raw.extend(std::iter::repeat_n(b'x', MAX_HEADER_BYTES));
        let mut r = BufReader::new(&raw[..]);
        assert!(matches!(read_head(&mut r), Err(HttpError::Malformed)));
    }

    #[test]
    fn read_head_budget_counts_every_line() {
        let line = format!("x-pad: {}\r\n", "y".repeat(100));
        let mut raw = b"HTTP/1.1 200 OK\r\n".to_vec();
        for _ in 0..MAX_HEADER_BYTES / line.len() + 1 {
            raw.extend(line.as_bytes());
        }
        raw.extend(b"\r\n");
        let mut r = BufReader::new(&raw[..]);
        assert!(matches!(read_head(&mut r), Err(HttpError::Malformed)));

        // a large head under the budget still reads
        let mut raw = b"HTTP/1.1 200 OK\r\n".to_vec();
        for _ in 0..(MAX_HEADER_BYTES - 64) / line.len() {
            raw.extend(line.as_bytes());
        }
        raw.extend(b"\r\nBODY");
        let mut r = BufReader::new(&raw[..]);
        let head = read_head(&mut r).unwrap();
        assert_eq!(head.status, 200);
        let mut body = String::new();
        r.read_to_string(&mut body).unwrap();
        assert_eq!(body, "BODY");
    }

    #[test]
    fn chunked_decoding() {
        let raw = b"5\r\nhello\r\n7;ext=1\r\n, world\r\n0\r\n\r\n";
        let mut r = ChunkedReader::new(BufReader::new(&raw[..]));
        let mut out = String::new();
        r.read_to_string(&mut out).unwrap();
        assert_eq!(out, "hello, world");

        let bad = b"zz\r\nhello\r\n";
        let mut r = ChunkedReader::new(BufReader::new(&bad[..]));
        let mut out = Vec::new();
        assert!(r.read_to_end(&mut out).is_err());
    }

    #[test]
    fn request_parts() {
        let u = Url::parse("http://radio.example:8000/stream?x=1").unwrap();
        assert_eq!(request_target(&u), "/stream?x=1");
        assert_eq!(host_header(&u), "radio.example:8000");
        let u = Url::parse("https://radio.example").unwrap();
        assert_eq!(request_target(&u), "/");
        assert_eq!(host_header(&u), "radio.example");
    }

    #[test]
    fn rejects_unknown_scheme() {
        let stop = Arc::new(AtomicBool::new(false));
        assert!(matches!(open("ftp://example/x", &stop), Err(HttpError::Scheme(_))));
        assert!(matches!(open("not a url", &stop), Err(HttpError::Url(_))));
    }
}
