//! The agent's home location and directory settings: the HomeLocation and
//! UserInfo capabilities of the agent's region, or their UDP messages when
//! the region has none (`crate::home`, `crate::user_info`).

use super::Session;
use crate::types::NetEvent;
use crate::{home, user_info};
use glam::Vec3;

impl Session<'_> {
    /// LLAgent::setStartPosition: the capability, else (FIRE-19894)
    /// SetStartLocationRequest. Only the capability answers; the simulator's
    /// alert says whether the home was set either way.
    pub(super) fn set_home_location(&mut self, position: Vec3, look_at: Vec3) {
        log::info!("home: setting home at {position:.1?}");
        let Some(url) = self.main_cap("HomeLocation") else {
            let m = home::udp_request(self.agent_id(), self.session_id(), position, look_at);
            self.send_main(&m, true);
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let reply = home::post(&http, &url, position, look_at).await;
            log::info!("home: HomeLocation answered {reply:?}");
            let _ = events.send(NetEvent::HomeLocation(reply));
        });
    }

    /// LLAgent::sendAgentUserInfoRequest: the capability, else
    /// UserInfoRequest (answered by UserInfoReply).
    pub(super) fn request_user_info(&mut self) {
        let Some(url) = self.main_cap("UserInfo") else {
            let m = user_info::udp_request(self.agent_id(), self.session_id());
            self.send_main(&m, true);
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = user_info::fetch(&http, &url).await;
            if let Err(e) = &result {
                log::warn!("user info: failed to get user information ({e})");
            }
            let _ = events.send(NetEvent::UserInfo(result));
        });
    }

    /// LLAgent::sendAgentUpdateUserInfo: the capability, else UpdateUserInfo.
    pub(super) fn update_user_info(&mut self, directory_visibility: String, im_via_email: Option<bool>) {
        let Some(url) = self.main_cap("UserInfo") else {
            let m = user_info::udp_update(self.agent_id(), self.session_id(), &directory_visibility, im_via_email);
            self.send_main(&m, true);
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            if let Err(e) = user_info::update(&http, &url, &directory_visibility, im_via_email).await {
                log::warn!("user info: failed to set user information ({e})");
                let _ = events.send(NetEvent::UserInfoUpdateFailed(e));
            }
        });
    }
}
