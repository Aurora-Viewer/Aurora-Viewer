//! Preferences › Avatars: everything about the avatars around in one
//! place, where the Legacy Viewer spreads it over several panels: their
//! name tags (panel_preferences_UI.xml) and Aurora's plates, the colours of
//! their names, the names themselves, the look-at targets
//! (panel_preferences_privacy.xml, tab-lookat) and the head that follows
//! the mouse (panel_preferences_firestorm.xml), the feet kept on the ground
//! and the animations of typing and speaking.

use super::super::cx::{Cx, field};
use super::super::{Page, swatch};
use crate::settings::PlateClick;

/// The name of a plate click action in its drop-down list: the names of
/// the avatar menu's entries they run.
fn plate_click_label(action: PlateClick) -> &'static str {
    match action {
        PlateClick::Nothing => aurora_i18n::tr!("ui-options-plate-click-nothing"),
        PlateClick::Profile => aurora_i18n::tr!("common-view-profile"),
        PlateClick::InstantMessage => aurora_i18n::tr!("ui-context-send-im"),
        PlateClick::OfferTeleport => aurora_i18n::tr!("common-offer-teleport"),
        PlateClick::Menu => aurora_i18n::tr!("ui-options-plate-click-menu"),
    }
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

pub fn page(cx: &mut Cx, tab: usize) {
    use aurora_i18n::tr;
    match tab {
        0 => {
            cx.group(tr!("ui-options-name-tags"), |cx| {
                cx.toggle(tr!("ui-options-show-own-tag"), "", &field!(show_own_name_tag));
                cx.toggle(tr!("ui-options-show-friend-tags"), "", &field!(show_friend_name_tags));
                cx.toggle(tr!("ui-options-show-other-tags"), "", &field!(show_other_name_tags));
                cx.toggle(
                    tr!("ui-options-temporary-tags"),
                    tr!("ui-options-temporary-tags-help"),
                    &field!(temporary_name_tags),
                );
                let off = !cx.s.temporary_name_tags;
                cx.subs(off, |cx| {
                    cx.slider_whole(
                        tr!("ui-options-tag-show-time"),
                        tr!("ui-options-tag-show-time-help"),
                        &field!(name_tag_show_seconds),
                        1.0..=60.0,
                        " s",
                    );
                });
                cx.slider(
                    tr!("ui-options-name-distance"),
                    tr!("ui-options-0-hide-avatar-names"),
                    &field!(name_tag_distance),
                    0.0..=128.0,
                    " m",
                );
                cx.toggle(
                    tr!("ui-options-complexity-in-name-tags"),
                    tr!("ui-options-below-the-name-from-green-to-red-based-on"),
                    &field!(tag_complexity),
                );
                let off = !cx.s.tag_complexity;
                cx.subs(off, |cx| {
                    cx.toggle(
                        tr!("ui-options-silhouettes-only"),
                        tr!("ui-options-only-avatars-above-the-limit-shown-as-silhouettes"),
                        &field!(tag_complexity_too_complex_only),
                    );
                    cx.toggle(
                        tr!("ui-options-mine-too"),
                        tr!("ui-options-your-complexity-below-your-name"),
                        &field!(tag_complexity_own),
                    );
                });
            });
            // the plates' settings mean nothing to the classic name tags
            cx.group(tr!("ui-options-aurora-avatar-plates"), |cx| {
                let classic = cx.s.classic_name_tags;
                cx.greyed(classic);
                cx.toggle(
                    tr!("ui-options-plates-hover-expand"),
                    tr!("ui-options-plates-hover-expand-help"),
                    &field!(plates_hover_expand),
                );
                cx.toggle(
                    tr!("ui-options-full-plates-friends-only"),
                    tr!("ui-options-full-plates-friends-only-help"),
                    &field!(plates_friends_only_full),
                );
                cx.toggle(
                    tr!("ui-options-plates-friend-tint"),
                    tr!("ui-options-plates-friend-tint-help"),
                    &field!(plates_friend_tint),
                );
                let choices: Vec<(PlateClick, &str)> = PlateClick::ALL.iter().map(|a| (*a, plate_click_label(*a))).collect();
                cx.combo(
                    tr!("ui-options-plate-click"),
                    tr!("ui-options-plate-click-help"),
                    &field!(plate_click),
                    &choices,
                );
                cx.combo(
                    tr!("ui-options-plate-shift-click"),
                    tr!("ui-options-plate-shift-click-help"),
                    &field!(plate_shift_click),
                    &choices,
                );
                cx.combo(
                    tr!("ui-options-plate-ctrl-click"),
                    tr!("ui-options-plate-ctrl-click-help"),
                    &field!(plate_ctrl_click),
                    &choices,
                );
            });
            // by relation (me, friends, blocked, Linden Lab) behind a switch,
            // off by default, for the plates and the classic tags alike;
            // then the classic tags' own pair
            cx.group(tr!("ui-options-name-tag-color"), |cx| {
                cx.note(tr!("ui-options-names-by-relation-note"));
                cx.toggle(tr!("ui-options-names-by-relation"), "", &field!(colors.relation_names));
                let off = !cx.s.colors.relation_names;
                let classic = cx.s.classic_name_tags;
                // the friends' color also tints their plates
                let tint = cx.s.plates_friend_tint && !classic;
                cx.subs(off, |cx| {
                    cx.color(tr!("ui-options-my-name"), "", &field!(colors.tag_me));
                    cx.live(tint).color(tr!("common-friends"), "", &field!(colors.tag_friend));
                    cx.color(tr!("ui-options-ignored"), "", &field!(colors.tag_muted));
                    cx.color("Linden Lab", "", &field!(colors.tag_linden));
                });
                let p = cx.p;
                let only = if classic { "" } else { tr!("ui-options-tag-colors-classic-only") };
                for (label, f) in [
                    (tr!("ui-options-matches"), field!(colors.tag_match)),
                    (tr!("ui-options-non-matches"), field!(colors.tag_mismatch)),
                ] {
                    let get = f.get_mut;
                    cx.row(label, only, &f, |ui, s| {
                        ui.add_enabled_ui(classic, |ui| swatch(ui, &p, ui.scope_id().with("swatch"), get(s)))
                            .inner
                    });
                }
            });
        }
        1 => {
            cx.group(tr!("ui-options-names"), |cx| {
                cx.toggle(
                    tr!("ui-options-display-names"),
                    tr!("ui-options-names-chosen-by-residents-otherwise-account-names"),
                    &field!(use_display_names),
                );
                let off = !cx.s.use_display_names;
                cx.subs(off, |cx| {
                    cx.toggle(
                        tr!("ui-options-with-username"),
                        tr!("ui-options-below-the-name-in-name-tags-beside-it-in"),
                        &field!(show_usernames),
                    );
                    cx.toggle(
                        tr!("ui-options-in-first-last-format"),
                        tr!("ui-options-jane-jane-doe-instead-of-jane-jane-doe"),
                        &field!(legacy_username_format),
                    );
                    cx.toggle(
                        tr!("ui-options-announce-name-changes"),
                        tr!("ui-options-in-chat-when-a-resident-changes-their-display-name"),
                        &field!(display_name_notices),
                    );
                });
                cx.toggle(
                    tr!("ui-options-hide-resident"),
                    tr!("ui-options-last-name-for-recent-accounts"),
                    &field!(trim_resident),
                );
                // Legacy Viewer › General: GroupTitlesTagMode, RenderHideGroupTitle
                let choices: Vec<(crate::world::names::GroupTitles, &str)> =
                    crate::world::names::GroupTitles::ALL.iter().map(|m| (*m, m.label())).collect();
                cx.combo(
                    tr!("ui-options-group-titles"),
                    tr!("ui-options-group-titles-help"),
                    &field!(group_titles),
                    &choices,
                );
            });
            // FSShowTypingStateInNameTag and FSTypingChevronPrefix
            cx.group(tr!("ui-options-typing"), |cx| {
                cx.toggle(
                    tr!("ui-options-typing-in-name-tags"),
                    tr!("ui-options-typing-in-name-tags-hint"),
                    &field!(typing_in_name_tags),
                );
                cx.toggle(
                    tr!("ui-options-typing-chevron"),
                    tr!("ui-options-typing-chevron-hint"),
                    &field!(typing_chevron),
                );
            });
        }
        2 => {
            cx.group(tr!("ui-options-look-at-of-others"), |cx| {
                cx.toggle(
                    tr!("ui-options-show-look-at-targets"),
                    tr!("ui-options-show-look-at-targets-help"),
                    &field!(lookat.show_targets),
                );
                // enabled_control="DebugLookAt"
                let off = !cx.s.lookat.show_targets;
                cx.subs(off, |cx| {
                    let formats = name_formats();
                    let choices: Vec<(u8, &str)> = formats.iter().enumerate().map(|(i, l)| (i as u8, *l)).collect();
                    cx.combo(
                        tr!("ui-options-look-at-names"),
                        tr!("ui-options-look-at-names-help"),
                        &field!(lookat.show_names),
                        &choices,
                    );
                    cx.toggle(
                        tr!("ui-options-hide-own-look-at"),
                        tr!("ui-options-hide-own-look-at-help"),
                        &field!(lookat.hide_own),
                    );
                    cx.toggle(
                        tr!("ui-options-look-at-lines"),
                        tr!("ui-options-look-at-lines-help"),
                        &field!(lookat.lines),
                    );
                });
                cx.link(
                    tr!("ui-options-my-look-at"),
                    tr!("ui-options-my-look-at-is-private"),
                    tr!("ui-options-privacy"),
                    Page::Privacy,
                    0,
                );
            });
            // the Legacy Viewer › Avatar (panel_preferences_firestorm.xml)
            cx.group(tr!("ui-options-head-follows-mouse"), |cx| {
                cx.slider_whole(
                    tr!("ui-options-head-vertical-range"),
                    tr!("ui-options-head-vertical-range-help"),
                    &field!(lookat.head_pitch),
                    crate::world::lookat::HEAD_RANGE,
                    "°",
                );
                cx.slider_whole(
                    tr!("ui-options-head-horizontal-range"),
                    tr!("ui-options-head-horizontal-range-help"),
                    &field!(lookat.head_yaw),
                    crate::world::lookat::HEAD_RANGE,
                    "°",
                );
                cx.toggle(
                    tr!("ui-options-static-eyes"),
                    tr!("ui-options-static-eyes-help"),
                    &field!(lookat.static_eyes),
                );
            });
        }
        _ => {
            cx.group(tr!("ui-options-feet"), |cx| {
                cx.toggle(
                    tr!("ui-options-aurora-ground-feet"),
                    tr!("ui-options-aurora-ground-feet-help"),
                    &field!(ground_avatar_feet),
                );
                let off = !cx.s.ground_avatar_feet;
                cx.subs(off, |cx| {
                    cx.toggle(
                        tr!("ui-options-aurora-ground-feet-moving"),
                        tr!("ui-options-aurora-ground-feet-moving-help"),
                        &field!(ground_feet_while_moving),
                    );
                    cx.slider(
                        tr!("ui-options-aurora-ground-feet-height"),
                        tr!("ui-options-aurora-ground-feet-height-help"),
                        &field!(ground_feet_max_height),
                        0.0..=crate::settings::MAX_GROUND_FEET_HEIGHT,
                        " m",
                    );
                    cx.slider(
                        tr!("ui-options-aurora-ground-feet-avatars"),
                        tr!("ui-options-aurora-ground-feet-avatars-help"),
                        &field!(ground_feet_max_avatars),
                        0..=crate::settings::MAX_GROUND_FEET_AVATARS,
                        "",
                    );
                });
            });
            // PlayTypingAnim and FSTypeDuringEmote (panel_preferences_chat.xml)
            cx.group(tr!("ui-options-typing-and-voice"), |cx| {
                cx.toggle(
                    tr!("ui-options-play-typing-anim"),
                    tr!("ui-options-play-typing-anim-hint"),
                    &field!(play_typing_anim),
                );
                let off = !cx.s.play_typing_anim;
                cx.subs(off, |cx| {
                    cx.toggle(
                        tr!("ui-options-type-during-emote"),
                        tr!("ui-options-type-during-emote-hint"),
                        &field!(type_during_emote),
                    );
                });
                cx.toggle(tr!("voice-lip-sync"), "", &field!(audio.lip_sync));
            });
            // ANIM_AGENT_CUSTOMIZE, always played by the Legacy Viewer
            cx.group(tr!("ui-options-appearance-editing"), |cx| {
                cx.toggle(
                    tr!("ui-options-customize-animation"),
                    tr!("ui-options-customize-animation-hint"),
                    &field!(customize_animation),
                );
            });
        }
    }
}
