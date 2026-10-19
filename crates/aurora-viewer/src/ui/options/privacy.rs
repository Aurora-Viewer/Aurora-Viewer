//! Preferences › Privacy: what the grid shows of us ("Show me in search
//! results", "Only friends and groups know I'm online" of
//! panel_preferences_privacy.xml, and "Email me IMs when I'm offline" of
//! panel_preferences_chat.xml; kept by the grid, `world/user_info.rs`),
//! "Only friends and groups can call or IM me" and the objects' messages
//! from elsewhere (`world/object_im_filter.rs`), then the Legacy Viewer's
//! "Privacy › LookAt" tab (tab-lookat): the crosshairs of where the avatars
//! look, and what we send of where we look.

use super::{group, row, slider_whole, toggle};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::lookat::{MAX_DISTANCE, Settings};
use crate::world::object_im_filter::{self, Scope};
use crate::world::user_info::Change;
use aurora_net::user_info::UserInfo;

/// Second Life's page for the e-mail of offline IMs (email_settings of
/// panel_preferences_chat.xml).
const SECOND_LIFE_EMAIL_SETTINGS: &str = "https://accounts.secondlife.com/change_email";

/// The account settings the grid keeps, while logged in.
#[derive(Debug, Clone, Default)]
pub struct Account {
    /// The UserInfo answer, None until it comes (the switches wait).
    pub info: Option<UserInfo>,
    /// Our profile's "show in search" flag, None until it comes.
    pub show_in_search: Option<bool>,
    /// Second Life keeps the e-mail of offline IMs on its web page.
    pub second_life: bool,
}

/// setPersonalInfo: the address in the label, cut at 30 characters.
fn shown_email(email: &str) -> String {
    if email.chars().count() > 30 {
        format!("{}...", email.chars().take(30).collect::<String>())
    } else {
        email.to_owned()
    }
}

/// The group of the settings the grid keeps; before login, a line says so
/// (email_settings_login_to_change).
fn account_group(ui: &mut egui::Ui, p: &Palette, account: Option<&Account>, change: &mut Option<Change>) {
    group(ui, p, aurora_i18n::tr!("ui-options-account-visibility"), |ui| {
        let Some(account) = account else {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-options-account-visibility-login"))
                    .size(11.5)
                    .color(p.muted_dim),
            );
            return;
        };
        let mut publish = account.show_in_search.unwrap_or(false);
        ui.add_enabled_ui(account.show_in_search.is_some(), |ui| {
            if row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-show-me-in-search"),
                aurora_i18n::tr!("ui-options-show-me-in-search-hint"),
                |ui| toggle(ui, p, &mut publish),
            ) {
                *change = Some(Change::ShowInSearch(publish));
            }
        });
        let info = account.info.as_ref();
        let mut hide = info.is_some_and(UserInfo::hide_online);
        ui.add_enabled_ui(info.is_some_and(UserInfo::visibility_editable), |ui| {
            if row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-online-friends-only"),
                aurora_i18n::tr!("ui-options-online-friends-only-hint"),
                |ui| toggle(ui, p, &mut hide),
            ) {
                *change = Some(Change::HideOnline(hide));
            }
        });
        if account.second_life {
            // the setting lives on the account's web page
            row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-offline-im-email"),
                aurora_i18n::tr!("ui-options-offline-im-email-web-hint"),
                |ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "external-link",
                        aurora_i18n::tr!("ui-options-offline-im-email-open"),
                        widgets::ActionKind::Flat,
                        egui::Vec2::new(0.0, 24.0),
                    )
                    .clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(SECOND_LIFE_EMAIL_SETTINGS));
                    }
                    false
                },
            );
        } else {
            let mut email = info.is_some_and(|i| i.im_via_email);
            let address = info.map(|i| shown_email(&i.email)).unwrap_or_default();
            ui.add_enabled_ui(info.is_some(), |ui| {
                if row(
                    ui,
                    p,
                    &aurora_i18n::tr!("ui-options-offline-im-email-to", email = address),
                    aurora_i18n::tr!("ui-options-offline-im-email-hint"),
                    |ui| toggle(ui, p, &mut email),
                ) {
                    *change = Some(Change::ImViaEmail(email));
                }
            });
        }
    });
}

/// Name formats of the crosshairs (DebugLookAtShowNames), in its order.
fn name_formats() -> [&'static str; 5] {
    [
        aurora_i18n::tr!("common-none"),
        aurora_i18n::tr!("ui-options-look-at-complete-name"),
        aurora_i18n::tr!("common-display-name"),
        aurora_i18n::tr!("ui-options-look-at-legacy-name"),
        aurora_i18n::tr!("common-username"),
    ]
}

pub fn page(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut Settings,
    friends_only: &mut bool,
    filter: &mut object_im_filter::Settings,
    account: Option<&Account>,
    change: &mut Option<Change>,
) -> bool {
    let mut c = false;
    account_group(ui, p, account, change);
    // "Only friends and groups can call or IM me" (voice_call_friends_only_check,
    // VoiceCallsFriendsOnly; its hint is the FriendsAndGroupsOnly notice)
    group(ui, p, aurora_i18n::tr!("ui-options-calls-and-messages"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-calls-friends-only"),
            aurora_i18n::tr!("ui-options-calls-friends-only-hint"),
            |ui| toggle(ui, p, friends_only),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-block-distant-object-messages"),
            aurora_i18n::tr!("ui-options-block-distant-object-messages-hint"),
            |ui| toggle(ui, p, &mut filter.on),
        );
        ui.add_enabled_ui(filter.on, |ui| {
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-object-messages-from"),
                aurora_i18n::tr!("ui-options-object-messages-from-hint"),
                |ui| {
                    let mut changed = false;
                    ui.horizontal_wrapped(|ui| {
                        for (value, label) in [
                            (Scope::Region, aurora_i18n::tr!("ui-options-object-messages-from-region")),
                            (Scope::Parcel, aurora_i18n::tr!("ui-options-object-messages-from-parcel")),
                        ] {
                            changed |= widgets::radio_value(ui, p, &mut filter.scope, value, label).changed();
                        }
                    });
                    changed
                },
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-object-messages-allow-friends"),
                aurora_i18n::tr!("ui-options-object-messages-allow-friends-hint"),
                |ui| toggle(ui, p, &mut filter.allow_friends),
            );
        });
    });
    group(ui, p, aurora_i18n::tr!("ui-options-look-at-targets"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-show-look-at-targets"),
            aurora_i18n::tr!("ui-options-show-look-at-targets-help"),
            |ui| toggle(ui, p, &mut s.show_targets),
        );
        // enabled_control="DebugLookAt"
        ui.add_enabled_ui(s.show_targets, |ui| {
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-look-at-names"),
                aurora_i18n::tr!("ui-options-look-at-names-help"),
                |ui| {
                    let formats = name_formats();
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("look-at-names")
                        .selected_text(formats[s.show_names.min(4) as usize])
                        .show_ui(ui, |ui| {
                            for (i, label) in formats.iter().enumerate() {
                                changed |= ui.selectable_value(&mut s.show_names, i as u8, *label).changed();
                            }
                        });
                    changed
                },
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-hide-own-look-at"),
                aurora_i18n::tr!("ui-options-hide-own-look-at-help"),
                |ui| toggle(ui, p, &mut s.hide_own),
            );
            c |= row(
                ui,
                p,
                aurora_i18n::tr!("ui-options-look-at-lines"),
                aurora_i18n::tr!("ui-options-look-at-lines-help"),
                |ui| toggle(ui, p, &mut s.lines),
            );
        });
    });
    group(ui, p, aurora_i18n::tr!("ui-options-my-look-at"), |ui| {
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-private-look-at"),
            aurora_i18n::tr!("ui-options-private-look-at-help"),
            |ui| toggle(ui, p, &mut s.private),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-private-local-look-at"),
            aurora_i18n::tr!("ui-options-private-local-look-at-help"),
            |ui| toggle(ui, p, &mut s.private_local),
        );
        c |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-limit-look-at-distance"),
            aurora_i18n::tr!("ui-options-more-distant-targets-are-limited-to-this-distance-from"),
            |ui| {
                let mut changed = toggle(ui, p, &mut s.limit_distance);
                changed |= slider_whole(ui, &mut s.max_distance, MAX_DISTANCE, " m");
                changed
            },
        );
    });
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_address_is_cut_like_the_legacy_viewer() {
        assert_eq!(shown_email("resident@example.com"), "resident@example.com");
        let long = "a.very.long.resident.name@example-provider.com";
        assert_eq!(shown_email(long), "a.very.long.resident.name@exam...");
    }
}
