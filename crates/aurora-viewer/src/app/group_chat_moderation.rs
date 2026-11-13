//! The confirmation of a group chat moderator's ejection or ban
//! (`ui::chat::moderation`), then the action: LLGroupActions::callbackEject,
//! which asks canEjectFromGroup again, and banMemberById
//! (indra/newview/llgroupactions.cpp, llgroupmgr.cpp).

use super::*;
use crate::ui::chat::moderation::{self, Confirm};

impl App {
    /// Each frame: the question asked, then what the bans sent tell
    /// (EjectAvatarFromGroup), at once or once their group was known.
    pub(super) fn group_chat_moderation_ui(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo && moderation::demo_mode().is_some() {
            // the demo account's restored Appearance window would cover
            // the conversation
            if self.frame_count == 300 {
                self.panels.appearance = false;
            }
            if self.frame_count == 650 {
                self.group_chat_moderation_demo();
            }
        }
        if let Some(c) = self.group_chat_confirm
            && let Some(yes) = moderation::confirm(ctx, p, &self.world, &c)
        {
            self.group_chat_confirm = None;
            if yes {
                self.apply_group_chat_moderation(c);
            }
        }
        for (group, member) in std::mem::take(&mut self.world.group_data.banned) {
            self.notify_ejected(group, &[member]);
        }
    }

    /// `AURORA_DEMO_CONV_MODERATE=eject|ban|banned|error`, on the builder
    /// shown as "Tess" in Aurora Builders (docs/TESTING.md).
    fn group_chat_moderation_demo(&mut self) {
        let (group, agent) = (crate::demo::DEMO_GROUP1, crate::demo::DEMO_NOVA);
        match moderation::demo_mode() {
            Some("eject") => self.group_chat_confirm = Some(Confirm::Eject { group, agent }),
            Some("ban") => self.group_chat_confirm = Some(Confirm::Ban { group, agent }),
            Some("banned") => self.apply_group_chat_moderation(Confirm::Ban { group, agent }),
            Some("error") => self.on_ctx_action(ui::context::CtxAction::ModerateGroupText {
                session: group,
                agent,
                allow: false,
            }),
            _ => {}
        }
    }

    fn apply_group_chat_moderation(&mut self, c: Confirm) {
        let me = self.world.agent_id;
        match c {
            Confirm::Eject { group, agent } => {
                let powers = moderation::powers_in(&self.world, &group);
                if !moderation::can_eject(self.world.group_data.get(&group), powers, agent, me) {
                    log::info!("group {group}: the ejection is no longer allowed");
                    return;
                }
                self.notify_ejected(group, &[agent]);
                self.world.group_data.eject(group, vec![agent], me);
            }
            Confirm::Ban { group, agent } => self.world.group_data.ban_member(group, agent, me),
        }
    }
}
