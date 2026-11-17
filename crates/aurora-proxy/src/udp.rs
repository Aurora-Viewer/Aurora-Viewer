//! The circuits' UDP through a SOCKS 5 relay: the UDP ASSOCIATE handshake
//! over a TCP control channel kept open for the whole session, and the
//! header of each datagram (RFC 1928 §7).
//!
//! Port of LLProxy::startSOCKSProxy and LLMessageSystem::sendPacketToSocket /
//! receivePacket (indra/llmessage/llproxy.cpp, message.cpp, originally LGPL
//! 2.1), stricter:
//! - the relay is the proxy's own address with the port of the reply (as the
//!   Legacy Viewer), so a reply cannot point the circuits at another host;
//! - an incoming datagram is accepted only from the relay, with a zero
//!   RSV and FRAG and an IPv4 or IPv6 source (the Legacy Viewer takes any
//!   datagram of more than 10 bytes from anyone and assumes IPv4);
//! - the end of the control channel ends the association (`closed`), where
//!   the Legacy Viewer's UDP silently stops.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

use crate::socks5::{ATYP_DOMAIN, ATYP_IPV4, ATYP_IPV6, Bound, Command, Credentials, Handshake, SocksError, run_async};

/// The header of an IPv4 datagram: RSV (2), FRAG, ATYP, address (4), port (2).
pub const HEADER_V4: usize = 10;
/// The header of an IPv6 datagram.
pub const HEADER_V6: usize = 22;

/// How long the connection to the proxy and the handshake may take. The
/// Legacy Viewer gives each exchange 100 ms (tcp_blocking_handshake), which a
/// distant proxy does not always meet; a hung proxy still ends the login.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Writes the relay header for `dst`, then `payload`, into `out` (cleared).
pub fn wrap(dst: SocketAddr, payload: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&[0, 0, 0]);
    match dst.ip() {
        IpAddr::V4(ip) => {
            out.push(ATYP_IPV4);
            out.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(ATYP_IPV6);
            out.extend_from_slice(&ip.octets());
        }
    }
    out.extend_from_slice(&dst.port().to_be_bytes());
    out.extend_from_slice(payload);
}

/// Why a datagram from the relay is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    Short,
    Reserved,
    /// A fragment: the viewer never asks for fragmentation (RFC 1928 lets an
    /// implementation that does not support it drop them).
    Fragment,
    /// A name or an unknown type as the source: a simulator is an address.
    AddressType,
    Empty,
}

/// Reads the relay header: the simulator that sent the datagram and its
/// payload.
pub fn unwrap(datagram: &[u8]) -> Result<(SocketAddr, &[u8]), Dropped> {
    let [r0, r1, frag, atyp, rest @ ..] = datagram else {
        return Err(Dropped::Short);
    };
    if *r0 != 0 || *r1 != 0 {
        return Err(Dropped::Reserved);
    }
    if *frag != 0 {
        return Err(Dropped::Fragment);
    }
    let (ip, rest) = match *atyp {
        ATYP_IPV4 => {
            let (ip, rest) = rest.split_first_chunk::<4>().ok_or(Dropped::Short)?;
            (IpAddr::V4(Ipv4Addr::from(*ip)), rest)
        }
        ATYP_IPV6 => {
            let (ip, rest) = rest.split_first_chunk::<16>().ok_or(Dropped::Short)?;
            (IpAddr::V6(Ipv6Addr::from(*ip)), rest)
        }
        ATYP_DOMAIN => return Err(Dropped::AddressType),
        _ => return Err(Dropped::AddressType),
    };
    let (port, payload) = rest.split_first_chunk::<2>().ok_or(Dropped::Short)?;
    if payload.is_empty() {
        return Err(Dropped::Empty);
    }
    // an IPv4 simulator relayed as an IPv4-mapped IPv6 address is the same
    // circuit
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    Ok((SocketAddr::new(ip, u16::from_be_bytes(*port)), payload))
}

/// A granted UDP association. The relay lives as long as `control`: drop it
/// and the proxy closes the relay.
pub struct UdpAssociation {
    control: TcpStream,
    relay: SocketAddr,
    /// The relay's own address in the reply (BND.ADDR), when it names one
    /// that differs: a proxy with several addresses may answer from it.
    announced: Option<SocketAddr>,
}

impl UdpAssociation {
    /// Connects to the proxy at `proxy` (already resolved) and asks for a
    /// relay.
    pub async fn open(proxy: SocketAddr, credentials: Option<Credentials>) -> Result<Self, SocksError> {
        let mut control = tokio::time::timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(proxy))
            .await
            .map_err(|_| SocksError::HostConnectFailed)?
            .map_err(|_| SocksError::HostConnectFailed)?;
        let _ = control.set_nodelay(true);
        let bound = run_async(&mut control, Handshake::new(credentials, Command::UdpAssociate), HANDSHAKE_TIMEOUT).await?;
        if bound.port() == 0 {
            return Err(SocksError::Protocol);
        }
        // LLProxy::proxyHandshake: the proxy's address, the reply's port
        let relay = SocketAddr::new(proxy.ip(), bound.port());
        Ok(Self {
            control,
            relay,
            announced: announced(&bound, relay),
        })
    }

    /// Where the datagrams are sent.
    pub fn relay(&self) -> SocketAddr {
        self.relay
    }

    /// Another address the relay's datagrams may come from (its BND.ADDR).
    pub fn announced(&self) -> Option<SocketAddr> {
        self.announced
    }

    /// Completes when the proxy ends the association: the control channel
    /// closes, fails, or carries anything (nothing is expected on it after
    /// the handshake). Cancel-safe, for a `select!` loop.
    pub async fn closed(&mut self) {
        let mut byte = [0u8; 1];
        let _ = self.control.read(&mut byte).await;
    }
}

/// The reply's BND.ADDR, when it is a real address other than the relay's:
/// not unspecified (0.0.0.0, `::`), not a name. IPv4-mapped IPv6 becomes
/// IPv4, as the senders `unwrap` returns.
fn announced(bound: &Bound, relay: SocketAddr) -> Option<SocketAddr> {
    let Bound::Addr(addr) = bound else { return None };
    let ip = match addr.ip() {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        ip => ip,
    };
    let addr = SocketAddr::new(ip, addr.port());
    (!ip.is_unspecified() && addr != relay).then_some(addr)
}

impl std::fmt::Debug for UdpAssociation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // the relay's address is the proxy's: kept out of logs
        f.write_str("UdpAssociation")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_then_unwrap() {
        let mut out = Vec::new();
        let dst: SocketAddr = "203.0.113.7:13005".parse().unwrap();
        wrap(dst, b"hello", &mut out);
        assert_eq!(&out[..HEADER_V4], &[0, 0, 0, 1, 203, 0, 113, 7, 0x32, 0xCD]);
        assert_eq!(unwrap(&out), Ok((dst, &b"hello"[..])));
        let v6: SocketAddr = "[2001:db8::2]:9000".parse().unwrap();
        wrap(v6, b"x", &mut out);
        assert_eq!(out.len(), HEADER_V6 + 1);
        assert_eq!(unwrap(&out), Ok((v6, &b"x"[..])));
    }

    #[test]
    fn mapped_v4_is_the_v4_circuit() {
        let mut d = vec![0, 0, 0, 4];
        d.extend_from_slice(&"::ffff:203.0.113.7".parse::<Ipv6Addr>().unwrap().octets());
        d.extend_from_slice(&[0, 80, 9]);
        assert_eq!(unwrap(&d), Ok(("203.0.113.7:80".parse().unwrap(), &[9u8][..])));
    }

    #[test]
    fn hostile_datagrams_are_dropped() {
        let ok = [0u8, 0, 0, 1, 1, 2, 3, 4, 0, 80, 0xAA];
        assert!(unwrap(&ok).is_ok());
        let mut cases: Vec<(Vec<u8>, Dropped)> = vec![
            (vec![], Dropped::Short),
            (vec![0, 0, 0], Dropped::Short),
            (ok[..9].to_vec(), Dropped::Short),
            (ok[..10].to_vec(), Dropped::Empty),
            (vec![0, 0, 0, 4, 1, 2, 3], Dropped::Short),
            (vec![0, 0, 0, 3, 3, b'a', b'b', b'c', 0, 80, 1], Dropped::AddressType),
            (vec![0, 0, 0, 9, 1, 2, 3, 4, 0, 80, 1], Dropped::AddressType),
        ];
        let mut reserved = ok.to_vec();
        reserved[1] = 1;
        cases.push((reserved, Dropped::Reserved));
        let mut fragment = ok.to_vec();
        fragment[2] = 1;
        cases.push((fragment, Dropped::Fragment));
        for (datagram, why) in cases {
            assert_eq!(unwrap(&datagram), Err(why), "{datagram:?}");
        }
    }

    /// A SOCKS 5 server on loopback that grants one association, then closes
    /// the control channel when told.
    #[tokio::test]
    async fn association_against_a_local_server() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = listener.local_addr().unwrap();
        let (close_tx, close_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut greet = [0u8; 3];
            s.read_exact(&mut greet).await.unwrap();
            assert_eq!(greet, [5, 1, 2]);
            s.write_all(&[5, 2]).await.unwrap();
            let mut auth = [0u8; 9];
            s.read_exact(&mut auth).await.unwrap();
            assert_eq!(&auth, b"\x01\x03bob\x03pw!");
            s.write_all(&[1, 0]).await.unwrap();
            let mut request = [0u8; 10];
            s.read_exact(&mut request).await.unwrap();
            assert_eq!(request, [5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
            // the datagrams go to the proxy's address; a BND.ADDR elsewhere
            // is only another address they may come back from
            s.write_all(&[5, 0, 0, 1, 198, 51, 100, 9, 0x1F, 0x90]).await.unwrap();
            let _ = close_rx.await;
        });
        let creds = Credentials::new("bob".into(), "pw!".into()).unwrap();
        let mut association = UdpAssociation::open(proxy, Some(creds)).await.unwrap();
        assert_eq!(association.relay(), SocketAddr::new(proxy.ip(), 8080));
        assert_eq!(association.announced(), Some("198.51.100.9:8080".parse().unwrap()));
        let _ = close_tx.send(());
        tokio::time::timeout(Duration::from_secs(5), association.closed())
            .await
            .expect("closed");
        server.await.unwrap();
    }

    #[test]
    fn announced_addresses() {
        let relay: SocketAddr = "203.0.113.1:7000".parse().unwrap();
        let addr = |a: &str| Bound::Addr(a.parse().unwrap());
        assert_eq!(announced(&addr("0.0.0.0:7000"), relay), None);
        assert_eq!(announced(&addr("[::]:7000"), relay), None);
        assert_eq!(announced(&addr("203.0.113.1:7000"), relay), None);
        assert_eq!(announced(&Bound::Domain(7000), relay), None);
        assert_eq!(
            announced(&addr("[::ffff:198.51.100.2]:7000"), relay),
            Some("198.51.100.2:7000".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn refused_association() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut greet = [0u8; 3];
            s.read_exact(&mut greet).await.unwrap();
            s.write_all(&[5, 0]).await.unwrap();
            let mut request = [0u8; 10];
            s.read_exact(&mut request).await.unwrap();
            s.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
        });
        assert_eq!(UdpAssociation::open(proxy, None).await.err(), Some(SocksError::Refused(7)));
    }

    #[tokio::test]
    async fn silent_server_times_out_and_closed_port_fails() {
        // nothing listens: the connection fails
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert_eq!(UdpAssociation::open(free, None).await.err(), Some(SocksError::HostConnectFailed));
        // a server that accepts and closes is a connect error
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            drop(s);
        });
        assert_eq!(UdpAssociation::open(proxy, None).await.err(), Some(SocksError::ConnectError));
    }
}
