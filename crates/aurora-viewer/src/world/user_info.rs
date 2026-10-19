//! The account settings of Preferences › Privacy that the grid keeps, as
//! LLFloaterPreference reads and writes them (onOpen, setPersonalInfo,
//! onBtnOK, processProperties / saveAvatarProperties,
//! indra/newview/llfloaterpreference.cpp, originally LGPL 2.1):
//!
//! - "Only friends and groups know I'm online" and "Email me IMs when I'm
//!   offline" (the UserInfo capability, `aurora_net::user_info`), asked each
//!   time the window opens; the second only off Second Life, which keeps it
//!   on its account web page;
//! - "Show me in search results", our profile's ALLOW_PUBLISH flag (the
//!   profile window's same checkbox), our profile asked again at the
//!   opening.
//!
//! The Legacy Viewer sends them with OK; Aurora's preferences apply at
//! once, like its other settings.

use super::{World, notifications};
use aurora_net::NetCommand;
use aurora_net::user_info::{self, UserInfo};

#[derive(Debug, Default)]
pub struct UserInfoState {
    /// What the server answered (mGotPersonalInfo once known).
    pub info: Option<UserInfo>,
    /// The preferences window was open last frame, logged in.
    window_open: bool,
    /// The grid is Second Life (the e-mail flag is left out).
    second_life: bool,
    out: Vec<NetCommand>,
}

/// A change made in the preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    HideOnline(bool),
    ImViaEmail(bool),
    ShowInSearch(bool),
}

impl UserInfoState {
    /// LLFloaterPreference::onOpen asks again each time; `second_life` is
    /// the grid logged in.
    pub fn follow_window(&mut self, open: bool, logged_in: bool, second_life: bool) -> bool {
        let opened = open && logged_in && !self.window_open;
        self.window_open = open && logged_in;
        if !logged_in {
            self.info = None;
        }
        self.second_life = second_life;
        if opened {
            self.out.push(NetCommand::RequestUserInfo);
        }
        opened
    }

    /// requestAgentUserInfoCoro / process_user_info_reply.
    pub fn answer(&mut self, result: Result<UserInfo, user_info::Error>) {
        match result {
            Ok(mut info) => {
                // Second Life's flag lives on its web page (FS keeps it for OpenSim)
                if self.second_life {
                    info.im_via_email = false;
                }
                self.info = Some(info);
            }
            Err(e) => log::warn!("user info: not known ({e})"),
        }
    }

    /// onBtnOK: the visibility and the e-mail flag go together, the flag
    /// only off Second Life.
    fn send(&mut self) {
        if let Some(info) = &self.info {
            self.out.push(NetCommand::UpdateUserInfo {
                directory_visibility: info.directory_visibility.clone(),
                im_via_email: (!self.second_life).then_some(info.im_via_email),
            });
        }
    }

    /// The checkbox only changes "default" and "hidden" (a liaison's
    /// visibility stays as it is).
    pub fn set_hide_online(&mut self, hide: bool) {
        let Some(info) = self.info.as_mut() else { return };
        if !info.visibility_editable() || info.hide_online() == hide {
            return;
        }
        info.directory_visibility = user_info::visibility(hide).to_owned();
        self.send();
    }

    pub fn set_im_via_email(&mut self, on: bool) {
        let Some(info) = self.info.as_mut() else { return };
        if self.second_life || info.im_via_email == on {
            return;
        }
        info.im_via_email = on;
        self.send();
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

impl World {
    /// The preferences window's frame: asks the account settings and our
    /// profile when it opens.
    pub fn follow_preferences(&mut self, open: bool, second_life: bool) {
        if self.user_info.follow_window(open, self.login.is_some(), second_life) {
            self.profiles.refresh(self.agent_id);
        }
    }

    /// "Show me in search results": our profile's flag, None until known.
    pub fn show_in_search(&self) -> Option<bool> {
        self.profiles.get(&self.agent_id).map(|p| p.allow_publish())
    }

    /// A change of the Privacy page, and the command it needs.
    pub fn change_user_info(&mut self, change: Change) -> Option<NetCommand> {
        match change {
            Change::HideOnline(hide) => self.user_info.set_hide_online(hide),
            Change::ImViaEmail(on) => self.user_info.set_im_via_email(on),
            // saveAvatarProperties: the AgentProfile capability
            Change::ShowInSearch(publish) => {
                return (self.show_in_search() != Some(publish))
                    .then(|| self.save_profile(self.agent_id, aurora_llsd::llsd_map! { "allow_publish" => publish }));
            }
        }
        None
    }

    /// The change did not go through: what the server has is asked again.
    pub(super) fn on_user_info_update_failed(&mut self, error: user_info::Error) {
        log::warn!("user info: the change was not saved ({error})");
        self.user_info.out.push(NetCommand::RequestUserInfo);
        let text = aurora_i18n::tr!("world-user-info-update-failed");
        self.system_message(text);
        self.notifications.push(
            notifications::Kind::Alert,
            aurora_i18n::tr!("ui-options-privacy"),
            text.to_owned(),
            notifications::Data::None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::user_info::{Error, VISIBILITY_DEFAULT};

    fn known(state: &mut UserInfoState, visibility: &str, email: bool) {
        state.answer(Ok(UserInfo {
            directory_visibility: visibility.into(),
            im_via_email: email,
            email: "resident@example.com".into(),
        }));
    }

    #[test]
    fn asked_at_each_opening_while_logged_in() {
        let mut s = UserInfoState::default();
        assert!(!s.follow_window(true, false, true), "not before login");
        assert!(s.follow_window(true, true, true));
        assert!(!s.follow_window(true, true, true), "once per opening");
        s.follow_window(false, true, true);
        assert!(s.follow_window(true, true, true));
        assert!(matches!(
            s.take_commands()[..],
            [NetCommand::RequestUserInfo, NetCommand::RequestUserInfo]
        ));
        known(&mut s, "hidden", false);
        s.follow_window(false, false, true);
        assert!(s.info.is_none(), "forgotten at logout");
    }

    #[test]
    fn hiding_online_sends_the_visibility_without_the_mail_flag_on_second_life() {
        let mut s = UserInfoState::default();
        s.follow_window(true, true, true);
        s.take_commands();
        s.set_hide_online(true);
        assert!(s.take_commands().is_empty(), "nothing before the answer");
        known(&mut s, VISIBILITY_DEFAULT, true);
        assert!(!s.info.as_ref().expect("info").im_via_email, "Second Life's flag is not shown");
        s.set_hide_online(true);
        match &s.take_commands()[..] {
            [
                NetCommand::UpdateUserInfo {
                    directory_visibility,
                    im_via_email: None,
                },
            ] => assert_eq!(directory_visibility, "hidden"),
            other => panic!("{other:?}"),
        }
        s.set_hide_online(true);
        assert!(s.take_commands().is_empty(), "unchanged");
        s.set_im_via_email(true);
        assert!(s.take_commands().is_empty(), "no e-mail flag on Second Life");
    }

    #[test]
    fn other_grids_send_the_mail_flag_with_the_visibility() {
        let mut s = UserInfoState::default();
        s.follow_window(true, true, false);
        s.take_commands();
        known(&mut s, "hidden", false);
        s.set_im_via_email(true);
        match &s.take_commands()[..] {
            [
                NetCommand::UpdateUserInfo {
                    directory_visibility,
                    im_via_email: Some(true),
                },
            ] => assert_eq!(directory_visibility, "hidden"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_liaison_visibility_is_not_changed() {
        let mut s = UserInfoState::default();
        s.follow_window(true, true, true);
        s.take_commands();
        known(&mut s, "liaison", false);
        s.set_hide_online(false);
        assert!(s.take_commands().is_empty());
        assert_eq!(s.info.as_ref().expect("info").directory_visibility, "liaison");
    }

    #[test]
    fn a_failed_read_keeps_the_page_waiting_and_a_failed_change_asks_again() {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.apply(aurora_net::NetEvent::UserInfo(Err(Error::Http("status 502".into()))));
        assert!(w.user_info.info.is_none());
        known(&mut w.user_info, "hidden", false);
        w.apply(aurora_net::NetEvent::UserInfoUpdateFailed(Error::Refused("busy".into())));
        assert!(matches!(w.user_info.take_commands()[..], [NetCommand::RequestUserInfo]));
        assert_eq!(w.notifications.list.len(), 1);
    }
}
