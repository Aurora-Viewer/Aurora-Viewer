//! Blocking TCP connections that follow the rule, for the clients that do not
//! use reqwest (the internet radio streams of `aurora-audio`): direct, a
//! SOCKS 5 CONNECT (the proxy resolves the name), or an HTTP CONNECT tunnel.
//! TLS, when the caller wants it, runs on top of the returned stream with the
//! caller's own certificate checks: the proxy never sees inside.
//!
//! The Legacy Viewer plays parcel music through FMOD, outside its proxy:
//! routing it through the proxy is stricter (the real address does not leak to
//! the stream's server).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::socks5::{self, Command, Handshake, Target};
use crate::{Endpoint, HttpRoute, is_loopback_host, rule};

/// The most an HTTP CONNECT reply's head may weigh.
const MAX_CONNECT_REPLY: usize = 8 * 1024;

fn other(message: &'static str) -> std::io::Error {
    std::io::Error::other(message)
}

/// Opens a TCP connection to `host:port` the way the installed rule says.
/// `timeout` bounds the connection and each read or write of the proxy's
/// handshake; the stream keeps it as its read and write timeout.
pub fn tcp_connect(host: &str, port: u16, timeout: Duration) -> std::io::Result<TcpStream> {
    let route = if is_loopback_host(host) { &HttpRoute::System } else { &rule().http };
    match route {
        // radio streams never followed the environment's HTTP proxies
        HttpRoute::System => direct(host, port, timeout),
        HttpRoute::Blocked => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the proxy is not available",
        )),
        HttpRoute::Socks(server) => {
            let mut stream = to_proxy(&server.endpoint, timeout)?;
            let target = Target::from_host(host, port).ok_or_else(|| other("invalid host"))?;
            socks5::run_blocking(&mut stream, Handshake::new(server.credentials.clone(), Command::Connect(target)))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            Ok(stream)
        }
        HttpRoute::Web(endpoint) => {
            let mut stream = to_proxy(endpoint, timeout)?;
            http_connect(&mut stream, host, port, Instant::now() + timeout)?;
            Ok(stream)
        }
    }
}

fn direct(host: &str, port: u16, timeout: Duration) -> std::io::Result<TcpStream> {
    let mut last = None;
    for addr in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return with_timeouts(stream, timeout),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| other("no address")))
}

fn to_proxy(endpoint: &Endpoint, timeout: Duration) -> std::io::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = (endpoint.host.as_str(), endpoint.port).to_socket_addrs()?.collect();
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return with_timeouts(stream, timeout),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| other("the proxy has no address")))
}

fn with_timeouts(stream: TcpStream, timeout: Duration) -> std::io::Result<TcpStream> {
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}

/// `host` as the authority of a CONNECT request: IPv6 literals in brackets.
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Opens an HTTP CONNECT tunnel on `stream`. The reply head is read byte by
/// byte, so nothing past it (the start of the tunnel) is consumed; the whole
/// head must arrive before `deadline` (a proxy that sends it a byte at a time
/// does not hold the caller past it).
fn http_connect<S: Read + Write>(stream: &mut S, host: &str, port: u16, deadline: Instant) -> std::io::Result<()> {
    // a host with a line break or a space would write other headers
    if host.bytes().any(|b| b.is_ascii_control() || b == b' ') {
        return Err(other("invalid host"));
    }
    let authority = authority(host, port);
    stream.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())?;
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_CONNECT_REPLY {
            return Err(other("the proxy's reply is too long"));
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the proxy's reply is too slow"));
        }
        if stream.read(&mut byte)? == 0 {
            return Err(other("the proxy closed the connection"));
        }
        head.push(byte[0]);
    }
    if connect_granted(&head) {
        Ok(())
    } else {
        Err(other("the proxy refused the tunnel"))
    }
}

/// Whether the head of a CONNECT reply grants the tunnel: an `HTTP/1.x 2xx`
/// status line.
fn connect_granted(head: &[u8]) -> bool {
    let line = head.split(|&b| b == b'\n').next().unwrap_or_default();
    // `HTTP/1.1 200` without a reason phrase ends with the `\r`
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut parts = line.split(|&b| b == b' ');
    let version = parts.next().unwrap_or_default();
    let status = parts.next().unwrap_or_default();
    (version == b"HTTP/1.1" || version == b"HTTP/1.0") && status.len() == 3 && status[0] == b'2' && status.iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Script {
        input: std::io::Cursor<Vec<u8>>,
        written: Vec<u8>,
    }
    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }
    impl Write for Script {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn later() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }
    fn script(reply: &[u8]) -> Script {
        Script {
            input: std::io::Cursor::new(reply.to_vec()),
            written: Vec::new(),
        }
    }

    #[test]
    fn connect_tunnel_granted_without_reading_past_the_head() {
        let mut s = script(b"HTTP/1.1 200 Connection established\r\nVia: x\r\n\r\nTLS-BYTES");
        http_connect(&mut s, "radio.example", 443, later()).unwrap();
        assert_eq!(s.written, b"CONNECT radio.example:443 HTTP/1.1\r\nHost: radio.example:443\r\n\r\n");
        let mut rest = Vec::new();
        s.input.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"TLS-BYTES");
    }

    #[test]
    fn connect_tunnel_refusals() {
        for reply in [
            &b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"[..],
            b"HTTP/1.1 302 Found\r\n\r\n",
            b"SOCKS\r\n\r\n",
            b"HTTP/1.1 200", // closed before the end of the head
            b"HTTP/1.1 2000 OK\r\n\r\n",
        ] {
            assert!(
                http_connect(&mut script(reply), "h", 1, later()).is_err(),
                "{:?}",
                String::from_utf8_lossy(reply)
            );
        }
        let endless = vec![b'a'; MAX_CONNECT_REPLY + 10];
        assert!(http_connect(&mut script(&endless), "h", 1, later()).is_err());
        // past its deadline, even a valid reply is refused
        let past = Instant::now();
        assert!(http_connect(&mut script(b"HTTP/1.1 200 OK\r\n\r\n"), "h", 1, past).is_err());
    }

    #[test]
    fn connect_refuses_header_injection() {
        let mut s = script(b"HTTP/1.1 200 OK\r\n\r\n");
        assert!(http_connect(&mut s, "a\r\nX-Evil: 1", 80, later()).is_err());
        assert!(s.written.is_empty());
    }

    #[test]
    fn status_line_without_reason_phrase() {
        assert!(connect_granted(b"HTTP/1.1 200\r\n\r\n"));
        assert!(connect_granted(b"HTTP/1.0 200 OK\r\n\r\n"));
        assert!(!connect_granted(b"HTTP/1.1 20\r\n\r\n"));
    }

    #[test]
    fn ipv6_authority() {
        assert_eq!(authority("2001:db8::1", 443), "[2001:db8::1]:443");
        assert_eq!(authority("[2001:db8::1]", 443), "[2001:db8::1]:443");
        assert_eq!(authority("host", 80), "host:80");
    }
}
