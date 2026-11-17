//! The circuits' UDP socket: direct, or through the SOCKS 5 relay of
//! Preferences › Network › Proxy (`aurora_proxy::udp`).
//!
//! Port of LLMessageSystem::sendPacketToSocket and receivePacket
//! (indra/llmessage/message.cpp, originally LGPL 2.1): through the relay each
//! datagram carries the SOCKS 5 UDP header. Stricter than the Legacy Viewer,
//! which takes any datagram of more than 10 bytes from anyone: only the
//! relay's datagrams are read, with a valid header, and a loopback simulator
//! (a local grid) stays direct.

use std::cell::RefCell;
use std::net::SocketAddr;
use std::ops::Range;
use std::sync::Arc;

use tokio::net::UdpSocket;

/// Where a circuit writes its datagrams: the session's `CircuitSocket` (a
/// plain socket only in the tests).
pub trait DatagramOut {
    fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<usize>;
}

impl DatagramOut for CircuitSocket {
    fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
        CircuitSocket::try_send_to(self, buf, addr)
    }
}

impl<T: DatagramOut + ?Sized> DatagramOut for Arc<T> {
    fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
        (**self).try_send_to(buf, addr)
    }
}

#[cfg(test)]
impl DatagramOut for UdpSocket {
    fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
        UdpSocket::try_send_to(self, buf, addr)
    }
}

pub struct CircuitSocket {
    sock: Arc<UdpSocket>,
    /// The SOCKS 5 relay, when the circuits go through the proxy.
    relay: Option<SocketAddr>,
}

thread_local! {
    /// The wrapped datagram being sent (no allocation per packet).
    static WRAPPED: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(1600));
}

impl CircuitSocket {
    pub fn direct(sock: Arc<UdpSocket>) -> Self {
        Self { sock, relay: None }
    }

    pub fn relayed(sock: Arc<UdpSocket>, relay: SocketAddr) -> Self {
        Self { sock, relay: Some(relay) }
    }

    #[cfg(test)]
    pub fn socket(&self) -> &UdpSocket {
        &self.sock
    }

    /// Sends one datagram to a simulator, through the relay when there is
    /// one. Returns the bytes written to the socket.
    pub fn try_send_to(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<usize> {
        match self.relay {
            Some(relay) if !addr.ip().is_loopback() => WRAPPED.with(|wrapped| {
                let mut wrapped = wrapped.borrow_mut();
                aurora_proxy::udp::wrap(addr, buf, &mut wrapped);
                self.sock.try_send_to(&wrapped, relay)
            }),
            _ => self.sock.try_send_to(buf, addr),
        }
    }

    /// Receives one datagram: the range of `buf` that holds the message and
    /// the simulator that sent it, or `None` for a datagram dropped here.
    pub async fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<Option<(Range<usize>, SocketAddr)>> {
        let (n, from) = self.sock.recv_from(buf).await?;
        Ok(self.accept(&buf[..n], from))
    }

    fn accept(&self, datagram: &[u8], from: SocketAddr) -> Option<(Range<usize>, SocketAddr)> {
        let Some(relay) = self.relay else {
            return Some((0..datagram.len(), from));
        };
        if from == relay {
            match aurora_proxy::udp::unwrap(datagram) {
                Ok((sender, payload)) => Some((datagram.len() - payload.len()..datagram.len(), sender)),
                Err(why) => {
                    log::trace!("relay datagram dropped: {why:?}");
                    None
                }
            }
        } else if from.ip().is_loopback() {
            Some((0..datagram.len(), from))
        } else {
            // around the relay: not one of our circuits' answers
            log::trace!("datagram from outside the relay dropped");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn socket() -> Arc<UdpSocket> {
        Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("socket"))
    }

    #[tokio::test]
    async fn direct_socket_is_unchanged() {
        let s = CircuitSocket::direct(socket().await);
        let from: SocketAddr = "203.0.113.1:13000".parse().unwrap();
        assert_eq!(s.accept(b"abc", from), Some((0..3, from)));
    }

    #[tokio::test]
    async fn relayed_datagrams() {
        let relay: SocketAddr = "198.51.100.5:7000".parse().unwrap();
        let s = CircuitSocket::relayed(socket().await, relay);
        let sim: SocketAddr = "203.0.113.1:13000".parse().unwrap();
        let mut wrapped = Vec::new();
        aurora_proxy::udp::wrap(sim, b"msg", &mut wrapped);
        let (range, sender) = s.accept(&wrapped, relay).expect("accepted");
        assert_eq!((&wrapped[range], sender), (&b"msg"[..], sim));
        // the same bytes from anyone else are not read
        assert_eq!(s.accept(&wrapped, sim), None);
        // a broken header from the relay is dropped
        assert_eq!(s.accept(&wrapped[..6], relay), None);
        // a loopback simulator stays direct
        let local: SocketAddr = "127.0.0.1:9000".parse().unwrap();
        assert_eq!(s.accept(b"x", local), Some((0..1, local)));
    }

    #[tokio::test]
    async fn sends_go_to_the_relay_wrapped() {
        let relay_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        // the relay is on loopback here; the simulator is not
        let s = CircuitSocket::relayed(socket().await, relay_sock.local_addr().unwrap());
        let sim: SocketAddr = "203.0.113.9:12035".parse().unwrap();
        s.socket().writable().await.unwrap();
        assert_eq!(s.try_send_to(b"hello", sim).unwrap(), aurora_proxy::udp::HEADER_V4 + 5);
        let mut buf = [0u8; 64];
        let (n, _) = relay_sock.recv_from(&mut buf).await.unwrap();
        assert_eq!(aurora_proxy::udp::unwrap(&buf[..n]), Ok((sim, &b"hello"[..])));
    }
}
