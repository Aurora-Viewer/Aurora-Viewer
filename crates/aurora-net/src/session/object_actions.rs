//! LLToolGrab messages, LLViewerObject task inventory cap / legacy Xfer
//! (Firestorm indra/newview, originally LGPL 2.1).
use super::{Session, emit};
use crate::{NetCommand, NetEvent, task_inventory};
use aurora_msg::{IncomingPacket, Msg, field_str, msgs::*, str_field};
use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, Instant},
};
use uuid::Uuid;

const LAST: u32 = 0x8000_0000;
const MAX_FILE: usize = 8 * 1024 * 1024;
#[derive(Default)]
pub(super) struct TaskRequests {
    pending: HashMap<Uuid, (SocketAddr, Instant)>,
    xfers: HashMap<u64, TaskXfer>,
}
struct TaskXfer {
    object: Uuid,
    /// ReplyTaskInventory serial of the listing.
    serial: i16,
    sim: SocketAddr,
    next: u32,
    size: usize,
    data: Vec<u8>,
    last: Instant,
}

impl TaskXfer {
    /// Duplicate packets are acknowledged without appending twice. Out of
    /// order packets wait for retransmission; packet 0 carries a LE size.
    fn append(&mut self, packet: u32, data: &[u8]) -> Option<Result<bool, ()>> {
        let num = packet & !LAST;
        if num > self.next {
            return None;
        }
        self.last = Instant::now();
        if num == self.next {
            let data = if num == 0 {
                let Some(prefix) = data.get(..4) else { return Some(Err(())) };
                self.size = u32::from_le_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) as usize;
                &data[4..]
            } else {
                data
            };
            if self.size > MAX_FILE || self.data.len() + data.len() > self.size {
                return Some(Err(()));
            }
            self.data.extend_from_slice(data);
            self.next += 1;
        }
        Some(Ok(packet & LAST != 0 && num + 1 == self.next))
    }
}

impl Session<'_> {
    pub(super) fn expire_task_inventory(&mut self) {
        let mut expired = Vec::new();
        self.tasks.pending.retain(|id, (_, last)| {
            let keep = last.elapsed() < Duration::from_secs(30);
            if !keep {
                expired.push(*id);
            }
            keep
        });
        self.tasks.xfers.retain(|_, x| {
            let keep = x.last.elapsed() < Duration::from_secs(30);
            if !keep {
                expired.push(x.object);
            }
            keep
        });
        for object in expired {
            emit(
                self.sh,
                NetEvent::TaskInventory {
                    object,
                    serial: None,
                    result: Err("Le simulateur n'a pas répondu à temps.".into()),
                },
            );
        }
    }
    pub(super) fn on_object_action_command(&mut self, c: &NetCommand) -> bool {
        match c {
            NetCommand::ObjectGrab {
                handle,
                local_id,
                offset,
                surface,
            } => {
                if let Some(addr) = self.sim_for_handle(*handle) {
                    let mut m = ObjectGrab::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.object_data.local_id = *local_id;
                    m.object_data.grab_offset = *offset;
                    m.surface_info.push(object_grab::SurfaceInfo {
                        uv_coord: surface.uv,
                        st_coord: surface.st,
                        face_index: surface.face,
                        position: surface.position,
                        normal: surface.normal,
                        binormal: surface.binormal,
                    });
                    self.send(addr, &m, true);
                }
            }
            NetCommand::ObjectGrabUpdate {
                handle,
                object,
                offset,
                position,
                elapsed_ms,
                surface,
            } => {
                if let Some(addr) = self.sim_for_handle(*handle) {
                    let mut m = ObjectGrabUpdate::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.object_data.object_id = *object;
                    m.object_data.grab_offset_initial = *offset;
                    m.object_data.grab_position = *position;
                    m.object_data.time_since_last = *elapsed_ms;
                    m.surface_info.push(object_grab_update::SurfaceInfo {
                        uv_coord: surface.uv,
                        st_coord: surface.st,
                        face_index: surface.face,
                        position: surface.position,
                        normal: surface.normal,
                        binormal: surface.binormal,
                    });
                    self.send(addr, &m, false);
                }
            }
            NetCommand::ObjectRelease { handle, local_id, surface } => {
                if let Some(addr) = self.sim_for_handle(*handle) {
                    let mut m = ObjectDeGrab::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.object_data.local_id = *local_id;
                    m.surface_info.push(object_de_grab::SurfaceInfo {
                        uv_coord: surface.uv,
                        st_coord: surface.st,
                        face_index: surface.face,
                        position: surface.position,
                        normal: surface.normal,
                        binormal: surface.binormal,
                    });
                    self.send(addr, &m, true);
                }
            }
            NetCommand::RequestTaskInventory { handle, local_id, object } => self.request_task_inventory(*handle, *local_id, *object),
            _ => return false,
        }
        true
    }

    pub(super) fn request_task_inventory(&mut self, handle: u64, local_id: u32, object: Uuid) {
        let Some(addr) = self.sim_for_handle(handle) else { return };
        if let Some(url) = self.sims.get(&addr).and_then(|s| s.caps.get("RequestTaskInventory")).cloned() {
            let http = self.sh.caps_http.clone();
            let events = self.sh.events.clone();
            tokio::spawn(async move {
                let result = async {
                    let mut url = reqwest::Url::parse(&url).map_err(|_| "Capability d'inventaire invalide.".to_string())?;
                    url.query_pairs_mut().append_pair("task_id", &object.to_string());
                    let mut response = http
                        .get(url)
                        .timeout(Duration::from_secs(30))
                        .header("Accept", "application/llsd+xml")
                        .send()
                        .await
                        .map_err(|_| "Impossible de charger l'inventaire de l'objet.".to_string())?;
                    if !response.status().is_success() {
                        return Err(format!("Inventaire refusé (HTTP {}).", response.status().as_u16()));
                    }
                    if response.content_length().is_some_and(|n| n > MAX_FILE as u64) {
                        return Err("Inventaire trop volumineux.".into());
                    }
                    let mut data = Vec::new();
                    while let Some(chunk) = response.chunk().await.map_err(|_| "Réponse d'inventaire illisible.".to_string())? {
                        if data.len() + chunk.len() > MAX_FILE {
                            return Err("Inventaire trop volumineux.".into());
                        }
                        data.extend_from_slice(&chunk);
                    }
                    let doc = aurora_llsd::from_xml(&data).map_err(|_| "Réponse d'inventaire invalide.".to_string())?;
                    Ok((task_inventory::cap_serial(&doc), task_inventory::parse_cap(&doc)?))
                }
                .await;
                let (serial, result) = match result {
                    Ok((serial, items)) => (serial, Ok(items)),
                    Err(e) => (None, Err(e)),
                };
                let _ = events.send(NetEvent::TaskInventory { object, serial, result });
            });
        } else {
            if self.tasks.pending.len() >= 32 {
                emit(
                    self.sh,
                    NetEvent::TaskInventory {
                        object,
                        serial: None,
                        result: Err("Trop de requêtes d'inventaire en cours.".into()),
                    },
                );
                return;
            }
            self.tasks.pending.insert(object, (addr, Instant::now()));
            let mut m = RequestTaskInventory::default();
            m.agent_data.agent_id = self.agent_id();
            m.agent_data.session_id = self.session_id();
            m.inventory_data.local_id = local_id;
            self.send(addr, &m, true);
        }
    }

    pub(super) fn dispatch_object_actions(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        if pkt.id == ReplyTaskInventory::ID {
            let m: ReplyTaskInventory = pkt.decode()?;
            let object = m.inventory_data.task_id;
            if !self.tasks.pending.get(&object).is_some_and(|(addr, _)| *addr == from) {
                return Ok(true);
            }
            self.tasks.pending.remove(&object);
            let filename = field_str(&m.inventory_data.filename);
            let serial = m.inventory_data.serial;
            if filename.is_empty() {
                emit(
                    self.sh,
                    NetEvent::TaskInventory {
                        object,
                        serial: Some(serial),
                        result: Ok(Vec::new()),
                    },
                );
            } else if self.tasks.xfers.len() < 16 {
                let id = Uuid::new_v4().as_u128() as u64;
                self.tasks.xfers.insert(
                    id,
                    TaskXfer {
                        object,
                        serial,
                        sim: from,
                        next: 0,
                        size: 0,
                        data: Vec::new(),
                        last: Instant::now(),
                    },
                );
                let mut m = RequestXfer::default();
                m.xfer_id.id = id;
                m.xfer_id.filename = str_field(&filename);
                m.xfer_id.file_path = 4; // LL_PATH_CACHE on the simulator, never a local path
                m.xfer_id.delete_on_completion = true;
                m.xfer_id.v_file_type = -1;
                self.send(from, &m, true);
            } else {
                emit(
                    self.sh,
                    NetEvent::TaskInventory {
                        object,
                        serial: None,
                        result: Err("Trop de transferts d'inventaire en cours.".into()),
                    },
                );
            }
            return Ok(true);
        }
        if pkt.id == SendXferPacket::ID {
            let m: SendXferPacket = pkt.decode()?;
            let Some(x) = self.tasks.xfers.get_mut(&m.xfer_id.id).filter(|x| x.sim == from) else {
                return Ok(false);
            };
            let num = m.xfer_id.packet & !LAST;
            let Some(result) = x.append(m.xfer_id.packet, &m.data_packet.data) else {
                return Ok(true);
            };
            let mut ack = ConfirmXferPacket::default();
            ack.xfer_id.id = m.xfer_id.id;
            ack.xfer_id.packet = num;
            self.send(from, &ack, false);
            if !matches!(result, Ok(false))
                && let Some(x) = self.tasks.xfers.remove(&m.xfer_id.id)
            {
                let result = if result.is_ok() && x.data.len() == x.size {
                    task_inventory::parse_legacy(&x.data)
                } else {
                    Err("Transfert d'inventaire incomplet ou invalide.".into())
                };
                emit(
                    self.sh,
                    NetEvent::TaskInventory {
                        object: x.object,
                        serial: Some(x.serial),
                        result,
                    },
                );
            }
            return Ok(true);
        }
        if pkt.id == AbortXfer::ID {
            let m: AbortXfer = pkt.decode()?;
            if self.tasks.xfers.get(&m.xfer_id.id).is_some_and(|x| x.sim == from)
                && let Some(x) = self.tasks.xfers.remove(&m.xfer_id.id)
            {
                emit(
                    self.sh,
                    NetEvent::TaskInventory {
                        object: x.object,
                        serial: None,
                        result: Err("Transfert d'inventaire annulé par le simulateur.".into()),
                    },
                );
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn xfer() -> TaskXfer {
        TaskXfer {
            object: Uuid::nil(),
            serial: 0,
            sim: SocketAddr::from(([127, 0, 0, 1], 1)),
            next: 0,
            size: 0,
            data: Vec::new(),
            last: Instant::now(),
        }
    }
    #[test]
    fn inventory_transfer_duplicate_order_last_and_bounds() {
        let mut x = xfer();
        let first = [3, 0, 0, 0, b'a'];
        assert_eq!(x.append(1, b"bc"), None);
        assert_eq!(x.append(0, &first), Some(Ok(false)));
        assert_eq!(x.append(0, &first), Some(Ok(false)));
        assert_eq!(x.data, b"a");
        assert_eq!(x.append(LAST | 1, b"bc"), Some(Ok(true)));
        assert_eq!(x.data, b"abc");
        assert_eq!(xfer().append(0, b"ab"), Some(Err(())));
        assert_eq!(xfer().append(LAST, &[1, 0, 0, 0, b'a', b'b']), Some(Err(())));
        assert_eq!(xfer().append(0, &((MAX_FILE + 1) as u32).to_le_bytes()), Some(Err(())));
    }
}
