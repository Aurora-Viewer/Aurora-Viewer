//! "Set display name", ported from LLFloaterDisplayName (Second
//! Life viewer and Firestorm, originally LGPL 2.1, Linden Research, Inc.): the new name
//! typed twice, locked for a week after a change, reset to the username.

use super::widgets::{Floater, flat_button};
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Vec2};

/// Characters, not bytes.
const DISPLAY_NAME_MAX_LENGTH: usize = 31;

#[derive(Default)]
pub struct DisplayNameUi {
    pub open: bool,
    name: String,
    confirm: String,
    error: Option<String>,
}

/// Change to send: (old display name, new one; "" = back to the username).
pub type SetDisplayName = (String, String);

/// "10/15/2026 at 14:30 SLT" (Firestorm LockOutDateFormat, in SLT).
fn slt_datetime(secs: f64) -> String {
    let (y, m, d, _, h, mi, _) = crate::ui::land::slt_parts(secs as i64);
    aurora_i18n::tr!(
        "ui-display-name-at-slt",
        d = format!("{:02}", d),
        m = format!("{:02}", m),
        y = format!("{}", y),
        value0 = format!("{:02}", h),
        value1 = format!("{:02}", mi)
    )
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl DisplayNameUi {
    /// onOpen: empty fields.
    pub fn open(&mut self) {
        self.open = true;
        self.name.clear();
        self.confirm.clear();
        self.error = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World) -> Option<SetDisplayName> {
        let screen = ctx.content_rect();
        let mut out = None;
        let mut open = self.open;
        Floater::new(
            "display_name",
            aurora_i18n::tr!("common-set-display-name"),
            egui::pos2(screen.center().x - 200.0, screen.center().y - 150.0),
            Vec2::new(400.0, 300.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            let names = &world.social.avatar_names;
            let me = names.get(&world.agent_id).cloned();
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ui-display-name-the-name-you-choose-for-your-avatar-is-your"))
                        .size(12.0)
                        .color(p.ink),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            let Some(me) = me else {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-display-name-your-name-is-not-yet-known"))
                        .size(12.0)
                        .color(p.muted),
                );
                return;
            };
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "ui-display-name-current-name",
                    value0 = me.display_name.to_string(),
                    value1 = me.username.to_string()
                ))
                .size(12.0)
                .color(p.muted),
            );
            let locked = now() < me.next_update;
            if locked {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!(
                        "ui-display-name-you-cannot-change-your-name-before",
                        value0 = slt_datetime(me.next_update).to_string()
                    ))
                    .size(12.0)
                    .color(p.danger),
                );
            }
            ui.add_space(8.0);
            ui.add_enabled_ui(!locked, |ui| {
                ui.label(RichText::new(aurora_i18n::tr!("ui-display-name-new-name")).size(12.0).color(p.ink));
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-display-name-retype-the-name"))
                        .size(12.0)
                        .color(p.ink),
                );
                ui.add(egui::TextEdit::singleline(&mut self.confirm).desired_width(f32::INFINITY));
            });
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(e).size(12.0).color(p.danger)).wrap());
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                // FIRE-33330: the reset is allowed during the lock-out week
                if flat_button(ui, p, aurora_i18n::tr!("common-reset"))
                    .on_hover_text(aurora_i18n::tr!("ui-display-name-use-username-as-display-name"))
                    .clicked()
                {
                    out = Some((me.display_name.clone(), String::new()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                        self.open = false;
                    }
                    let save = ui
                        .add_enabled_ui(!locked, |ui| {
                            flat_button(ui, p, aurora_i18n::tr!("common-save"))
                                .on_hover_text(aurora_i18n::tr!("ui-display-name-saves-your-new-name"))
                        })
                        .inner;
                    if save.clicked() {
                        // onSave
                        let name = self.name.trim().to_owned();
                        let user_name = if me.legacy_last.is_empty() || me.legacy_last == "Resident" {
                            me.legacy_first.clone()
                        } else {
                            format!("{} {}", me.legacy_first, me.legacy_last)
                        };
                        if name != self.confirm.trim() {
                            self.error = Some(aurora_i18n::tr!("ui-display-name-the-display-names-do-not-match-please-enter-them").into());
                        } else if name.is_empty() {
                            self.error = Some(aurora_i18n::tr!("ui-display-name-enter-a-name").into());
                        } else if name == user_name {
                            // a reset
                            out = Some((me.display_name.clone(), String::new()));
                        } else if name.chars().count() > DISPLAY_NAME_MAX_LENGTH {
                            self.error = Some(aurora_i18n::tr!(
                                "ui-display-name-the-name-is-too-long-the-maximum-length-is",
                                DISPLAY_NAME_MAX_LENGTH = format!("{}", DISPLAY_NAME_MAX_LENGTH)
                            ));
                        } else {
                            out = Some((me.display_name.clone(), name));
                        }
                    }
                });
            });
        });
        if out.is_some() {
            open = false;
        }
        self.open = open && self.open;
        out
    }
}

/// Text of the answer to a display name change (onCacheSetName): the
/// welcome message, or the error.
pub fn reply_message(status: i32, content: &aurora_llsd::Llsd) -> Result<String, String> {
    if status == 200 {
        let name = content["display_name"].to_string_value();
        return Ok(aurora_i18n::tr!(
            "ui-display-name-hello-as-in-real-life-people-need-some-time",
            name = name.to_string()
        ));
    }
    // known error tags (notifications.xml)
    let tag = content["error_tag"].to_string_value();
    let known = match tag.as_str() {
        "SetDisplayNameBlocked" => Some(aurora_i18n::tr!(
            "ui-display-name-cannot-change-your-display-name-if-you-think-this"
        )),
        "SetDisplayNameMismatch" => Some(aurora_i18n::tr!("ui-display-name-the-display-names-do-not-match-please-enter-them")),
        "AgentDisplayNameUpdateThresholdExceeded" => {
            Some(aurora_i18n::tr!("ui-display-name-you-cannot-change-your-display-name-yet-see-http"))
        }
        "AgentDisplayNameSetBlocked" => Some(aurora_i18n::tr!(
            "ui-display-name-cannot-set-this-name-because-it-contains-a-prohibited"
        )),
        "AgentDisplayNameSetInvalidUnicode" => Some(aurora_i18n::tr!(
            "ui-display-name-your-requested-display-name-contains-invalid-characters"
        )),
        "AgentDisplayNameSetOnlyPunctuation" => Some(aurora_i18n::tr!(
            "ui-display-name-your-display-name-must-contain-characters-other-than-punctuation"
        )),
        _ => None,
    };
    if let Some(m) = known {
        return Err(m.to_owned());
    }
    // Prefer the viewer language when the server has a localized message.
    let desc = &content["error_description"];
    for lang in [aurora_i18n::language(), "en"] {
        let m = desc[lang].to_string_value();
        if !m.is_empty() {
            return Err(m);
        }
    }
    Err(aurora_i18n::tr!("ui-display-name-could-not-set-your-display-name-please-try-again").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockout_date_in_slt() {
        // 2026-10-15 21:30 UTC = 14:30 PDT
        assert_eq!(
            slt_datetime(1_792_099_800.0),
            aurora_i18n::tr!(
                "ui-display-name-at-slt",
                d = "15",
                m = "10",
                y = "2026",
                value0 = "14",
                value1 = "30"
            )
            .as_str()
        );
    }
}
