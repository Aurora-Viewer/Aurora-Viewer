//! SOCKS 5 client (RFC 1928, user name / password of RFC 1929), without I/O:
//! the handshake is a small state machine fed with the exact bytes it asks
//! for, so the async driver (UDP ASSOCIATE for the circuits, `udp.rs`) and the
//! blocking one (CONNECT for the radio streams, `connect.rs`) share every
//! check, and the tests feed it hostile replies without a socket.
//!
//! Port of LLProxy::proxyHandshake (indra/llmessage/llproxy.cpp, originally
//! LGPL 2.1), stricter: the Legacy Viewer reads every reply as a fixed 10-byte
//! IPv4 structure and accepts short reads; here each field is checked, a reply
//! names its own length (bounded: a domain is at most 255 bytes, a whole
//! handshake reads at most `MAX_READ` bytes) and nothing is taken on trust.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub const VERSION: u8 = 0x05;
const AUTH_VERSION: u8 = 0x01;

const METHOD_NONE: u8 = 0x00;
const METHOD_PASSWORD: u8 = 0x02;
const METHOD_NOT_ACCEPTABLE: u8 = 0xFF;

pub(crate) const ATYP_IPV4: u8 = 0x01;
pub(crate) const ATYP_DOMAIN: u8 = 0x03;
pub(crate) const ATYP_IPV6: u8 = 0x04;

const CMD_CONNECT: u8 = 0x01;
const CMD_UDP_ASSOCIATE: u8 = 0x03;

const REPLY_GRANTED: u8 = 0x00;
const REPLY_NOT_ALLOWED: u8 = 0x02;

/// SOCKS 5 user names and passwords are 1 to 255 bytes (SOCKSMINUSERNAMELEN …
/// SOCKSMAXPASSWORDLEN, llproxy.h).
pub const MAX_CREDENTIAL: usize = 255;

/// The most a whole handshake reads from the server: method reply (2), auth
/// reply (2), reply head (4), the longest address (1 + 255) and the port (2).
pub const MAX_READ: usize = 2 + 2 + 4 + 1 + 255 + 2;

/// Why a SOCKS 5 exchange failed. Each kind has the Legacy Viewer's message
/// (`SOCKS_*` notifications); none carries text from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SocksError {
    /// The proxy's host name does not resolve, or its address or port is not
    /// valid (SOCKS_INVALID_HOST).
    #[error("invalid SOCKS 5 proxy address or port")]
    InvalidHost,
    /// No TCP connection to the proxy (SOCKS_HOST_CONNECT_FAILED).
    #[error("could not connect to the SOCKS 5 proxy")]
    HostConnectFailed,
    /// The control channel failed or closed during the exchange
    /// (SOCKS_CONNECT_ERROR).
    #[error("the SOCKS 5 proxy closed the connection")]
    ConnectError,
    /// The proxy did not answer in time (SOCKS_CONNECT_ERROR).
    #[error("the SOCKS 5 proxy did not answer in time")]
    Timeout,
    /// The reply is not SOCKS 5 or breaks the protocol (SOCKS_CONNECT_ERROR).
    #[error("the SOCKS 5 proxy sent an invalid reply")]
    Protocol,
    /// The proxy refused the authentication method (SOCKS_NOT_ACCEPTABLE).
    #[error("the SOCKS 5 proxy refused the authentication method")]
    NotAcceptable,
    /// The proxy refused the user name and password (SOCKS_AUTH_FAIL).
    #[error("the SOCKS 5 proxy refused the credentials")]
    AuthFailed,
    /// Refused by the proxy's rule set (SOCKS_NOT_PERMITTED).
    #[error("the SOCKS 5 proxy's rules refused the connection")]
    NotPermitted,
    /// Any other refusal of the command (SOCKS_UDP_FWD_NOT_GRANTED for a UDP
    /// ASSOCIATE); the code is the server's reply field.
    #[error("the SOCKS 5 proxy refused the request (code {0})")]
    Refused(u8),
    /// The user name or password is empty or longer than 255 bytes
    /// (SOCKS_BAD_CREDS).
    #[error("invalid SOCKS 5 user name or password")]
    BadCredentials,
}

/// A user name and password for the proxy. `Debug` never shows them; they are
/// only written into the RFC 1929 request and, for the HTTP clients, into
/// reqwest's proxy (which keeps them out of its own `Debug`).
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    /// Checks the lengths like LLProxy::setAuthPassword (1 to 255 bytes each).
    pub fn new(username: String, password: String) -> Result<Self, SocksError> {
        let ok = |s: &str| (1..=MAX_CREDENTIAL).contains(&s.len());
        if ok(&username) && ok(&password) {
            Ok(Self { username, password })
        } else {
            Err(SocksError::BadCredentials)
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub(crate) fn password(&self) -> &str {
        &self.password
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(<redacted>)")
    }
}

/// Where a CONNECT goes: an address, or a name the proxy resolves (no DNS
/// request leaves this computer for it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Addr(SocketAddr),
    Domain(String, u16),
}

impl Target {
    /// A host and port as an URL gives them: an IP literal stays an address,
    /// anything else is a name for the proxy. `None` for a name too long for
    /// SOCKS 5 (255 bytes) or empty.
    pub fn from_host(host: &str, port: u16) -> Option<Self> {
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = bare.parse::<IpAddr>() {
            return Some(Self::Addr(SocketAddr::new(ip, port)));
        }
        (!host.is_empty() && host.len() <= 255).then(|| Self::Domain(host.to_owned(), port))
    }

    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Addr(SocketAddr::V4(a)) => {
                out.push(ATYP_IPV4);
                out.extend_from_slice(&a.ip().octets());
                out.extend_from_slice(&a.port().to_be_bytes());
            }
            Self::Addr(SocketAddr::V6(a)) => {
                out.push(ATYP_IPV6);
                out.extend_from_slice(&a.ip().octets());
                out.extend_from_slice(&a.port().to_be_bytes());
            }
            Self::Domain(name, port) => {
                out.push(ATYP_DOMAIN);
                // from_host bounds the length; a longer name cannot be built
                out.push(name.len().min(255) as u8);
                out.extend_from_slice(&name.as_bytes()[..name.len().min(255)]);
                out.extend_from_slice(&port.to_be_bytes());
            }
        }
    }
}

/// The address the proxy names in its reply (BND.ADDR / BND.PORT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    Addr(SocketAddr),
    /// A name; only its port is used.
    Domain(u16),
}

impl Bound {
    pub fn port(&self) -> u16 {
        match self {
            Self::Addr(a) => a.port(),
            Self::Domain(port) => *port,
        }
    }
}

/// Which command the handshake ends with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Connect(Target),
    /// UDP ASSOCIATE with 0.0.0.0:0, as the Legacy Viewer ("If the client is
    /// not in possession of the information at the time of the UDP
    /// ASSOCIATE, the client MUST use a port number and address of all
    /// zeros", RFC 1928).
    UdpAssociate,
}

/// What the driver does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Write these bytes (maybe none), then read exactly `read` bytes and feed
    /// them back.
    Exchange { write: Vec<u8>, read: usize },
    /// The proxy granted the command.
    Done(Bound),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Method,
    Auth,
    Head,
    DomainLength,
    Address { atyp: u8, len: usize },
    Finished,
}

/// One handshake. `start` gives the first write; `feed` takes each reply.
pub struct Handshake {
    credentials: Option<Credentials>,
    command: Command,
    state: State,
    read_total: usize,
}

impl Handshake {
    pub fn new(credentials: Option<Credentials>, command: Command) -> Self {
        Self {
            credentials,
            command,
            state: State::Method,
            read_total: 0,
        }
    }

    fn method(&self) -> u8 {
        if self.credentials.is_some() { METHOD_PASSWORD } else { METHOD_NONE }
    }

    /// The greeting: one method only, the one the settings chose (as the
    /// Legacy Viewer): with credentials set, a proxy that does not ask for
    /// them is refused rather than used without them.
    pub fn start(&self) -> Step {
        Step::Exchange {
            write: vec![VERSION, 1, self.method()],
            read: 2,
        }
    }

    fn request(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 1 + 255 + 2);
        out.push(VERSION);
        match &self.command {
            Command::Connect(target) => {
                out.extend_from_slice(&[CMD_CONNECT, 0]);
                target.encode(&mut out);
            }
            Command::UdpAssociate => {
                out.extend_from_slice(&[CMD_UDP_ASSOCIATE, 0]);
                Target::Addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).encode(&mut out);
            }
        }
        out
    }

    fn auth_request(credentials: &Credentials) -> Vec<u8> {
        let (user, pass) = (credentials.username().as_bytes(), credentials.password().as_bytes());
        let mut out = Vec::with_capacity(3 + user.len() + pass.len());
        out.push(AUTH_VERSION);
        // Credentials::new bounds both to 255 bytes
        out.push(user.len() as u8);
        out.extend_from_slice(user);
        out.push(pass.len() as u8);
        out.extend_from_slice(pass);
        out
    }

    fn fail(&mut self, error: SocksError) -> Result<Step, SocksError> {
        self.state = State::Finished;
        Err(error)
    }

    /// The reply asked for by the previous step. A length other than the one
    /// asked for, or any byte after the end, is a protocol error.
    pub fn feed(&mut self, input: &[u8]) -> Result<Step, SocksError> {
        self.read_total = self.read_total.saturating_add(input.len());
        if self.read_total > MAX_READ {
            return self.fail(SocksError::Protocol);
        }
        match self.state {
            State::Method => {
                let [version, method] = input else {
                    return self.fail(SocksError::Protocol);
                };
                if *version != VERSION {
                    return self.fail(SocksError::Protocol);
                }
                if *method == METHOD_NOT_ACCEPTABLE {
                    return self.fail(SocksError::NotAcceptable);
                }
                // a method we did not offer: RFC 1928 lets the server only
                // choose among ours
                if *method != self.method() {
                    return self.fail(SocksError::Protocol);
                }
                match &self.credentials {
                    Some(credentials) => {
                        self.state = State::Auth;
                        Ok(Step::Exchange {
                            write: Self::auth_request(credentials),
                            read: 2,
                        })
                    }
                    None => {
                        self.state = State::Head;
                        Ok(Step::Exchange {
                            write: self.request(),
                            read: 4,
                        })
                    }
                }
            }
            State::Auth => {
                let [version, status] = input else {
                    return self.fail(SocksError::Protocol);
                };
                if *version != AUTH_VERSION {
                    return self.fail(SocksError::Protocol);
                }
                if *status != 0 {
                    return self.fail(SocksError::AuthFailed);
                }
                self.state = State::Head;
                Ok(Step::Exchange {
                    write: self.request(),
                    read: 4,
                })
            }
            State::Head => {
                let [version, reply, _reserved, atyp] = input else {
                    return self.fail(SocksError::Protocol);
                };
                if *version != VERSION {
                    return self.fail(SocksError::Protocol);
                }
                match *reply {
                    REPLY_GRANTED => {}
                    REPLY_NOT_ALLOWED => return self.fail(SocksError::NotPermitted),
                    code => return self.fail(SocksError::Refused(code)),
                }
                match *atyp {
                    ATYP_IPV4 => {
                        self.state = State::Address { atyp: ATYP_IPV4, len: 4 };
                        Ok(Step::Exchange {
                            write: Vec::new(),
                            read: 4 + 2,
                        })
                    }
                    ATYP_IPV6 => {
                        self.state = State::Address { atyp: ATYP_IPV6, len: 16 };
                        Ok(Step::Exchange {
                            write: Vec::new(),
                            read: 16 + 2,
                        })
                    }
                    ATYP_DOMAIN => {
                        self.state = State::DomainLength;
                        Ok(Step::Exchange {
                            write: Vec::new(),
                            read: 1,
                        })
                    }
                    _ => self.fail(SocksError::Protocol),
                }
            }
            State::DomainLength => {
                let [len] = input else {
                    return self.fail(SocksError::Protocol);
                };
                if *len == 0 {
                    return self.fail(SocksError::Protocol);
                }
                let len = usize::from(*len);
                self.state = State::Address { atyp: ATYP_DOMAIN, len };
                Ok(Step::Exchange {
                    write: Vec::new(),
                    read: len + 2,
                })
            }
            State::Address { atyp, len } => {
                if input.len() != len + 2 {
                    return self.fail(SocksError::Protocol);
                }
                let port = u16::from_be_bytes([input[len], input[len + 1]]);
                let bound = match atyp {
                    ATYP_IPV4 => {
                        let mut ip = [0u8; 4];
                        ip.copy_from_slice(&input[..4]);
                        Bound::Addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port))
                    }
                    ATYP_IPV6 => {
                        let mut ip = [0u8; 16];
                        ip.copy_from_slice(&input[..16]);
                        Bound::Addr(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip)), port))
                    }
                    _ => Bound::Domain(port),
                };
                self.state = State::Finished;
                Ok(Step::Done(bound))
            }
            State::Finished => Err(SocksError::Protocol),
        }
    }
}

/// Drives a handshake over a blocking stream (the radio's CONNECT). The
/// stream's read and write timeouts bound every exchange.
pub fn run_blocking<S: std::io::Read + std::io::Write>(stream: &mut S, mut handshake: Handshake) -> Result<Bound, SocksError> {
    let mut step = handshake.start();
    loop {
        match step {
            Step::Done(bound) => return Ok(bound),
            Step::Exchange { write, read } => {
                if !write.is_empty() {
                    stream.write_all(&write).map_err(io_error)?;
                }
                let mut buf = [0u8; 257];
                let buf = buf.get_mut(..read).ok_or(SocksError::Protocol)?;
                stream.read_exact(buf).map_err(io_error)?;
                step = handshake.feed(buf)?;
            }
        }
    }
}

/// Drives a handshake over an async stream (the circuits' UDP ASSOCIATE).
pub async fn run_async<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    mut handshake: Handshake,
    timeout: std::time::Duration,
) -> Result<Bound, SocksError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let exchange = async {
        let mut step = handshake.start();
        loop {
            match step {
                Step::Done(bound) => return Ok(bound),
                Step::Exchange { write, read } => {
                    if !write.is_empty() {
                        stream.write_all(&write).await.map_err(io_error)?;
                    }
                    let mut buf = [0u8; 257];
                    let buf = buf.get_mut(..read).ok_or(SocksError::Protocol)?;
                    stream.read_exact(buf).await.map_err(io_error)?;
                    step = handshake.feed(buf)?;
                }
            }
        }
    };
    tokio::time::timeout(timeout, exchange).await.unwrap_or(Err(SocksError::Timeout))
}

fn io_error(error: std::io::Error) -> SocksError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => SocksError::Timeout,
        _ => SocksError::ConnectError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials::new("user".into(), "pa:ss".into()).expect("valid")
    }

    fn write_of(step: &Step) -> &[u8] {
        match step {
            Step::Exchange { write, .. } => write,
            Step::Done(_) => panic!("no write"),
        }
    }

    fn granted_v4(port: u16) -> Vec<u8> {
        let mut v = vec![1, 2, 3, 4];
        v.extend_from_slice(&port.to_be_bytes());
        v
    }

    #[test]
    fn no_auth_udp_associate() {
        let mut h = Handshake::new(None, Command::UdpAssociate);
        assert_eq!(
            h.start(),
            Step::Exchange {
                write: vec![5, 1, 0],
                read: 2
            }
        );
        let step = h.feed(&[5, 0]).unwrap();
        assert_eq!(write_of(&step), &[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(h.feed(&[5, 0, 0, 1]).unwrap(), Step::Exchange { write: vec![], read: 6 });
        assert_eq!(
            h.feed(&granted_v4(4000)).unwrap(),
            Step::Done(Bound::Addr("1.2.3.4:4000".parse().unwrap()))
        );
    }

    #[test]
    fn password_auth_then_connect_by_name() {
        let target = Target::from_host("radio.example", 8000).unwrap();
        let mut h = Handshake::new(Some(creds()), Command::Connect(target));
        assert_eq!(write_of(&h.start()), &[5, 1, 2]);
        let step = h.feed(&[5, 2]).unwrap();
        assert_eq!(write_of(&step), b"\x01\x04user\x05pa:ss");
        let step = h.feed(&[1, 0]).unwrap();
        let mut expected = vec![5, 1, 0, 3, 13];
        expected.extend_from_slice(b"radio.example");
        expected.extend_from_slice(&8000u16.to_be_bytes());
        assert_eq!(write_of(&step), expected.as_slice());
        assert_eq!(h.feed(&[5, 0, 0, 3]).unwrap(), Step::Exchange { write: vec![], read: 1 });
        assert_eq!(h.feed(&[3]).unwrap(), Step::Exchange { write: vec![], read: 5 });
        assert_eq!(h.feed(b"abc\x00\x50").unwrap(), Step::Done(Bound::Domain(80)));
    }

    #[test]
    fn ipv6_reply_and_target() {
        let mut h = Handshake::new(None, Command::Connect(Target::from_host("[::1]", 9).unwrap()));
        let step = h.feed(&[5, 0]).unwrap();
        assert_eq!(write_of(&step)[3], ATYP_IPV6);
        assert_eq!(write_of(&step).len(), 4 + 16 + 2);
        assert_eq!(h.feed(&[5, 0, 0, 4]).unwrap(), Step::Exchange { write: vec![], read: 18 });
        let mut addr = [0u8; 18];
        addr[15] = 1;
        addr[17] = 7;
        assert_eq!(h.feed(&addr).unwrap(), Step::Done(Bound::Addr("[::1]:7".parse().unwrap())));
    }

    #[test]
    fn hostile_method_replies() {
        for (reply, error) in [
            (&[4u8, 0][..], SocksError::Protocol),   // SOCKS 4
            (&[5, 0xFF], SocksError::NotAcceptable), // no acceptable method
            (&[5, 2], SocksError::Protocol),         // a method we did not offer
            (&[5, 1], SocksError::Protocol),         // GSSAPI, not offered
            (&[5], SocksError::Protocol),            // short
            (&[5, 0, 0], SocksError::Protocol),      // long
            (&[], SocksError::Protocol),             // empty
        ] {
            let mut h = Handshake::new(None, Command::UdpAssociate);
            assert_eq!(h.feed(reply), Err(error), "{reply:?}");
            // a failed handshake stays failed
            assert_eq!(h.feed(&[5, 0]), Err(SocksError::Protocol));
        }
    }

    #[test]
    fn with_credentials_a_proxy_that_skips_auth_is_refused() {
        let mut h = Handshake::new(Some(creds()), Command::UdpAssociate);
        assert_eq!(h.feed(&[5, 0]), Err(SocksError::Protocol));
    }

    #[test]
    fn hostile_auth_replies() {
        for (reply, error) in [
            (&[1u8, 1][..], SocksError::AuthFailed),
            (&[1, 0xFF], SocksError::AuthFailed),
            (&[5, 0], SocksError::Protocol),
            (&[1], SocksError::Protocol),
        ] {
            let mut h = Handshake::new(Some(creds()), Command::UdpAssociate);
            h.feed(&[5, 2]).unwrap();
            assert_eq!(h.feed(reply), Err(error), "{reply:?}");
        }
    }

    #[test]
    fn hostile_command_replies() {
        for (reply, error) in [
            (&[5u8, 1, 0, 1][..], SocksError::Refused(1)),
            (&[5, 2, 0, 1], SocksError::NotPermitted),
            (&[5, 7, 0, 1], SocksError::Refused(7)),
            (&[5, 0, 0, 9], SocksError::Protocol), // unknown address type
            (&[5, 0, 0, 0], SocksError::Protocol),
            (&[4, 0, 0, 1], SocksError::Protocol),
            (&[5, 0, 0], SocksError::Protocol),
        ] {
            let mut h = Handshake::new(None, Command::UdpAssociate);
            h.feed(&[5, 0]).unwrap();
            assert_eq!(h.feed(reply), Err(error), "{reply:?}");
        }
    }

    #[test]
    fn hostile_addresses() {
        // empty domain name
        let mut h = Handshake::new(None, Command::UdpAssociate);
        h.feed(&[5, 0]).unwrap();
        h.feed(&[5, 0, 0, 3]).unwrap();
        assert_eq!(h.feed(&[0]), Err(SocksError::Protocol));
        // truncated and oversized IPv4 address
        for bad in [&[1u8, 2, 3, 4, 0][..], &[1, 2, 3, 4, 0, 1, 9]] {
            let mut h = Handshake::new(None, Command::UdpAssociate);
            h.feed(&[5, 0]).unwrap();
            h.feed(&[5, 0, 0, 1]).unwrap();
            assert_eq!(h.feed(bad), Err(SocksError::Protocol));
        }
        // the longest domain the protocol allows is read, never more
        let mut h = Handshake::new(Some(creds()), Command::UdpAssociate);
        h.feed(&[5, 2]).unwrap();
        h.feed(&[1, 0]).unwrap();
        h.feed(&[5, 0, 0, 3]).unwrap();
        assert_eq!(h.feed(&[255]).unwrap(), Step::Exchange { write: vec![], read: 257 });
        let mut long = vec![b'a'; 255];
        long.extend_from_slice(&[0, 1]);
        assert_eq!(h.feed(&long).unwrap(), Step::Done(Bound::Domain(1)));
        assert_eq!(h.feed(&[0]), Err(SocksError::Protocol));
    }

    #[test]
    fn credential_lengths() {
        assert!(Credentials::new("a".into(), "b".into()).is_ok());
        assert!(Credentials::new("a".repeat(255), "b".repeat(255)).is_ok());
        for (u, p) in [("", "b"), ("a", ""), (&*"a".repeat(256), "b"), ("a", &*"b".repeat(256))] {
            assert_eq!(Credentials::new(u.into(), p.into()), Err(SocksError::BadCredentials));
        }
        // multi-byte characters count in bytes, as the wire format does
        assert!(Credentials::new("é".repeat(128), "b".into()).is_err());
    }

    #[test]
    fn credentials_never_print() {
        let c = Credentials::new("secret-user".into(), "secret-pass".into()).unwrap();
        let shown = format!("{c:?} {:?}", Some(&c));
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[test]
    fn target_from_host() {
        assert_eq!(
            Target::from_host("10.0.0.1", 80),
            Some(Target::Addr("10.0.0.1:80".parse().unwrap()))
        );
        assert_eq!(
            Target::from_host("[2001:db8::1]", 80),
            Some(Target::Addr("[2001:db8::1]:80".parse().unwrap()))
        );
        assert_eq!(Target::from_host("host", 80), Some(Target::Domain("host".into(), 80)));
        assert_eq!(Target::from_host("", 80), None);
        assert_eq!(Target::from_host(&"a".repeat(256), 80), None);
    }

    #[test]
    fn blocking_driver_over_a_scripted_stream() {
        struct Script {
            input: std::io::Cursor<Vec<u8>>,
            written: Vec<u8>,
        }
        impl std::io::Read for Script {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.input.read(buf)
            }
        }
        impl std::io::Write for Script {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.written.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut ok = Script {
            input: std::io::Cursor::new(vec![5, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0, 0]),
            written: Vec::new(),
        };
        let target = Target::from_host("example.org", 443).unwrap();
        assert!(run_blocking(&mut ok, Handshake::new(None, Command::Connect(target.clone()))).is_ok());
        // a server that stops halfway is a closed connection, not a success
        let mut cut = Script {
            input: std::io::Cursor::new(vec![5, 0, 5, 0]),
            written: Vec::new(),
        };
        assert_eq!(
            run_blocking(&mut cut, Handshake::new(None, Command::Connect(target))),
            Err(SocksError::ConnectError)
        );
    }
}
