//! Session side of the About Land floater (land.rs): parcel selection and
//! properties update, access lists, dwell, object owners and returns,
//! release / deed / pass, the estate covenant (EstateCovenantRequest, then
//! the notecard by estate asset transfer) and the parcel environment.
//! Senders follow LLViewerParcelMgr and LLFloaterLand (llviewerparcelmgr.cpp,
//! llfloaterland.cpp, originally LGPL 2.1).

use super::{Session, emit};
use crate::land::{self, LandCommand, LandEvent};
use crate::types::{NetEvent, RegionHandle};
use aurora_llsd::Llsd;
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg, field_str, str_field};
use std::net::SocketAddr;
use uuid::Uuid;

/// LLTransferChannelType / LLTransferSourceType / LLTSCode values.
const LLTCT_ASSET: i32 = 2;
const LLTST_SIM_ESTATE: i32 = 4;
const LLTS_OK: i32 = 0;
const LLTS_DONE: i32 = 1;
/// Names asked per UUIDGroupNameRequest (the variable block holds 255).
const MAX_NAME_BLOCKS: usize = 50;

/// The covenant notecard being received.
#[derive(Debug)]
struct CovenantTransfer {
    transfer: Uuid,
    covenant: Uuid,
    data: land::TransferAssembly,
}

#[derive(Debug, Default)]
pub(super) struct LandNet {
    covenant: Option<CovenantTransfer>,
    /// AvatarPickerRequest queries waiting for their reply (query id, text).
    picker_queries: Vec<(Uuid, String)>,
}

impl Session<'_> {
    fn land_sim(&self, handle: RegionHandle) -> Option<SocketAddr> {
        self.sim_for_handle(handle)
    }

    fn land_cap(&self, handle: RegionHandle, name: &str) -> Option<String> {
        let addr = self.land_sim(handle)?;
        self.sims.get(&addr)?.caps.get(name).cloned()
    }

    pub(super) fn on_land_command(&mut self, c: LandCommand) {
        let (agent, session) = (self.agent_id(), self.session_id());
        match c {
            LandCommand::Select {
                handle,
                west,
                south,
                east,
                north,
                snap,
            } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelPropertiesRequest::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                let d = &mut m.parcel_data;
                d.sequence_id = land::SELECTED_PARCEL_SEQ_ID;
                d.west = west;
                d.south = south;
                d.east = east;
                d.north = north;
                d.snap_selection = snap;
                self.send(addr, &m, true);
            }
            LandCommand::Update { handle, update } => self.send_parcel_update(handle, &update),
            LandCommand::AccessListRequest { handle, local_id, flags } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                // FIRE-17280: experience lists only where experiences exist
                let mut flags = flags;
                if self.land_cap(handle, "RegionExperiences").is_none() {
                    flags &= !(land::AL_ALLOW_EXPERIENCE | land::AL_BLOCK_EXPERIENCE);
                }
                let mut m = ParcelAccessListRequest::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.sequence_id = 0;
                m.data.flags = flags;
                m.data.local_id = local_id;
                self.send(addr, &m, true);
            }
            LandCommand::AccessListUpdate {
                handle,
                local_id,
                flags,
                entries,
            } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let transaction = Uuid::new_v4();
                let (sections, chunks) = land::access_list_chunks(&entries);
                for (i, chunk) in chunks.iter().enumerate() {
                    let mut m = ParcelAccessListUpdate::default();
                    m.agent_data.agent_id = agent;
                    m.agent_data.session_id = session;
                    m.data.flags = flags;
                    m.data.local_id = local_id;
                    m.data.transaction_id = transaction;
                    m.data.sequence_id = i as i32 + 1;
                    m.data.sections = sections;
                    m.list = chunk
                        .iter()
                        .map(|e| parcel_access_list_update::List {
                            id: e.id,
                            time: e.time,
                            flags: e.flags,
                        })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            LandCommand::DwellRequest { handle, local_id } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelDwellRequest::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.local_id = local_id;
                // filled in on the simulator
                m.data.parcel_id = Uuid::nil();
                self.send(addr, &m, true);
            }
            LandCommand::ObjectOwnersRequest { handle, local_id } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelObjectOwnersRequest::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.parcel_data.local_id = local_id;
                self.send(addr, &m, true);
            }
            LandCommand::ReturnObjects {
                handle,
                local_id,
                return_type,
                owners,
            } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelReturnObjects::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.parcel_data.local_id = local_id;
                m.parcel_data.return_type = return_type;
                // dummy task id, not used
                m.task_i_ds = vec![parcel_return_objects::TaskIDs { task_id: Uuid::nil() }];
                let owners = if owners.is_empty() { vec![Uuid::nil()] } else { owners };
                m.owner_i_ds = owners
                    .into_iter()
                    .map(|owner_id| parcel_return_objects::OwnerIDs { owner_id })
                    .collect();
                self.send(addr, &m, true);
            }
            LandCommand::SetOtherCleanTime { handle, local_id, minutes } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelSetOtherCleanTime::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.parcel_data.local_id = local_id;
                m.parcel_data.other_clean_time = minutes;
                self.send(addr, &m, true);
            }
            LandCommand::Release { handle, local_id } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelRelease::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.local_id = local_id;
                self.send(addr, &m, true);
            }
            LandCommand::Reclaim { handle, local_id } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelReclaim::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.local_id = local_id;
                self.send(addr, &m, true);
            }
            LandCommand::DeedToGroup { handle, local_id, group } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelDeedToGroup::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.group_id = group;
                m.data.local_id = local_id;
                self.send(addr, &m, true);
            }
            LandCommand::BuyPass { handle, local_id } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelBuyPass::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.parcel_data.local_id = local_id;
                self.send(addr, &m, true);
            }
            LandCommand::CovenantRequest { handle } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = EstateCovenantRequest::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                self.send(addr, &m, true);
            }
            LandCommand::ParcelIdRequest {
                handle,
                local_id,
                region_id,
                position,
            } => self.request_parcel_id(handle, local_id, region_id, position),
            LandCommand::EnvironmentRequest { local_id } => self.parcel_environment(local_id, EnvVerb::Get),
            LandCommand::EnvironmentUpdate {
                local_id,
                day_length,
                day_offset,
            } => self.parcel_environment(local_id, EnvVerb::Put(land::environment_update_body(day_length, day_offset))),
            LandCommand::EnvironmentReset { local_id } => self.parcel_environment(local_id, EnvVerb::Delete),
            LandCommand::ExperienceInfo(ids) => self.request_experience_info(ids),
            LandCommand::AvatarSearch { query } => self.avatar_search(query),
            LandCommand::MediaType { url } => {
                let http = self.sh.http.clone();
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    // headers only, redirects followed; an error is "none/none"
                    let mime = match http
                        .head(&url)
                        .header("Accept", "*/*")
                        .timeout(std::time::Duration::from_secs(15))
                        .send()
                        .await
                    {
                        Ok(r) => r
                            .headers()
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .map(|t| t.split(';').next().unwrap_or("").trim().to_lowercase())
                            .filter(|t| !t.is_empty())
                            .unwrap_or_else(|| "none/none".into()),
                        Err(e) => {
                            log::info!("media type of {url}: {e}");
                            "none/none".into()
                        }
                    };
                    let _ = events.send(NetEvent::Land(LandEvent::MediaType { url, mime }));
                });
            }
            LandCommand::GroupNames(ids) => {
                // LLCacheName::Impl::sendRequest: one block per group
                for chunk in ids.chunks(MAX_NAME_BLOCKS) {
                    let m = UUIDGroupNameRequest {
                        uuid_name_block: chunk.iter().map(|&id| uuid_group_name_request::UUIDNameBlock { id }).collect(),
                    };
                    self.send_main(&m, true);
                }
            }
        }
    }

    /// LLViewerParcelMgr::sendParcelPropertiesUpdate: the capability when the
    /// region has it, else the UDP message (without the newer fields).
    fn send_parcel_update(&mut self, handle: RegionHandle, u: &land::ParcelUpdate) {
        if let Some(url) = self.land_cap(handle, "ParcelPropertiesUpdate") {
            let http = self.sh.caps_http.clone();
            let body = aurora_llsd::to_xml(&u.to_llsd());
            let local_id = u.local_id;
            tokio::spawn(async move {
                match http
                    .post(&url)
                    .header("Content-Type", "application/llsd+xml")
                    .header("Accept", "application/llsd+xml")
                    .body(body)
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => log::info!("parcel {local_id}: properties sent"),
                    Ok(r) => log::warn!("parcel {local_id}: properties update HTTP {}", r.status().as_u16()),
                    Err(e) => log::warn!("parcel {local_id}: properties update failed: {e}"),
                }
            });
            return;
        }
        let Some(addr) = self.land_sim(handle) else {
            return;
        };
        let mut m = ParcelPropertiesUpdate::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        let d = &mut m.parcel_data;
        d.local_id = u.local_id;
        d.flags = 1;
        d.parcel_flags = u.flags;
        d.sale_price = u.sale_price;
        d.name = str_field(&u.name);
        d.desc = str_field(&u.desc);
        d.music_url = str_field(&u.music_url);
        d.media_url = str_field(&u.media_url);
        d.media_id = u.media_id;
        d.media_auto_scale = u.media_auto_scale as u8;
        d.group_id = u.group_id;
        d.pass_price = u.pass_price;
        d.pass_hours = u.pass_hours;
        d.category = u.category;
        d.auth_buyer_id = u.auth_buyer;
        d.snapshot_id = u.snapshot_id;
        d.user_location = u.user_location;
        d.user_look_at = u.user_look_at;
        d.landing_type = u.landing_type;
        self.send(addr, &m, true);
    }

    /// LLRemoteParcelInfoProcessor::regionParcelInfoCoro: the parcel UUID
    /// of a point of the selection region.
    fn request_parcel_id(&mut self, handle: RegionHandle, local_id: i32, region_id: Uuid, position: glam::Vec3) {
        let events = self.sh.events.clone();
        let Some(url) = self.land_cap(handle, "RemoteParcelRequest") else {
            log::warn!("RemoteParcelRequest not available. Cannot request parcel ID");
            emit(self.sh, NetEvent::Land(LandEvent::ParcelId { local_id, id: None }));
            return;
        };
        let mut body = Llsd::new_map();
        body.insert(
            "location",
            Llsd::Array(vec![
                Llsd::Real(position.x as f64),
                Llsd::Real(position.y as f64),
                Llsd::Real(position.z as f64),
            ]),
        );
        if !region_id.is_nil() {
            body.insert("region_id", region_id);
        }
        body.insert("region_handle", Llsd::Binary(handle.to_be_bytes().to_vec()));
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let id = match http
                .post(&url)
                .header("Content-Type", "application/llsd+xml")
                .header("Accept", "application/llsd+xml")
                .body(aurora_llsd::to_xml(&body))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => r
                    .bytes()
                    .await
                    .ok()
                    .and_then(|b| aurora_llsd::from_xml(&b).ok())
                    .map(|v| v["parcel_id"].as_uuid()),
                Ok(r) => {
                    log::warn!("RemoteParcelRequest: HTTP {}", r.status().as_u16());
                    None
                }
                Err(e) => {
                    log::warn!("RemoteParcelRequest failed: {e}");
                    None
                }
            };
            let _ = events.send(NetEvent::Land(LandEvent::ParcelId { local_id, id }));
        });
    }

    /// LLEnvironment::requestParcel / coroUpdateEnvironment /
    /// coroResetEnvironment on `ExtEnvironment?parcelid=N` of the agent region.
    fn parcel_environment(&mut self, local_id: i32, verb: EnvVerb) {
        let Some(url) = self.main_cap("ExtEnvironment") else {
            let result = Err("la région n'a pas la capability ExtEnvironment".to_owned());
            emit(self.sh, NetEvent::Land(LandEvent::Environment { local_id, result }));
            return;
        };
        let url = format!("{url}?parcelid={local_id}");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let req = match verb {
                EnvVerb::Get => http.get(&url),
                EnvVerb::Put(body) => http
                    .put(&url)
                    .header("Content-Type", "application/llsd+xml")
                    .body(aurora_llsd::to_xml(&body)),
                EnvVerb::Delete => http.delete(&url),
            };
            let result = match req.header("Accept", "application/llsd+xml").send().await {
                Ok(r) if r.status().is_success() => match r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()) {
                    // a refused change comes back as success = false with a message
                    Some(v) if v.has("success") && !v["success"].as_bool() => Err(v["message"].to_string_value()),
                    Some(v) => Ok(v["environment"].clone()),
                    None => Err("réponse illisible".to_owned()),
                },
                Ok(r) => Err(format!("HTTP {}", r.status().as_u16())),
                Err(e) => Err(e.to_string()),
            };
            if let Err(e) = &result {
                log::warn!("parcel {local_id} environment: {e}");
            }
            let _ = events.send(NetEvent::Land(LandEvent::Environment { local_id, result }));
        });
    }

    /// LLExperienceCache::requestExperiences: GetExperienceInfo with
    /// public_id parameters.
    fn request_experience_info(&mut self, ids: Vec<Uuid>) {
        let Some(base) = self.main_cap("GetExperienceInfo") else {
            return;
        };
        let mut url = format!("{base}?page_size={}", ids.len().max(1));
        for id in &ids {
            url.push_str(&format!("&public_id={id}"));
        }
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(r) if r.status().is_success() => {
                    if let Some(v) = r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()) {
                        let list = v["experience_keys"]
                            .as_array()
                            .iter()
                            .map(|e| (e["public_id"].as_uuid(), e["name"].to_string_value()))
                            .filter(|(id, _)| !id.is_nil())
                            .collect();
                        let _ = events.send(NetEvent::Land(LandEvent::ExperienceInfo(list)));
                    }
                }
                Ok(r) => log::warn!("GetExperienceInfo: HTTP {}", r.status().as_u16()),
                Err(e) => log::warn!("GetExperienceInfo failed: {e}"),
            }
        });
    }

    /// LLFloaterAvatarPicker::find: the capability searches usernames and
    /// display names; the UDP request is the legacy fallback (FIRE-15194).
    fn avatar_search(&mut self, query: String) {
        let Some(mut url) = self.main_cap("AvatarPickerSearch") else {
            let query_id = Uuid::new_v4();
            let mut m = AvatarPickerRequest::default();
            m.agent_data.agent_id = self.agent_id();
            m.agent_data.session_id = self.session_id();
            m.agent_data.query_id = query_id;
            m.data.name = str_field(&query);
            self.send_main(&m, true);
            self.land.picker_queries.push((query_id, query));
            return;
        };
        if !url.ends_with('/') {
            url.push('/');
        }
        url.push_str(&format!("?page_size=100&names={}", land::avatar_search_query(&query)));
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let results = match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(r) if r.status().is_success() => r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()).map(|v| {
                    v["agents"]
                        .as_array()
                        .iter()
                        .map(|a| land::FoundAvatar {
                            id: a["id"].as_uuid(),
                            display_name: a["display_name"].to_string_value(),
                            username: a["username"].to_string_value(),
                        })
                        .filter(|a| !a.id.is_nil())
                        .collect()
                }),
                Ok(r) => {
                    log::warn!("AvatarPickerSearch: HTTP {}", r.status().as_u16());
                    None
                }
                Err(e) => {
                    log::warn!("AvatarPickerSearch failed: {e}");
                    None
                }
            };
            let _ = events.send(NetEvent::Land(LandEvent::AvatarSearch { query, results }));
        });
    }

    /// UDP messages of this module; Ok(false) when not one of them.
    pub(super) fn dispatch_land(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        let id = pkt.id;
        if id == ParcelAccessListReply::ID {
            let m: ParcelAccessListReply = pkt.decode()?;
            let entries = m
                .list
                .iter()
                .filter(|e| !e.id.is_nil())
                .map(|e| land::AccessEntry {
                    id: e.id,
                    time: e.time,
                    flags: e.flags,
                })
                .collect();
            emit(
                self.sh,
                NetEvent::Land(LandEvent::AccessList {
                    local_id: m.data.local_id,
                    flags: m.data.flags,
                    entries,
                }),
            );
        } else if id == AvatarPickerReply::ID {
            let m: AvatarPickerReply = pkt.decode()?;
            let Some(i) = self.land.picker_queries.iter().position(|(q, _)| *q == m.agent_data.query_id) else {
                return Ok(true);
            };
            let (_, query) = self.land.picker_queries.remove(i);
            let results = m
                .data
                .iter()
                .filter(|d| !d.avatar_id.is_nil())
                .map(|d| {
                    let (first, last) = (field_str(&d.first_name), field_str(&d.last_name));
                    land::FoundAvatar {
                        id: d.avatar_id,
                        display_name: if last.is_empty() || last.eq_ignore_ascii_case("resident") {
                            first.clone()
                        } else {
                            format!("{first} {last}")
                        },
                        username: if last.is_empty() || last.eq_ignore_ascii_case("resident") {
                            first.to_lowercase()
                        } else {
                            format!("{first}.{last}").to_lowercase()
                        },
                    }
                })
                .collect();
            emit(
                self.sh,
                NetEvent::Land(LandEvent::AvatarSearch {
                    query,
                    results: Some(results),
                }),
            );
        } else if id == UUIDGroupNameReply::ID {
            let m: UUIDGroupNameReply = pkt.decode()?;
            let names = m.uuid_name_block.iter().map(|b| (b.id, field_str(&b.group_name))).collect();
            emit(self.sh, NetEvent::Land(LandEvent::GroupNames(names)));
        } else if id == ParcelDwellReply::ID {
            let m: ParcelDwellReply = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::Land(LandEvent::Dwell {
                    local_id: m.data.local_id,
                    dwell: m.data.dwell,
                }),
            );
        } else if id == ParcelObjectOwnersReply::ID {
            // deprecated over UDP, still handled like the viewer does
            let m: ParcelObjectOwnersReply = pkt.decode()?;
            let owners = m
                .data
                .iter()
                .filter(|d| !d.owner_id.is_nil())
                .map(|d| land::ObjectOwner {
                    id: d.owner_id,
                    is_group: d.is_group_owned,
                    count: d.count,
                    online: d.online_status,
                    most_recent: 0,
                })
                .collect();
            emit(self.sh, NetEvent::Land(LandEvent::ObjectOwners(owners)));
        } else if id == EstateCovenantReply::ID {
            let m: EstateCovenantReply = pkt.decode()?;
            let d = &m.data;
            let handle = self.handle_of(from);
            emit(
                self.sh,
                NetEvent::Land(LandEvent::Covenant {
                    handle,
                    covenant_id: d.covenant_id,
                    timestamp: d.covenant_timestamp,
                    estate_name: field_str(&d.estate_name),
                    estate_owner: d.estate_owner_id,
                }),
            );
            if !d.covenant_id.is_nil() {
                self.request_covenant(from, d.covenant_id);
            }
        } else if id == TransferInfo::ID {
            let m: TransferInfo = pkt.decode()?;
            let t = &m.transfer_info;
            if let Some(c) = self.land.covenant.as_mut().filter(|c| c.transfer == t.transfer_id) {
                if t.status == LLTS_OK {
                    c.data.set_size(t.size);
                } else {
                    log::warn!("covenant transfer refused (status {})", t.status);
                    let covenant_id = c.covenant;
                    self.land.covenant = None;
                    emit(self.sh, NetEvent::Land(LandEvent::CovenantText { covenant_id, text: None }));
                }
            }
        } else if id == TransferPacket::ID {
            let m: TransferPacket = pkt.decode()?;
            let t = m.transfer_data;
            let Some(c) = self.land.covenant.as_mut().filter(|c| c.transfer == t.transfer_id) else {
                return Ok(true);
            };
            if t.status != LLTS_OK && t.status != LLTS_DONE {
                log::warn!("covenant transfer failed (status {})", t.status);
                let covenant_id = c.covenant;
                self.land.covenant = None;
                emit(self.sh, NetEvent::Land(LandEvent::CovenantText { covenant_id, text: None }));
                return Ok(true);
            }
            c.data.add(t.packet, t.data, t.status == LLTS_DONE);
            if let Some(bytes) = c.data.complete() {
                let covenant_id = c.covenant;
                self.land.covenant = None;
                let text = land::notecard_text(&bytes);
                if text.is_none() {
                    log::warn!("covenant {covenant_id}: not a notecard");
                }
                emit(self.sh, NetEvent::Land(LandEvent::CovenantText { covenant_id, text }));
            }
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// gAssetStorage->getEstateAsset(ET_Covenant): TransferRequest from the
    /// simulator that answered the covenant request.
    fn request_covenant(&mut self, sim: SocketAddr, covenant: Uuid) {
        let transfer = Uuid::new_v4();
        let mut m = TransferRequest::default();
        let t = &mut m.transfer_info;
        t.transfer_id = transfer;
        t.channel_type = LLTCT_ASSET;
        t.source_type = LLTST_SIM_ESTATE;
        t.priority = 100.0;
        t.params = land::covenant_transfer_params(self.agent_id(), self.session_id());
        self.send(sim, &m, true);
        self.land.covenant = Some(CovenantTransfer {
            transfer,
            covenant,
            data: land::TransferAssembly::default(),
        });
    }

    /// Event-queue messages of this module; false when not one of them.
    pub(super) fn on_land_eq(&mut self, message: &str, b: &Llsd) -> bool {
        match message {
            // ParcelObjectOwnersReply is UDPDeprecated: it comes this way
            "ParcelObjectOwnersReply" => {
                emit(self.sh, NetEvent::Land(LandEvent::ObjectOwners(land::parse_object_owners(b))));
                true
            }
            _ => false,
        }
    }
}

enum EnvVerb {
    Get,
    Put(Llsd),
    Delete,
}
