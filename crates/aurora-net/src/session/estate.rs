//! Session side of the Region / Estate window (estate.rs): what
//! LLFloaterRegionInfo::refreshFromRegion has each panel ask (the
//! EstateOwnerMessage `getinfo` with a new invoice, the EstateAccess lists,
//! RegionExperiences, the region environment, the restart schedule),
//! RequestRegionInfo, and the `estateupdateinfo` answer of the last invoice
//! (processEstateOwnerRequest drops the others). Port of
//! newview/llfloaterregioninfo.cpp (originally LGPL 2.1).

use super::{Session, emit};
use crate::estate::{self, EstateCommand, EstateEvent};
use crate::types::{NetEvent, RegionHandle};
use aurora_llsd::Llsd;
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg, str_field};
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

impl Session<'_> {
    /// LLPBRTerrainFeatures::queueQuery on arrival, also for ordinary residents.
    pub(super) fn request_terrain_transforms(&self, handle: RegionHandle) {
        if let Some(url) = self.sim_for_handle(handle).and_then(|addr| self.sim_cap(addr, "ModifyRegion")) {
            self.estate_get(handle, Some(url), Get::Transforms);
        }
    }

    pub(super) fn on_estate_command(&mut self, c: EstateCommand) {
        match c {
            EstateCommand::Refresh { handle } => self.refresh_estate(handle),
            EstateCommand::RequestRegionInfo { handle } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let mut m = RequestRegionInfo::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                self.send(addr, &m, true);
            }
            EstateCommand::SetRegionInfo { handle, rules } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                // the capability first, else the old message
                match self.sim_cap(addr, "DispatchRegionInfo") {
                    Some(url) => self.estate_post(url, rules.body(), "region info update"),
                    None => {
                        let invoice = self.estate_invoice;
                        self.send_estate_owner_message(addr, "setregioninfo", invoice, &rules.strings(), true);
                    }
                }
            }
            EstateCommand::SetEstateInfo { handle, info } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                match self.sim_cap(addr, "EstateChangeInfo") {
                    Some(url) => {
                        let body = estate::estate_change_body(&info, self.estate_invoice);
                        self.estate_post(url, body, "estate info");
                    }
                    None => {
                        // nextInvoice, then commitEstateInfoDataserver (sent
                        // with gAgent.sendMessage: not reliable); the answer
                        // comes on that new invoice
                        let invoice = Uuid::new_v4();
                        self.estate_invoice = invoice;
                        let strings = estate::estate_change_strings(&info);
                        self.send_estate_owner_message(addr, "estatechangeinfo", invoice, &strings, false);
                    }
                }
            }
            EstateCommand::SetTerrain { handle, terrain, sun } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                for (method, strings) in estate::terrain_messages(&terrain, sun) {
                    self.send_estate_owner_message(addr, method, invoice, &strings, true);
                }
            }
            EstateCommand::BakeTerrain { handle } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                self.send_estate_owner_message(addr, "terrain", invoice, &["bake".to_owned()], true);
            }
            EstateCommand::AccessDelta { handle, flags, ids, batch } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                // sendEstateAccessDelta, reliable, with the last invoice
                let invoice = self.estate_invoice;
                let agent = self.agent_id();
                for (id, f) in ids.iter().zip(estate::access_delta_flags(flags, ids.len(), batch)) {
                    let strings = estate::access_delta_strings(agent, f, *id);
                    self.send_estate_owner_message(addr, "estateaccessdelta", invoice, &strings, true);
                }
            }
            EstateCommand::RefreshAccess { handle } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let url = self.sim_cap(addr, "EstateAccess");
                self.estate_get(handle, url, Get::Access);
            }
            EstateCommand::Message {
                handle,
                estate,
                name,
                text,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                if estate {
                    let strings = estate::estate_message_strings(&name, &text);
                    self.send_estate_owner_message(addr, "instantmessage", invoice, &strings, true);
                } else {
                    let strings = estate::region_message_strings(self.agent_id(), &name, &text);
                    self.send_estate_owner_message(addr, "simulatormessage", invoice, &strings, true);
                }
            }
            EstateCommand::TeleportHome { handle, target } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                let me = self.agent_id().hyphenated().to_string();
                match target {
                    Some(id) => {
                        let strings = [me, id.hyphenated().to_string()];
                        self.send_estate_owner_message(addr, "teleporthomeuser", invoice, &strings, true);
                    }
                    // "historical message name" (onKickAllCommit)
                    None => self.send_estate_owner_message(addr, "teleporthomeallusers", invoice, &[me], true),
                }
            }
            EstateCommand::KickFromEstate { handle, target } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                self.send_estate_owner_message(addr, "kickestate", invoice, &[target.hyphenated().to_string()], true);
            }
            EstateCommand::ReturnObjects {
                handle,
                target,
                flags,
                estate_wide,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                if estate_wide {
                    // routed by the space server to every region of the estate
                    let invoice = self.estate_invoice;
                    let strings = [flags.to_string(), target.hyphenated().to_string()];
                    self.send_estate_owner_message(addr, "estateobjectreturn", invoice, &strings, true);
                } else {
                    // send_sim_wide_deletes (llfloatergodtools.cpp): this region only
                    log::info!("sending SimWideDeletes (flags {flags})");
                    let mut m = SimWideDeletes::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.data_block.target_id = target;
                    m.data_block.flags = flags;
                    self.send(addr, &m, true);
                }
            }
            EstateCommand::Restart { handle, seconds } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                self.send_estate_owner_message(addr, "restart", invoice, &[seconds.to_string()], true);
            }
            EstateCommand::Telehub { handle, verb, objects } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                // sendGodlikeRequest: no invoice
                self.send_estate_owner_message(addr, "telehub", Uuid::nil(), &verb.strings(&objects), true);
            }
            EstateCommand::TerrainRaw { handle, transfer } => self.terrain_raw(handle, transfer),
            EstateCommand::TerrainTransforms { handle, transforms } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let Some(url) = self.sim_cap(addr, "ModifyRegion") else {
                    return;
                };
                // modifyRegionCoro, then the simulator's RegionHandshake has
                // queryRegionCoro ask again
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                let body = estate::terrain_overrides_body(&transforms);
                tokio::spawn(async move {
                    let posted = crate::caps::post_llsd(&http, &url, &body).await;
                    let failed = match &posted {
                        Ok(v) if v["success"].as_bool() => None,
                        Ok(v) => Some(v["message"].to_string_value()),
                        Err(e) => Some(e.to_string()),
                    };
                    let transforms = match failed {
                        Some(why) => {
                            log::warn!("terrain transforms not modified: {why}");
                            Err(aurora_i18n::tr!("net-estate-transforms-refused", reason = why).to_owned())
                        }
                        None => {
                            log::info!("terrain transforms modified");
                            fetch_llsd(&http, &url, Get::Transforms)
                                .await
                                .as_ref()
                                .and_then(estate::parse_terrain_overrides)
                                .ok_or_else(|| aurora_i18n::tr!("net-estate-transforms-unavailable").to_owned())
                        }
                    };
                    let _ = events.send(NetEvent::Estate(EstateEvent::TerrainTransforms { handle, transforms }));
                });
            }
            EstateCommand::ExperienceDelta { handle, flags, ids } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                // sendEstateExperienceDelta: who asks, "%u", the experience
                let invoice = self.estate_invoice;
                let agent = self.agent_id();
                for (id, f) in ids.iter().zip(estate::access_delta_flags(flags, ids.len(), true)) {
                    let strings = estate::access_delta_strings(agent, f, *id);
                    self.send_estate_owner_message(addr, "estateexperiencedelta", invoice, &strings, true);
                }
            }
            EstateCommand::RefreshExperiences { handle } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let url = self.sim_cap(addr, "RegionExperiences");
                self.estate_get(handle, url, Get::Experiences);
            }
            EstateCommand::ChangeCovenant { handle, asset } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                self.send_estate_owner_message(addr, "estatechangecovenantid", invoice, &[asset.hyphenated().to_string()], true);
            }
            EstateCommand::SetRegionDebug { handle, debug } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                self.send_estate_owner_message(addr, "setregiondebug", invoice, &debug.strings(), true);
            }
            EstateCommand::TopObjects { handle, report } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let invoice = self.estate_invoice;
                let (method, count) = estate::top_request(report);
                self.send_estate_owner_message(addr, method, invoice, &[count.to_owned()], true);
            }
            EstateCommand::LandStat {
                handle,
                report,
                flags,
                filter,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let mut m = LandStatRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.request_data.report_type = report;
                m.request_data.request_flags = flags;
                m.request_data.filter = str_field(&filter);
                // the whole region
                m.request_data.parcel_local_id = 0;
                self.send(addr, &m, true);
            }
            EstateCommand::ReturnTopObjects { handle, tasks } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                // returnObjects: the whole region (-1), RT_NONE, the tasks
                // cut as a full message would be (isSendFullFast)
                for chunk in tasks.chunks(50) {
                    let mut m = ParcelReturnObjects::default();
                    m.agent_data.agent_id = self.agent_id();
                    m.agent_data.session_id = self.session_id();
                    m.parcel_data.local_id = -1;
                    m.parcel_data.return_type = 1;
                    m.task_i_ds = chunk.iter().map(|&task_id| parcel_return_objects::TaskIDs { task_id }).collect();
                    self.send(addr, &m, true);
                }
            }
            EstateCommand::SetRestartSchedule { handle, days, seconds } => {
                let Some(addr) = self.sim_for_handle(handle) else {
                    return;
                };
                let Some(url) = self.sim_cap(addr, "RegionSchedule") else {
                    log::warn!("restart schedule: no RegionSchedule capability");
                    return;
                };
                // setRegionShcheduleCoro, then the schedule asked again for
                // the window (the Legacy Viewer closes its floater)
                let body = estate::restart_schedule_body(days, seconds);
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    match crate::caps::post_llsd(&http, &url, &body).await {
                        Ok(_) => log::info!("restart schedule posted"),
                        Err(e) => log::warn!("restart schedule not posted: {e}"),
                    }
                    let body = fetch_llsd(&http, &url, Get::Schedule).await;
                    let _ = events.send(NetEvent::Estate(Get::Schedule.event(handle, body)));
                });
            }
        }
    }

    fn sim_cap(&self, addr: SocketAddr, name: &str) -> Option<String> {
        self.sims.get(&addr).and_then(|s| s.caps.get(name).cloned())
    }

    /// LLPanelRegionInfo::sendEstateOwnerMessage: one parameter per string,
    /// a single empty one when there is none.
    pub(super) fn send_estate_owner_message(&mut self, addr: SocketAddr, method: &str, invoice: Uuid, strings: &[String], reliable: bool) {
        log::info!("sending estate request '{method}'");
        let mut m = EstateOwnerMessage::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        m.method_data.method = str_field(method);
        m.method_data.invoice = invoice;
        m.param_list = if strings.is_empty() {
            vec![estate_owner_message::ParamList { parameter: Vec::new() }]
        } else {
            strings
                .iter()
                .map(|s| estate_owner_message::ParamList { parameter: str_field(s) })
                .collect()
        };
        self.send(addr, &m, reliable);
    }

    /// POST of an estate setting (messageHttpPost, postAndSuspend): only
    /// logged, the simulator's next RegionInfo tells the result.
    fn estate_post(&self, url: String, body: Llsd, what: &'static str) {
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            match crate::caps::post_llsd(&http, &url, &body).await {
                Ok(_) => log::info!("{what} posted"),
                Err(e) => log::warn!("{what} not posted: {e}"),
            }
        });
    }

    /// The panels' refreshFromRegion, in their order: Estate (getinfo),
    /// Access (EstateAccess), Experiences, Environment; the schedule of the
    /// General panel's restart button last. The covenant is asked by the
    /// window (LandCommand::CovenantRequest), as the Covenant panel does.
    fn refresh_estate(&mut self, handle: RegionHandle) {
        let Some(addr) = self.sim_for_handle(handle) else {
            return;
        };
        // sendEstateOwnerMessage("getinfo") with no strings: a single
        // empty parameter
        let invoice = Uuid::new_v4();
        self.estate_invoice = invoice;
        self.send_estate_owner_message(addr, "getinfo", invoice, &[], true);

        let cap = |name: &str| self.sim_cap(addr, name);
        let gets = [
            (cap("EstateAccess"), Get::Access),
            (cap("RegionExperiences"), Get::Experiences),
            (cap("ExtEnvironment"), Get::Environment),
            (cap("RegionSchedule"), Get::Schedule),
            // the terrain's material overrides (queueQuery, which the
            // Legacy Viewer sends at each RegionHandshake)
            (cap("ModifyRegion"), Get::Transforms),
        ];
        for (url, what) in gets {
            self.estate_get(handle, url, what);
        }
    }

    /// GET a capability of the window and turn its answer into an event;
    /// a missing capability or a failure is an empty answer.
    fn estate_get(&self, handle: RegionHandle, url: Option<String>, what: Get) {
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let body = match &url {
                Some(url) => fetch_llsd(&http, url, what).await,
                None => None,
            };
            let _ = events.send(NetEvent::Estate(what.event(handle, body)));
        });
    }

    /// processEstateOwnerRequest: `estateupdateinfo` and `setaccess` of the
    /// last invoice.
    pub(super) fn dispatch_estate(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        if pkt.id == TelehubInfo::ID {
            // LLFloaterTelehub::unpackTelehubInfo
            let m: TelehubInfo = pkt.decode()?;
            let b = &m.telehub_block;
            let info = estate::TelehubInfo {
                object: b.object_id,
                name: estate::parameter_string(&b.object_name),
                position: b.telehub_pos.to_array(),
                rotation: b.telehub_rot.to_array(),
                spawns: m
                    .spawn_point_block
                    .iter()
                    .take(estate::MAX_SPAWNPOINTS_PER_TELEHUB)
                    .map(|s| s.spawn_point_pos.to_array())
                    .collect(),
            };
            let handle = self.handle_of(from);
            emit(
                self.sh,
                NetEvent::Estate(EstateEvent::Telehub {
                    handle,
                    info: Arc::new(info),
                }),
            );
            return Ok(true);
        }
        if pkt.id == LandStatReply::ID {
            // an older simulator's UDP answer (UDPDeprecated): the same
            // report as the event queue's, without DataExtended
            let m: LandStatReply = pkt.decode()?;
            let report = estate::TopObjects {
                report: m.request_data.report_type,
                flags: m.request_data.request_flags,
                total: m.request_data.total_object_count,
                objects: m
                    .report_data
                    .iter()
                    .map(|r| estate::TopObject {
                        local_id: r.task_local_id,
                        task_id: r.task_id,
                        position: [r.location_x, r.location_y, r.location_z],
                        score: r.score,
                        name: estate::parameter_string(&r.task_name),
                        owner: estate::username_of(&estate::parameter_string(&r.owner_name)),
                        parcel: "unknown".to_owned(),
                        ..Default::default()
                    })
                    .collect(),
            };
            let handle = self.handle_of(from);
            emit(
                self.sh,
                NetEvent::Estate(EstateEvent::TopObjects {
                    handle,
                    report: Arc::new(report),
                }),
            );
            return Ok(true);
        }
        if pkt.id != EstateOwnerMessage::ID {
            return Ok(false);
        }
        let m: EstateOwnerMessage = pkt.decode()?;
        let method = estate::parameter_string(&m.method_data.method);
        if m.method_data.invoice != self.estate_invoice {
            log::debug!("estate: mismatched {method} answer");
            return Ok(true);
        }
        if method == "estateupdateinfo" {
            let strings: Vec<String> = m.param_list.iter().map(|p| estate::parameter_string(&p.parameter)).collect();
            match estate::parse_estate_update_info(&strings) {
                Some(info) => {
                    let handle = self.handle_of(from);
                    emit(
                        self.sh,
                        NetEvent::Estate(EstateEvent::EstateInfo {
                            handle,
                            info: Arc::new(info),
                        }),
                    );
                }
                None => log::warn!("estate: estateupdateinfo with {} strings", strings.len()),
            }
        } else if method == "setaccess" {
            // its lists are not read: the window asks EstateAccess again
            let handle = self.handle_of(from);
            emit(self.sh, NetEvent::Estate(EstateEvent::AccessChanged { handle }));
        } else if method == "setexperience" {
            // the same: the window asks RegionExperiences again
            let handle = self.handle_of(from);
            emit(self.sh, NetEvent::Estate(EstateEvent::ExperiencesChanged { handle }));
        }
        Ok(true)
    }
}

impl Session<'_> {
    /// LandStatReply of the event queue (handle_land_reply).
    pub(super) fn on_land_stat_reply(&mut self, from: SocketAddr, body: &Llsd) {
        let report = estate::parse_land_stat(body);
        log::info!(
            "land stat: report {} with {} of {} objects",
            report.report,
            report.objects.len(),
            report.total
        );
        let handle = self.handle_of(from);
        emit(
            self.sh,
            NetEvent::Estate(EstateEvent::TopObjects {
                handle,
                report: Arc::new(report),
            }),
        );
    }
}

/// GET of a capability of the window: its LLSD, None on a failure.
async fn fetch_llsd(http: &reqwest::Client, url: &str, what: Get) -> Option<Llsd> {
    match http.get(url).header("Accept", "application/llsd+xml").send().await {
        Ok(r) if r.status().is_success() => r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()),
        Ok(r) => {
            log::info!("region / estate {what:?}: HTTP {}", r.status().as_u16());
            None
        }
        Err(e) => {
            log::warn!("region / estate {what:?}: {e}");
            None
        }
    }
}

/// The capabilities `refresh_estate` reads.
#[derive(Debug, Clone, Copy)]
enum Get {
    Access,
    Experiences,
    Environment,
    Schedule,
    Transforms,
}

impl Get {
    fn event(self, handle: RegionHandle, body: Option<Llsd>) -> EstateEvent {
        match self {
            Get::Access => EstateEvent::Access {
                handle,
                access: body.map(|v| Arc::new(estate::parse_estate_access(&v))),
            },
            Get::Experiences => EstateEvent::Experiences {
                handle,
                experiences: body.map(|v| estate::parse_region_experiences(&v)),
            },
            Get::Environment => EstateEvent::Environment {
                handle,
                environment: body.filter(|v| v.has("environment")).map(|v| v["environment"].clone()),
            },
            Get::Schedule => EstateEvent::Schedule {
                handle,
                schedule: body.map(|v| estate::parse_restart_schedule(&v)).ok_or(()),
            },
            Get::Transforms => EstateEvent::TerrainTransforms {
                handle,
                transforms: body
                    .as_ref()
                    .and_then(estate::parse_terrain_overrides)
                    .ok_or_else(|| aurora_i18n::tr!("net-estate-transforms-unavailable").to_owned()),
            },
        }
    }
}
