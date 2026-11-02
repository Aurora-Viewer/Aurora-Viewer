//! Session side of the About Land floater (land.rs): parcel selection and
//! properties update, access lists, dwell, object owners and returns,
//! release / deed / pass, purchase (ParcelBuy / ParcelClaim), the estate covenant (EstateCovenantRequest, then
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
                log::debug!(
                    "ParcelAccessListUpdate: parcel {local_id}, kind {flags:#x}, {} entries in {} message(s), to {addr}",
                    entries.len(),
                    chunks.len()
                );
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
            LandCommand::SelectObjects {
                handle,
                local_id,
                return_type,
                owners,
            } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = ParcelSelectObjects::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.parcel_data.local_id = local_id;
                m.parcel_data.return_type = return_type;
                // a null key when there is no list, so that the message is complete
                let owners = if owners.is_empty() { vec![Uuid::nil()] } else { owners };
                m.return_i_ds = owners
                    .into_iter()
                    .map(|return_id| parcel_select_objects::ReturnIDs { return_id })
                    .collect();
                self.send(addr, &m, true);
            }
            LandCommand::ReturnTasks { handle, local_id, tasks } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                // one message per batch of TaskIDs, as isSendFullFast cuts them
                for chunk in tasks.chunks(200) {
                    let mut m = ParcelReturnObjects::default();
                    m.agent_data.agent_id = agent;
                    m.agent_data.session_id = session;
                    m.parcel_data.local_id = local_id;
                    m.parcel_data.return_type = land::RT_LIST;
                    m.task_i_ds = chunk.iter().map(|&task_id| parcel_return_objects::TaskIDs { task_id }).collect();
                    self.send(addr, &m, true);
                }
            }
            LandCommand::ScriptResources { parcel_id } => self.script_resources(parcel_id),
            LandCommand::AttachmentScripts => self.attachment_scripts(),
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
            LandCommand::FreezeUser { handle, target, unfreeze } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = FreezeUser::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.target_id = target;
                m.data.flags = u32::from(unfreeze);
                self.send(addr, &m, true);
            }
            LandCommand::EjectUser { handle, target, ban } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                let mut m = EjectUser::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.target_id = target;
                m.data.flags = u32::from(ban);
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
            LandCommand::Buy {
                handle,
                local_id,
                group,
                for_group,
                remove_contribution,
                price,
                area,
            } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                log::info!("ParcelBuy: parcel {local_id}, L$ {price}, {area} m²");
                let mut m = ParcelBuy::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.group_id = group;
                m.data.is_group_owned = for_group;
                m.data.remove_contribution = remove_contribution;
                m.data.local_id = local_id;
                // don't allow escrow buys
                m.data.final_ = true;
                m.parcel_data.price = price;
                m.parcel_data.area = area;
                self.send(addr, &m, true);
            }
            LandCommand::Claim {
                handle,
                group,
                for_group,
                west,
                south,
                east,
                north,
            } => {
                let Some(addr) = self.land_sim(handle) else {
                    return;
                };
                log::info!("ParcelClaim: {west},{south} to {east},{north}");
                let mut m = ParcelClaim::default();
                m.agent_data.agent_id = agent;
                m.agent_data.session_id = session;
                m.data.group_id = group;
                m.data.is_group_owned = for_group;
                m.data.final_ = true;
                m.parcel_data = vec![parcel_claim::ParcelData { west, south, east, north }];
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
            LandCommand::EnvironmentRequest { local_id } => self.parcel_environment(local_id, None, EnvVerb::Get),
            LandCommand::EnvironmentUpdate {
                local_id,
                day_length,
                day_offset,
            } => self.parcel_environment(local_id, None, EnvVerb::Put(land::environment_update_body(day_length, day_offset))),
            LandCommand::EnvironmentReset { local_id } => self.parcel_environment(local_id, None, EnvVerb::Delete),
            LandCommand::EnvironmentAltitudes {
                local_id,
                day_length,
                day_offset,
                altitudes,
            } => {
                let body = land::environment_altitudes_body(day_length, day_offset, altitudes);
                self.parcel_environment(local_id, None, EnvVerb::Put(body));
            }
            LandCommand::EnvironmentApply {
                local_id,
                track,
                asset,
                name,
                flags,
                day_length,
                day_offset,
            } => {
                let body = land::environment_apply_body(track.is_none(), asset, &name, flags, day_length, day_offset);
                self.parcel_environment(local_id, track, EnvVerb::Put(body));
            }
            LandCommand::EnvironmentApplyDay {
                local_id,
                track,
                day,
                flags,
                day_length,
                day_offset,
            } => {
                let body = land::environment_day_body(track.is_none(), &day, flags, day_length, day_offset);
                self.parcel_environment(local_id, track, EnvVerb::Put(body));
            }
            LandCommand::ExperienceSearch { query, page } => self.experience_search(query, page),
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
        let body = land::remote_parcel_body(position, region_id, Some(handle));
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let id = match post_remote_parcel(&http, &url, &body).await {
                Ok(id) => Some(id),
                Err(land::RemoteParcelError::NoParcel) => Some(Uuid::nil()),
                Err(_) => None,
            };
            let _ = events.send(NetEvent::Land(LandEvent::ParcelId { local_id, id }));
        });
    }

    /// LLPanelPlaceInfo::displayParcelInfo for a place of any region: the
    /// capability of the agent's region (gAgent.getRegion()), the point in
    /// its region's coordinates and the handle of its 256 m slot, no region
    /// id for a place link (LLURLDispatcherImpl::regionHandleCallback), the
    /// region id of a landmark (LLPanelPlaces::onLandmarkLoaded).
    pub(super) fn remote_parcel_request(&mut self, handle: RegionHandle, position: glam::Vec3, region_id: Uuid) {
        let Some(url) = self.main_cap("RemoteParcelRequest") else {
            log::warn!("RemoteParcelRequest not available. Cannot request parcel ID");
            let result = Err(land::RemoteParcelError::NoCapability);
            emit(self.sh, NetEvent::RemoteParcel { handle, position, result });
            return;
        };
        let body = land::remote_parcel_body(position, region_id, Some(handle));
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = post_remote_parcel(&http, &url, &body).await;
            if let Err(e) = result {
                // the slot asked, to tell a wrong handle from a missing parcel
                log::warn!(
                    "RemoteParcelRequest for region {}, {} at {position}: {e:?}",
                    (handle >> 32) / 256,
                    (handle & 0xffff_ffff) / 256
                );
            }
            let _ = events.send(NetEvent::RemoteParcel { handle, position, result });
        });
    }

    /// LLEnvironment::requestParcel / coroUpdateEnvironment /
    /// coroResetEnvironment on `ExtEnvironment?parcelid=N` of the agent
    /// region, or without a parcel for the whole region (REGION_PARCEL_ID).
    fn parcel_environment(&mut self, local_id: i32, track: Option<i32>, verb: EnvVerb) {
        let Some(url) = self.main_cap("ExtEnvironment") else {
            let result = Err(aurora_i18n::tr!("session-land-the-region-does-not-provide-the-extenvironment-capability").to_owned());
            emit(self.sh, NetEvent::Land(LandEvent::Environment { local_id, result }));
            return;
        };
        let url = format!("{url}{}", land::environment_query(local_id, track));
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
                    None => Err(aurora_i18n::tr!("session-land-unreadable-response").to_owned()),
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

    /// LLExperienceCache::requestExperiences: GetExperienceInfo's `id/`
    /// resource with public_id parameters (crate::experience::info_urls).
    fn request_experience_info(&mut self, ids: Vec<Uuid>) {
        let Some(base) = self.main_cap("GetExperienceInfo") else {
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            for (url, _) in crate::experience::info_urls(&base, &ids) {
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
            }
        });
    }

    /// LLFloaterAvatarPicker::find: the capability searches usernames and
    /// display names; the UDP request is the legacy fallback (FIRE-15194).
    /// getLandScriptResourcesCoro, then the summary and details coroutines.
    fn script_resources(&mut self, parcel_id: Uuid) {
        let events = self.sh.events.clone();
        let Some(url) = self.main_cap("LandResources") else {
            let e = aurora_i18n::tr!("session-land-no-land-resources").to_owned();
            emit(self.sh, NetEvent::Land(LandEvent::ScriptSummary(Err(e))));
            return;
        };
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let mut body = Llsd::new_map();
            body.insert("parcel_id", parcel_id);
            let first = http
                .post(&url)
                .header("Content-Type", "application/llsd+xml")
                .header("Accept", "application/llsd+xml")
                .body(aurora_llsd::to_xml(&body))
                .send()
                .await;
            let answer = match first {
                Ok(r) if r.status().is_success() => r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()),
                Ok(r) => {
                    log::warn!("LandResources: HTTP {}", r.status().as_u16());
                    None
                }
                Err(e) => {
                    log::warn!("LandResources failed: {e}");
                    None
                }
            };
            let Some(answer) = answer else {
                let e = aurora_i18n::tr!("session-land-script-info-failed").to_owned();
                let _ = events.send(NetEvent::Land(LandEvent::ScriptSummary(Err(e))));
                return;
            };
            let get = |url: String| {
                let http = http.clone();
                async move {
                    match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                        Ok(r) if r.status().is_success() => r
                            .bytes()
                            .await
                            .ok()
                            .and_then(|b| aurora_llsd::from_xml(&b).ok())
                            .ok_or_else(|| aurora_i18n::tr!("session-land-unreadable-response").to_owned()),
                        Ok(r) => Err(format!("HTTP {}", r.status().as_u16())),
                        Err(e) => Err(e.to_string()),
                    }
                }
            };
            let summary_url = answer["ScriptResourceSummary"].to_string_value();
            let details_url = answer["ScriptResourceDetails"].to_string_value();
            let (summary, details) = tokio::join!(
                async {
                    if summary_url.is_empty() {
                        None
                    } else {
                        Some(get(summary_url).await)
                    }
                },
                async {
                    if details_url.is_empty() {
                        None
                    } else {
                        Some(get(details_url).await)
                    }
                }
            );
            if let Some(s) = summary {
                let _ = events.send(NetEvent::Land(LandEvent::ScriptSummary(s.map(|v| land::parse_script_summary(&v)))));
            }
            if let Some(d) = details {
                let _ = events.send(NetEvent::Land(LandEvent::ScriptDetails(d.map(|v| land::parse_script_details(&v)))));
            }
        });
    }

    /// LLFloaterMyScripts::getAttachmentLimitsCoro.
    fn attachment_scripts(&mut self) {
        let events = self.sh.events.clone();
        let Some(url) = self.main_cap("AttachmentResources") else {
            let e = aurora_i18n::tr!("session-land-no-attachment-resources").to_owned();
            emit(self.sh, NetEvent::Land(LandEvent::AttachmentScripts(Err(e))));
            return;
        };
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let result = match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(r) if r.status().is_success() => r
                    .bytes()
                    .await
                    .ok()
                    .and_then(|b| aurora_llsd::from_xml(&b).ok())
                    .map(|v| (land::parse_script_summary(&v), land::parse_attachment_scripts(&v)))
                    .ok_or_else(|| aurora_i18n::tr!("session-land-unreadable-response").to_owned()),
                Ok(r) => Err(format!("HTTP {}", r.status().as_u16())),
                Err(e) => Err(e.to_string()),
            };
            let _ = events.send(NetEvent::Land(LandEvent::AttachmentScripts(result)));
        });
    }

    /// LLExperienceCache::findExperienceByNameCoro.
    fn experience_search(&mut self, query: String, page: i32) {
        let events = self.sh.events.clone();
        let Some(base) = self.main_cap("FindExperienceByName") else {
            log::warn!("FindExperienceByName not available");
            emit(self.sh, NetEvent::Land(LandEvent::ExperienceSearch { query, results: None }));
            return;
        };
        let url = land::experience_search_url(&base, &query, page);
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let results = match http.get(&url).header("Accept", "application/llsd+xml").send().await {
                Ok(r) if r.status().is_success() => r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()).map(|v| {
                    v["experience_keys"]
                        .as_array()
                        .iter()
                        .map(land::FoundExperience::from_llsd)
                        .filter(|e| !e.id.is_nil())
                        .collect()
                }),
                Ok(r) => {
                    log::warn!("FindExperienceByName: HTTP {}", r.status().as_u16());
                    None
                }
                Err(e) => {
                    log::warn!("FindExperienceByName failed: {e}");
                    None
                }
            };
            let _ = events.send(NetEvent::Land(LandEvent::ExperienceSearch { query, results }));
        });
    }

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
            log::debug!(
                "ParcelAccessListReply: parcel {}, kind {:#x}, {} entries, from {from}",
                m.data.local_id,
                m.data.flags,
                m.list.iter().filter(|e| !e.id.is_nil()).count()
            );
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
        } else if id == ForceObjectSelect::ID {
            // LLSelectMgr::processForceObjectSelect: highlight, not select
            let m: ForceObjectSelect = pkt.decode()?;
            let handle = self.handle_of(from);
            emit(
                self.sh,
                NetEvent::Land(LandEvent::Highlight {
                    handle,
                    reset: m.header.reset_list,
                    local_ids: m.data.iter().map(|d| d.local_id).collect(),
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

/// POST of a RemoteParcelRequest body: the parcel, or why there is none
/// (the HTTP status is kept for LLPanelPlaceInfo::setErrorStatus).
async fn post_remote_parcel(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Uuid, land::RemoteParcelError> {
    match crate::caps::post_llsd(http, url, body).await {
        Ok(v) => land::parse_remote_parcel_reply(&v),
        Err(crate::caps::CapsError::Status(s)) => {
            log::warn!("RemoteParcelRequest: HTTP {s}");
            Err(land::RemoteParcelError::Status(s))
        }
        Err(e) => {
            log::warn!("RemoteParcelRequest failed: {e}");
            Err(land::RemoteParcelError::Failed)
        }
    }
}
