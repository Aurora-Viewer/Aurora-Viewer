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
        }
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
        let mut m = EstateOwnerMessage::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        m.method_data.method = str_field("getinfo");
        m.method_data.invoice = invoice;
        m.param_list = vec![estate_owner_message::ParamList { parameter: Vec::new() }];
        self.send(addr, &m, true);

        let cap = |name: &str| self.sims.get(&addr).and_then(|s| s.caps.get(name).cloned());
        let gets = [
            (cap("EstateAccess"), Get::Access),
            (cap("RegionExperiences"), Get::Experiences),
            (cap("ExtEnvironment"), Get::Environment),
            (cap("RegionSchedule"), Get::Schedule),
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
                Some(url) => match http.get(url).header("Accept", "application/llsd+xml").send().await {
                    Ok(r) if r.status().is_success() => r.bytes().await.ok().and_then(|b| aurora_llsd::from_xml(&b).ok()),
                    Ok(r) => {
                        log::info!("region / estate {what:?}: HTTP {}", r.status().as_u16());
                        None
                    }
                    Err(e) => {
                        log::warn!("region / estate {what:?}: {e}");
                        None
                    }
                },
                None => None,
            };
            let _ = events.send(NetEvent::Estate(what.event(handle, body)));
        });
    }

    /// processEstateOwnerRequest: `estateupdateinfo` of the last invoice.
    pub(super) fn dispatch_estate(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
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
        }
        Ok(true)
    }
}

/// The capabilities `refresh_estate` reads.
#[derive(Debug, Clone, Copy)]
enum Get {
    Access,
    Experiences,
    Environment,
    Schedule,
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
        }
    }
}
