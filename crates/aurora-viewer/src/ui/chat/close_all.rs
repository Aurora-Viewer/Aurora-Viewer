//! Close All in the Legacy Viewer's IM well (LLIMWellWindow::closeAll and
//! closeAllImpl, indra/newview/llsyswellwindow.cpp).

use super::ChatUi;
use crate::theme::Palette;
use crate::ui::{context, menu};
use crate::world::{World, calls::CallState};
use aurora_i18n::tr;
use uuid::Uuid;

pub(super) fn menu_item(ui: &mut egui::Ui, p: &Palette, has_open_ims: bool) -> bool {
    menu::item_if(ui, p, "x", tr!("ui-chat-close-all-ims"), has_open_ims)
}

impl ChatUi {
    /// The same session leave, call teardown and window cleanup as the tab's X.
    pub fn close_im(&mut self, world: &mut World, id: Uuid) {
        world.conversation_closed(&id);
        world.leave_session(id);
        self.forget(id);
        if self.selected == Some(id) {
            self.selected = None;
            self.show_profile = false;
        }
        if world.social.focus_im == Some(id) {
            world.social.focus_im = None;
            world.social.focus_im_to_type = false;
        }
    }

    pub fn request_close_all(&mut self, world: &mut World, confirm_voice: bool) {
        // A request for channel information has no IM voice channel yet
        // (LLSpeakerMgr::isVoiceActive). Nearby voice is independent.
        if confirm_voice
            && world.calls.active.as_ref().is_some_and(|call| {
                call.state != CallState::Requesting && world.social.ims.iter().any(|im| im.other == call.conversation())
            })
        {
            self.close_all_prompt.get_or_insert(false);
        } else {
            self.close_all(world);
        }
    }

    fn close_all(&mut self, world: &mut World) {
        // Nearby chat is separate from the list of IM sessions, even when detached.
        while let Some(id) = world.social.ims.last().map(|im| im.other) {
            self.close_im(world, id);
        }
        self.close_all_prompt = None;
    }

    /// Returns true when the user accepted and asked to skip future prompts.
    pub fn close_all_dialog(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World) -> bool {
        let Some(dont_ask) = &mut self.close_all_prompt else {
            return false;
        };
        let question = context::Question {
            id: "close_all_ims",
            title: tr!("ui-chat-close-all-ims"),
            text: tr!("ui-chat-close-all-ims-question"),
            yes: tr!("common-ok"),
            no: tr!("common-cancel"),
        };
        let Some(yes) = context::ask(ctx, p, &question, dont_ask) else {
            return false;
        };
        let skip = self.close_all_prompt.take() == Some(true);
        if yes {
            self.close_all(world);
        }
        yes && skip
    }
}
