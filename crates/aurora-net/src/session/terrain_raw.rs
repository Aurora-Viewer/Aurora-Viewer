//! The RAW terrain file of Region / Estate › Terrain (LLPanelRegionTerrainInfo
//! onDownloadRawFilepickerCB / onUploadRawFilepickerCB in
//! newview/llfloaterregioninfo.cpp, process_initiate_download in
//! llviewermessage.cpp, LLXferManager::requestFile / processFileRequest in
//! llmessage/llxfermanager.cpp, originally LGPL 2.1). The window names a
//! local path in `terrain` "download filename" / "upload filename"; for a
//! download the simulator answers InitiateDownload with that path and its
//! own file, which the viewer asks by Xfer; for an upload it asks the path
//! with RequestXfer and the viewer sends the file's bytes. Only the path
//! the window named is accepted (validateFileForRequest /
//! validateFileForTransfer); the bytes go to and come from the window, which
//! reads and writes the file.

use super::inventory_upload::packet;
use super::{Session, emit};
use crate::estate::{EstateEvent, RawTransfer};
use crate::types::{NetEvent, RegionHandle};
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg, field_str};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The last packet's flag of an Xfer packet number.
const LAST_PACKET: u32 = 0x8000_0000;
/// LL_XFER_CHUNK_SIZE, LL_XFER_LARGE_PAYLOAD.
const CHUNK: usize = 1000;
const LARGE_CHUNK: usize = 7680;
/// A RAW file of a 256 m region is 851,968 bytes; a var region's is larger.
const MAX_RAW_FILE: usize = 64 << 20;
/// How long the simulator may take to start (the Legacy Viewer waits
/// forever; the window would stay busy).
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// An unconfirmed packet is sent again after this, three times.
const RESEND: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(super) struct RawXfers {
    download: Option<Download>,
    upload: Option<Upload>,
}

struct Download {
    handle: RegionHandle,
    path: String,
    asked: Instant,
    xfer: Option<Incoming>,
}

struct Incoming {
    id: u64,
    sim: SocketAddr,
    next: u32,
    size: usize,
    data: Vec<u8>,
    last: Instant,
}

struct Upload {
    handle: RegionHandle,
    path: String,
    data: Arc<Vec<u8>>,
    asked: Instant,
    out: Option<Outgoing>,
}

struct Outgoing {
    id: u64,
    sim: SocketAddr,
    chunk: usize,
    packet: u32,
    sent: Instant,
    retries: u8,
}

impl Outgoing {
    /// Every packet confirmed.
    fn done(&self, len: usize) -> bool {
        // the first packet carries 4 bytes of size more, not counted here
        (self.packet as usize) * self.chunk >= len.max(1)
    }
}

impl Session<'_> {
    /// "Download RAW…" / "Upload RAW…": the window's path in `terrain`, with
    /// the last invoice.
    pub(super) fn terrain_raw(&mut self, handle: RegionHandle, transfer: RawTransfer) {
        let Some(addr) = self.sim_for_handle(handle) else {
            return;
        };
        let invoice = self.estate_invoice;
        let now = Instant::now();
        let (verb, path) = match transfer {
            RawTransfer::Download { path } => {
                self.terrain_raw.download = Some(Download {
                    handle,
                    path: path.clone(),
                    asked: now,
                    xfer: None,
                });
                ("download filename", path)
            }
            RawTransfer::Upload { path, data } => {
                self.terrain_raw.upload = Some(Upload {
                    handle,
                    path: path.clone(),
                    data,
                    asked: now,
                    out: None,
                });
                ("upload filename", path)
            }
        };
        self.send_estate_owner_message(addr, "terrain", invoice, &[verb.to_owned(), path], true);
    }

    fn raw_downloaded(&mut self, result: Result<Vec<u8>, String>) {
        if let Some(d) = self.terrain_raw.download.take() {
            emit(
                self.sh,
                NetEvent::Estate(EstateEvent::RawDownloaded {
                    handle: d.handle,
                    path: d.path,
                    result: result.map(Arc::new),
                }),
            );
        }
    }

    fn raw_uploaded(&mut self, result: Result<(), String>) {
        if let Some(u) = self.terrain_raw.upload.take() {
            emit(self.sh, NetEvent::Estate(EstateEvent::RawUploaded { handle: u.handle, result }));
        }
    }

    fn send_raw_packet(&mut self) {
        let Some(u) = self.terrain_raw.upload.as_ref() else {
            return;
        };
        let Some(out) = &u.out else {
            return;
        };
        let Some((index, data)) = packet(&u.data, out.chunk, out.packet) else {
            return;
        };
        let mut m = SendXferPacket::default();
        m.xfer_id.id = out.id;
        m.xfer_id.packet = index;
        m.data_packet.data = data;
        let sim = out.sim;
        self.send(sim, &m, false);
    }

    /// The Xfer messages of the RAW file; Ok(false) when not ours.
    pub(super) fn dispatch_terrain_raw(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        if pkt.id == InitiateDownload::ID {
            let m: InitiateDownload = pkt.decode()?;
            if m.agent_data.agent_id != self.agent_id() {
                log::warn!("initiate download for another agent");
                return Ok(true);
            }
            let viewer = field_str(&m.file_data.viewer_filename);
            let Some(d) = self.terrain_raw.download.as_mut().filter(|d| d.path == viewer && d.xfer.is_none()) else {
                log::warn!("unrequested download to a local file refused");
                return Ok(true);
            };
            let id = Uuid::new_v4().as_u128() as u64;
            d.xfer = Some(Incoming {
                id,
                sim: from,
                next: 0,
                size: 0,
                data: Vec::new(),
                last: Instant::now(),
            });
            // requestFile(viewer, sim, LL_PATH_NONE, host, false, …)
            let mut r = RequestXfer::default();
            r.xfer_id.id = id;
            r.xfer_id.filename = m.file_data.sim_filename;
            r.xfer_id.file_path = 0;
            r.xfer_id.delete_on_completion = false;
            r.xfer_id.use_big_packets = false;
            r.xfer_id.v_file_id = Uuid::nil();
            r.xfer_id.v_file_type = -1;
            self.send(from, &r, true);
            log::info!("raw terrain: downloading");
            return Ok(true);
        }
        if pkt.id == SendXferPacket::ID {
            let m: SendXferPacket = pkt.decode()?;
            let Some(x) = self
                .terrain_raw
                .download
                .as_mut()
                .and_then(|d| d.xfer.as_mut())
                .filter(|x| x.id == m.xfer_id.id && x.sim == from)
            else {
                return Ok(false);
            };
            let num = m.xfer_id.packet & !LAST_PACKET;
            let id = x.id;
            if num != x.next {
                // a packet sent again: its confirmation was lost
                if num + 1 == x.next {
                    self.confirm_xfer(from, id, num);
                }
                return Ok(true);
            }
            let data = &m.data_packet.data[..];
            if num == 0 {
                // the file's size first (S32, little endian)
                let Some(size) = data.get(..4) else {
                    self.raw_downloaded(Err(aurora_i18n::tr!("net-estate-raw-incomplete").into()));
                    return Ok(true);
                };
                x.size = i32::from_le_bytes([size[0], size[1], size[2], size[3]]).max(0) as usize;
                x.data.extend_from_slice(&data[4..]);
            } else {
                x.data.extend_from_slice(data);
            }
            x.next += 1;
            x.last = Instant::now();
            let too_big = x.data.len() > MAX_RAW_FILE;
            let last = m.xfer_id.packet & LAST_PACKET != 0;
            self.confirm_xfer(from, id, num);
            if too_big {
                self.raw_downloaded(Err(aurora_i18n::tr!("net-estate-raw-incomplete").into()));
            } else if last {
                let x = self.terrain_raw.download.as_mut().and_then(|d| d.xfer.take());
                let result = match x {
                    Some(x) if x.data.len() == x.size => Ok(x.data),
                    _ => Err(aurora_i18n::tr!("net-estate-raw-incomplete").into()),
                };
                log::info!(
                    "raw terrain: download finished ({})",
                    if result.is_ok() { "ok" } else { "incomplete" }
                );
                self.raw_downloaded(result);
            }
            return Ok(true);
        }
        if pkt.id == RequestXfer::ID {
            let m: RequestXfer = pkt.decode()?;
            let x = &m.xfer_id;
            let Some(u) = self.terrain_raw.upload.as_mut() else {
                return Ok(false);
            };
            // validateFileForTransfer: the path the window named, LL_PATH_NONE
            if !x.v_file_id.is_nil() || x.file_path != 0 || field_str(&x.filename) != u.path {
                return Ok(false);
            }
            u.out = Some(Outgoing {
                id: x.id,
                sim: from,
                chunk: if x.use_big_packets { LARGE_CHUNK } else { CHUNK },
                packet: 0,
                sent: Instant::now(),
                retries: 0,
            });
            log::info!("raw terrain: uploading {} bytes", u.data.len());
            self.send_raw_packet();
            return Ok(true);
        }
        if pkt.id == ConfirmXferPacket::ID {
            let m: ConfirmXferPacket = pkt.decode()?;
            let Some(u) = self.terrain_raw.upload.as_mut() else {
                return Ok(false);
            };
            let len = u.data.len();
            let Some(out) = u.out.as_mut().filter(|o| o.id == m.xfer_id.id) else {
                return Ok(false);
            };
            if m.xfer_id.packet & !LAST_PACKET != out.packet {
                return Ok(true);
            }
            out.packet += 1;
            out.sent = Instant::now();
            out.retries = 0;
            if out.done(len) {
                log::info!("raw terrain: upload finished");
                self.raw_uploaded(Ok(()));
            } else {
                self.send_raw_packet();
            }
            return Ok(true);
        }
        if pkt.id == AbortXfer::ID {
            let m: AbortXfer = pkt.decode()?;
            let id = m.xfer_id.id;
            if self
                .terrain_raw
                .download
                .as_ref()
                .and_then(|d| d.xfer.as_ref())
                .is_some_and(|x| x.id == id)
            {
                log::warn!("raw terrain: download aborted by the simulator ({})", m.xfer_id.result);
                self.raw_downloaded(Err(aurora_i18n::tr!("net-estate-raw-aborted").into()));
                return Ok(true);
            }
            if self
                .terrain_raw
                .upload
                .as_ref()
                .and_then(|u| u.out.as_ref())
                .is_some_and(|o| o.id == id)
            {
                log::warn!("raw terrain: upload aborted by the simulator ({})", m.xfer_id.result);
                self.raw_uploaded(Err(aurora_i18n::tr!("net-estate-raw-aborted").into()));
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn confirm_xfer(&mut self, to: SocketAddr, id: u64, packet: u32) {
        let mut c = ConfirmXferPacket::default();
        c.xfer_id.id = id;
        c.xfer_id.packet = packet;
        self.send(to, &c, false);
    }

    /// A transfer the simulator never started or stopped answering ends
    /// with an error; an unconfirmed packet goes again (LLXfer resend).
    pub(super) fn expire_terrain_raw(&mut self) {
        let now = Instant::now();
        let stalled = |asked: Instant, last: Option<Instant>| now.duration_since(last.unwrap_or(asked)) > START_TIMEOUT;
        if let Some(d) = &self.terrain_raw.download
            && stalled(d.asked, d.xfer.as_ref().map(|x| x.last))
        {
            log::warn!("raw terrain: download timed out");
            self.raw_downloaded(Err(aurora_i18n::tr!("net-estate-raw-timeout").into()));
        }
        let mut resend = false;
        let mut failed = false;
        if let Some(u) = self.terrain_raw.upload.as_mut() {
            match u.out.as_mut() {
                None => failed = now.duration_since(u.asked) > START_TIMEOUT,
                Some(o) if now.duration_since(o.sent) > RESEND => {
                    if o.retries >= 3 {
                        failed = true;
                    } else {
                        o.retries += 1;
                        o.sent = now;
                        resend = true;
                    }
                }
                Some(_) => {}
            }
        }
        if failed {
            log::warn!("raw terrain: upload timed out");
            self.raw_uploaded(Err(aurora_i18n::tr!("net-estate-raw-timeout").into()));
        } else if resend {
            self.send_raw_packet();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2,500 bytes in 1,000-byte packets: three packets, the last flagged,
    /// the first with the size before.
    #[test]
    fn the_file_goes_in_numbered_packets() {
        let data = vec![7u8; 2500];
        let (i0, p0) = packet(&data, CHUNK, 0).expect("first");
        assert_eq!((i0, p0.len()), (0, 1004));
        assert_eq!(&p0[..4], &2500i32.to_le_bytes());
        let (i2, p2) = packet(&data, CHUNK, 2).expect("last");
        assert_eq!((i2, p2.len()), (2 | LAST_PACKET, 500));
        assert!(packet(&data, CHUNK, 3).is_none());
        let out = |packet| Outgoing {
            id: 1,
            sim: "127.0.0.1:1".parse().expect("address"),
            chunk: CHUNK,
            packet,
            sent: Instant::now(),
            retries: 0,
        };
        assert!(!out(2).done(2500));
        assert!(out(3).done(2500));
    }
}
