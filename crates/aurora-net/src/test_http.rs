//! Test support for the loopback HTTP servers of the unit tests.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How long a finished connection waits for the client to close its side.
const DRAIN: Duration = Duration::from_secs(5);

/// Ends a test server's connection once its reply is written, without
/// losing the reply.
///
/// Dropping the socket at once can reset the connection instead of closing
/// it: Windows sends a reset when unread request bytes are left behind, and
/// Windows 11 26H2 also resets loopback connections closed right after a
/// write (#1422). A reset discards the part of the reply the client has not
/// read yet, and the test fails on a reply it never saw. Here the write side
/// is shut (a clean end of the reply), then the socket is drained until the
/// client closes its side, which it does after reading the whole reply, or
/// for `DRAIN` at most. The wait runs in its own task, so the server goes on
/// to its next connection at once.
pub(crate) fn finish(mut socket: TcpStream) {
    tokio::spawn(async move {
        let _ = socket.shutdown().await;
        let mut sink = [0; 4096];
        let _ = tokio::time::timeout(DRAIN, async { while matches!(socket.read(&mut sink).await, Ok(n) if n > 0) {} }).await;
    });
}
