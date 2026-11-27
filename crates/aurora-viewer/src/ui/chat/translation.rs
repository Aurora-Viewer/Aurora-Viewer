use super::LineEnv;
use crate::chat_translation::{Outcome, Settings};
use crate::theme::Palette;
use crate::ui::{
    context::{CtxAction, request},
    icons::Icons,
    menu, widgets,
};
use crate::world::ChatLine;
use aurora_i18n::tr;
use egui::{PopupCloseBehavior, RichText};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Controls {
    ready: bool,
    enabled: bool,
    automatic: bool,
    target: String,
}

impl Controls {
    pub fn update(&mut self, settings: &Settings) {
        self.ready = settings.ready();
        self.enabled = settings.enabled && self.ready;
        self.automatic = settings.automatic_nearby;
        self.target.clone_from(&settings.target);
    }

    pub fn offline_demo(&mut self) {
        self.ready = true;
        self.enabled = true;
        self.automatic = false;
        self.target.clear();
        self.target.push_str("en");
    }
}

pub(super) fn toolbar(ui: &mut egui::Ui, p: &Palette, icons: &Icons, controls: &Controls, characters: usize) {
    let button = super::tool_resp(ui, p, icons, "language", controls.enabled).on_hover_text(tr!("ui-chat-translation-menu"));
    menu::dropdown(&button, p, PopupCloseBehavior::CloseOnClick, |ui| {
        if menu::check(
            ui,
            p,
            "language",
            tr!("ui-options-chat-translation-on"),
            controls.enabled,
            controls.ready,
        ) {
            request(ui.ctx(), CtxAction::EnableChatTranslation(!controls.enabled));
        }
        if menu::check(
            ui,
            p,
            "message",
            tr!("ui-options-chat-translation-auto"),
            controls.automatic,
            controls.enabled,
        ) {
            request(ui.ctx(), CtxAction::AutoTranslateNearby(!controls.automatic));
        }
        let label = if controls.ready {
            tr!("ui-chat-translation-settings")
        } else {
            tr!("ui-chat-translation-connect")
        };
        if menu::item(ui, p, "settings", label) {
            request(ui.ctx(), CtxAction::OpenChatTranslation);
        }
        let count = aurora_i18n::integer(characters as i64);
        ui.label(RichText::new(tr!("ui-chat-translation-used", count = count.as_str())).color(p.muted));
        ui.label(RichText::new(tr!("ui-chat-translation-f0")).color(p.muted));
        if menu::item(ui, p, "external-link", tr!("ui-chat-translation-azure-usage")) {
            ui.ctx().open_url(egui::OpenUrl::new_tab("https://portal.azure.com/"));
        }
    });
}

pub(super) fn message(
    ui: &mut egui::Ui,
    env: &LineEnv,
    controls: &Controls,
    conversation: Option<Uuid>,
    line: &ChatLine,
    (token, seq): (u64, u64),
    inset: f32,
) {
    let worker = &env.world.chat_translation;
    let outcome = worker.outcome(token, seq);
    let pending = worker.pending(token, seq);
    let mut detected = None;
    let offer = controls.enabled
        && !pending
        && !matches!(outcome, Some(Outcome::Translated(_) | Outcome::SameLanguage))
        && env.world.translation_text(conversation, line).is_some_and(|text| {
            let id = egui::Id::unique(("chat_translation_language", token, seq));
            detected = ui.ctx().data_mut(|data| {
                if let Some(cached) = data.get_temp::<Option<whatlang::Lang>>(id) {
                    return cached;
                }
                let source = crate::chat_translation::detect_source(text);
                data.insert_temp(id, source);
                source
            });
            !detected.is_some_and(|source| crate::chat_translation::targets_source(source, &controls.target))
        });
    if outcome.is_none() && !pending && !offer {
        return;
    }
    ui.horizontal_top(|ui| {
        ui.add_space((inset - 20.0).max(0.0));
        if let Some(Outcome::Translated(text)) = outcome {
            if let Some(texture) = env.icons.get("language") {
                ui.add(egui::Image::new((texture.id(), egui::Vec2::splat(15.0))).tint(env.p.indigo_light))
                    .on_hover_text(tr!("ui-chat-translation-translated"));
            }
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .size(super::super::fonts::chat_size(ui.ctx(), 14.0))
                        .color(env.p.ink),
                )
                .wrap(),
            );
            return;
        }
        let tip = detected.map_or_else(
            || tr!("ui-chat-translate-message").to_owned(),
            |source| tr!("ui-chat-translate-detected", language = source.name()),
        );
        if offer && widgets::icon_button(ui, env.p, "language", &tip, true) {
            request(ui.ctx(), CtxAction::TranslateChatLine { conversation, token, seq });
        }
        let status = match outcome {
            Some(Outcome::SameLanguage) => Some(tr!("ui-chat-translation-same")),
            Some(Outcome::Credentials) => Some(tr!("ui-chat-translation-credentials")),
            Some(Outcome::Quota) => Some(tr!("ui-chat-translation-quota")),
            Some(Outcome::Unavailable) => Some(tr!("ui-chat-translation-unavailable")),
            _ if pending => Some(tr!("ui-chat-translation-pending")),
            _ => None,
        };
        if let Some(status) = status {
            ui.add(egui::Label::new(RichText::new(status).color(env.p.muted)).wrap());
        }
    });
}
