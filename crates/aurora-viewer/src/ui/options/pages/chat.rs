//! Preferences › Chat: nearby chat and conversations, the notifications
//! of messages (Legacy Viewer: Chat › Notifications, Notifications), the
//! keyword alerts (panel_preferences_chat.xml, "ChatKeywordAlerts"), the
//! transcripts (Privacy › Logs & Transcripts), away and the automatic
//! replies (Privacy › Autoresponse), groups and blocking.

use super::super::cx::{Cx, Nothing, field, fields};
use super::super::{COMBO_W, Folder, FolderChoice, parse_uuid_text, path_label, swatch, toggle, uuid_text};
use egui::{RichText, Vec2};

pub fn page(cx: &mut Cx, tab: usize) {
    use aurora_i18n::tr;
    match tab {
        0 => {
            cx.group(tr!("common-nearby-chat"), |cx| {
                cx.toggle(tr!("ui-options-show-time"), "", &field!(chat_timestamps));
                // FSShowChatChannel: the "/0" chip of the field (off, as in
                // the Legacy Viewer)
                cx.toggle(
                    tr!("ui-options-show-chat-channel"),
                    tr!("ui-options-show-chat-channel-hint"),
                    &field!(show_chat_channel),
                );
                cx.toggle(
                    tr!("ui-options-chat-keep-focus"),
                    tr!("ui-options-chat-keep-focus-hint"),
                    &field!(chat_keep_focus),
                );
                cx.slider(
                    tr!("ui-options-chat-bubble-duration"),
                    tr!("ui-options-0-no-chat-bubbles-above-the-chat-bar"),
                    &field!(chat_toast_seconds),
                    0.0..=60.0,
                    " s",
                );
            });
            cx.group(tr!("ui-options-conversations"), |cx| {
                cx.toggle(
                    tr!("ui-options-show-contacts-tab"),
                    tr!("ui-options-show-contacts-tab-hint"),
                    &field!(show_contacts_tab),
                );
                // FSSendTypingState: the IM_TYPING notices of a conversation
                // with one resident (LLIMModel::sendTypingState)
                cx.toggle(
                    tr!("ui-options-send-typing"),
                    tr!("ui-options-send-typing-hint"),
                    &field!(send_typing_state),
                );
                // the tab of a conversation with news flashes
                // (FSFloaterIMContainer::startFlashingTab), in violet and softly
                cx.toggle(
                    tr!("ui-options-flash-unread"),
                    tr!("ui-options-flash-unread-hint"),
                    &field!(flash_unread_conversations),
                );
                // "Flash chat toolbar button if a new IM arrives" (FSNotifyIMFlash)
                cx.toggle(
                    tr!("ui-options-flash-conversations-button"),
                    tr!("ui-options-flash-conversations-button-hint"),
                    &field!(flash_conversations_button),
                );
            });
            link_previews(cx);
        }
        1 => notifications(cx),
        2 => keyword_alerts(cx),
        3 => history(cx),
        4 => away_and_replies(cx),
        _ => cx.group(tr!("ui-options-groups-and-blocking"), |cx| {
            cx.toggle(
                tr!("ui-options-no-group-chats"),
                tr!("extra-reject-all-group-chats-fsmuteallgroups"),
                &field!(mute_all_groups),
            );
            cx.toggle(
                tr!("ui-options-groups-without-notices"),
                tr!("ui-options-reject-chat-from-groups-whose-notices-you-do-not"),
                &field!(mute_groups_without_notices),
            );
            // FSIgnoreAdHocSessions, FSReportIgnoredAdHocSession,
            // FSDontIgnoreAdHocFromFriends: the same switches as Online
            // status › Ad hoc conferences
            cx.toggle(
                tr!("ui-options-ignore-conferences"),
                tr!("ui-options-ignore-conferences-hint"),
                &field!(autoresponse.ignore_adhoc),
            );
            let off = !cx.s.autoresponse.ignore_adhoc;
            cx.subs(off, |cx| {
                cx.toggle(
                    tr!("ui-options-ignore-conferences-report"),
                    tr!("ui-options-ignore-conferences-report-hint"),
                    &field!(autoresponse.report_ignored_adhoc),
                );
                cx.toggle(
                    tr!("ui-options-ignore-conferences-friends"),
                    tr!("ui-options-ignore-conferences-friends-hint"),
                    &field!(autoresponse.adhoc_from_friends),
                );
            });
            cx.toggle(
                tr!("ui-options-report-in-chat"),
                tr!("ui-options-announce-additions-to-and-removals-from-the-block-list"),
                &field!(report_blocks),
            );
            // Legacy Viewer › Privacy (FSShowJoinedGroupInvitations)
            cx.toggle(
                tr!("ui-options-joined-group-invitations"),
                tr!("ui-options-joined-group-invitations-help"),
                &field!(show_joined_group_invitations),
            );
        }),
    }
}

/// The pictures and videos shown under the links of the conversations
/// (`link_preview.rs`): when they load, where, how large, the sites hidden.
fn link_previews(cx: &mut Cx) {
    use crate::link_preview::{Load, Player, Size};
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-link-previews"), |cx| {
        cx.choice(
            tr!("ui-options-link-previews-load"),
            tr!("ui-options-link-previews-load-hint"),
            &field!(link_previews.load),
            &[
                (Load::Never, tr!("ui-options-link-previews-never"), true),
                (Load::Trusted, tr!("ui-options-link-previews-trusted"), true),
                (Load::All, tr!("ui-options-link-previews-all"), true),
            ],
        );
        cx.toggle(
            tr!("ui-options-link-previews-ims"),
            tr!("ui-options-link-previews-ims-hint"),
            &field!(link_previews.ims),
        );
        cx.toggle(tr!("ui-options-link-previews-groups"), "", &field!(link_previews.groups));
        let off = !cx.s.link_previews.groups;
        cx.subs(off, |cx| {
            cx.toggle(
                tr!("ui-options-link-previews-blur"),
                tr!("ui-options-link-previews-blur-hint"),
                &field!(link_previews.blur_groups),
            );
        });
        cx.toggle(
            tr!("ui-options-link-previews-nearby"),
            tr!("ui-options-link-previews-nearby-hint"),
            &field!(link_previews.nearby),
        );
        cx.choice(
            tr!("ui-options-link-previews-player"),
            tr!("ui-options-link-previews-player-hint"),
            &field!(link_previews.player),
            &[
                (Player::InCard, tr!("ui-options-link-previews-in-card"), true),
                (Player::Window, tr!("ui-options-link-previews-in-window"), true),
            ],
        );
        cx.choice(
            tr!("ui-options-link-previews-size"),
            "",
            &field!(link_previews.size),
            &[
                (Size::Small, tr!("ui-options-link-previews-small"), true),
                (Size::Medium, tr!("ui-options-link-previews-medium"), true),
                (Size::Large, tr!("ui-options-link-previews-large"), true),
            ],
        );
        if !cx.s.link_previews.hidden_sites.is_empty() {
            let sites = cx.s.link_previews.hidden_sites.join(" ");
            cx.below().keywords(&sites).row(
                tr!("ui-options-link-previews-hidden-sites"),
                tr!("ui-options-link-previews-hidden-sites-hint"),
                &field!(link_previews.hidden_sites),
                |ui, s| {
                    let mut removed = None;
                    ui.horizontal_wrapped(|ui| {
                        for (i, site) in s.link_previews.hidden_sites.iter().enumerate() {
                            ui.label(egui::RichText::new(site).size(12.0).color(p.ink));
                            let tip = tr!("ui-options-link-previews-unhide", site = site.as_str());
                            if crate::ui::widgets::icon_button(ui, &p, "x", &tip, true) {
                                removed = Some(i);
                            }
                        }
                    });
                    if let Some(i) = removed {
                        s.link_previews.hidden_sites.remove(i);
                    }
                    removed.is_some()
                },
            );
        } else {
            cx.track(&field!(link_previews.hidden_sites));
        }
    });
}

/// The Legacy Viewer's Chat › Notifications (EnableIMChatPopups,
/// EnableGroupChatPopups, ChatOnlineNotification), Notifications › People
/// / Objects & Scripts (FSFlashOnMessage, FSFlashOnObjectIM,
/// FSFlashOnScriptDialog) and World (FSEnableGrowl, FSGrowlWhenActive).
fn notifications(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-toasts"), |cx| {
        cx.toggle(
            tr!("ui-options-toast-friends-online"),
            tr!("ui-options-toast-friends-online-hint"),
            &field!(toast_friends_online),
        );
        // where they stand with Aurora's center; the Legacy toasts keep
        // them in their own stack (top right, like its notification channel)
        let center = cx.s.notifications_style == crate::settings::NotificationsStyle::Aurora;
        let off = !cx.s.toast_friends_online;
        cx.subs(off, |cx| {
            let hint = if center {
                tr!("ui-options-toast-friends-position-hint")
            } else {
                tr!("ui-options-toast-friends-position-legacy")
            };
            cx.row(
                tr!("ui-options-toast-friends-position"),
                hint,
                &field!(presence_position),
                |ui, s| {
                    let mut changed = false;
                    ui.add_enabled_ui(center, |ui| {
                        egui::ComboBox::from_id_salt(ui.scope_id().with("presence-position"))
                            .width(COMBO_W)
                            .selected_text(s.presence_position.label())
                            .show_ui(ui, |ui| {
                                for at in crate::settings::PresencePosition::ALL {
                                    changed |= ui.selectable_value(&mut s.presence_position, at, at.label()).changed();
                                }
                            });
                    });
                    changed
                },
            );
        });
        // OnlineOfflinetoNearbyChat, beside its toast as in the Legacy
        // Viewer's "Notify me when my friends log in or out"
        cx.toggle(
            tr!("ui-options-friends-online-in-chat"),
            tr!("ui-options-friends-online-in-chat-hint"),
            &field!(friends_online_in_chat),
        );
        // #465: an offer of friendship in the middle of the screen
        cx.toggle(
            tr!("ui-options-friend-request-prompt"),
            tr!("ui-options-friend-request-prompt-hint"),
            &field!(friend_request_prompt),
        );
        cx.toggle(tr!("ui-options-toast-im"), tr!("ui-options-toast-im-hint"), &field!(toast_im));
        cx.toggle(tr!("ui-options-toast-group-im"), "", &field!(toast_group_im));
        // only what toasts can stay under the bell
        let any = cx.s.toast_im || cx.s.toast_group_im;
        let p = cx.p;
        cx.row(
            tr!("ui-options-ims-in-notification-center"),
            tr!("ui-options-ims-in-notification-center-hint"),
            &field!(ims_in_notification_center),
            |ui, s| ui.add_enabled_ui(any, |ui| toggle(ui, &p, &mut s.ims_in_notification_center)).inner,
        );
        cx.slider(tr!("ui-options-toast-duration"), "", &field!(im_toast_seconds), 3.0..=60.0, " s");
    });
    cx.group(tr!("ui-options-attention"), |cx| {
        cx.toggle(
            tr!("ui-options-flash-on-message"),
            tr!("ui-options-flash-on-message-hint"),
            &field!(flash_on_message),
        );
        let off = !cx.s.flash_on_message;
        cx.subs(off, |cx| {
            cx.toggle(tr!("ui-options-flash-on-object-im"), "", &field!(flash_on_object_im));
        });
        cx.toggle(
            tr!("ui-options-flash-on-script-dialog"),
            tr!("ui-options-flash-on-script-dialog-hint"),
            &field!(flash_on_script_dialog),
        );
        cx.toggle(
            tr!("ui-options-system-notifications"),
            tr!("ui-options-system-notifications-hint"),
            &field!(system_notifications),
        );
        let off = !cx.s.system_notifications;
        cx.subs(off, |cx| {
            cx.toggle(
                tr!("ui-options-system-notifications-when-active"),
                "",
                &field!(system_notifications_when_active),
            );
        });
    });
}

/// The rows greyed while the alerts are off, the sound's asset while its
/// switch is off, as the Legacy panel's enabled_control.
fn keyword_alerts(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-keywords"), |cx| {
        cx.toggle(
            tr!("ui-options-keywords-on"),
            tr!("ui-options-keywords-on-hint"),
            &field!(keywords.on),
        );
        cx.greyed(!cx.s.keywords.on);
        cx.below().row(
            tr!("ui-options-keywords-words"),
            tr!("ui-options-keywords-words-hint"),
            &field!(keywords.words),
            |ui, s| {
                ui.add(
                    egui::TextEdit::singleline(&mut s.keywords.words)
                        .desired_width(f32::INFINITY)
                        .hint_text(tr!("ui-options-keywords-words-example")),
                )
                .changed()
            },
        );
        cx.toggle(tr!("ui-options-keywords-in-chat"), "", &field!(keywords.in_chat));
        cx.toggle(tr!("ui-options-keywords-in-im"), "", &field!(keywords.in_im));
        cx.toggle(
            tr!("ui-options-keywords-speakers-name"),
            tr!("ui-options-keywords-speakers-name-hint"),
            &field!(keywords.speakers_name),
        );
        cx.toggle(tr!("ui-options-keywords-case-sensitive"), "", &field!(keywords.case_sensitive));
        cx.toggle(
            tr!("ui-options-keywords-whole-words"),
            tr!("ui-options-keywords-whole-words-hint"),
            &field!(keywords.whole_words),
        );
        let p = cx.p;
        cx.row(
            tr!("ui-options-keywords-color"),
            "",
            &fields!(keywords.change_color, keywords.color),
            |ui, s| {
                let mut ch = toggle(ui, &p, &mut s.keywords.change_color);
                ui.add_enabled_ui(s.keywords.change_color, |ui| {
                    ch |= swatch(ui, &p, "keyword-color", &mut s.keywords.color);
                });
                ch
            },
        );
        cx.wide().row_with(
            tr!("ui-options-keywords-sound"),
            tr!("ui-options-keywords-sound-hint"),
            &fields!(keywords.play_sound, keywords.sound),
            |ui, s, st, _| {
                let k = &mut s.keywords;
                let mut ch = toggle(ui, &p, &mut k.play_sound);
                ui.add_enabled_ui(k.play_sound, |ui| {
                    // typed for this asset; a reset of the page starts it again
                    let (typed_for, text) = st.keyword_sound_text.get_or_insert_with(|| (k.sound, uuid_text(k.sound)));
                    if *typed_for != k.sound {
                        (*typed_for, *text) = (k.sound, uuid_text(k.sound));
                    }
                    let valid = parse_uuid_text(text).is_some();
                    let edit = ui.add(
                        egui::TextEdit::singleline(text)
                            .desired_width(290.0)
                            .font(egui::TextStyle::Monospace)
                            .text_color(if valid { p.ink } else { p.danger })
                            .hint_text(tr!("ui-options-no-sound")),
                    );
                    if edit.changed()
                        && let Some(id) = parse_uuid_text(text)
                    {
                        (k.sound, *typed_for) = (id, id);
                        ch = true;
                    }
                    // force_sound: heard even with the interface sounds off
                    if crate::ui::widgets::flat_button(ui, &p, tr!("ui-options-listen")).clicked() && !k.sound.is_nil() {
                        crate::ui::sound_cues::preview(ui.ctx(), k.sound);
                    }
                    let default = crate::keywords::DEFAULT_SOUND;
                    if k.sound != default && crate::ui::widgets::flat_button(ui, &p, tr!("common-default")).clicked() {
                        (k.sound, *typed_for, *text) = (default, default, uuid_text(default));
                        ch = true;
                    }
                });
                ch
            },
        );
    });
}

/// What is written to the account's transcripts and where they are
/// (Legacy Viewer: Preferences › Privacy, "Logs & Transcripts").
fn history(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-recording"), |cx| {
        cx.toggle(tr!("ui-options-chat-logs-im"), tr!("ui-options-chat-logs-im-help"), &field!(log_im));
        cx.toggle(
            tr!("ui-options-chat-logs-nearby"),
            tr!("ui-options-chat-logs-nearby-help"),
            &field!(log_nearby),
        );
        cx.toggle(
            tr!("ui-options-chat-logs-show"),
            tr!("ui-options-chat-logs-show-help"),
            &field!(log_show_history),
        );
        // Legacy Viewer › Chat (FSRestoreOpenIMs)
        cx.toggle(tr!("ui-options-reopen-ims"), tr!("ui-options-reopen-ims-help"), &field!(reopen_ims));
        cx.toggle(
            tr!("ui-options-chat-logs-dated"),
            tr!("ui-options-chat-logs-dated-help"),
            &field!(log_dated_names),
        );
    });
    cx.group(tr!("ui-options-folder"), |cx| {
        cx.note(tr!("ui-options-chat-logs-note"));
        let p = cx.p;
        cx.below().row_with(
            tr!("ui-options-chat-logs-folder"),
            tr!("ui-options-chat-logs-folder-help"),
            &Nothing,
            |ui, _, st, r| {
                let Some(dir) = st.chat_log_dir.clone() else {
                    ui.label(RichText::new(tr!("ui-options-chat-logs-no-account")).size(11.5).color(p.muted));
                    return false;
                };
                ui.vertical(|ui| {
                    path_label(ui, &p, &dir, ui.available_width());
                    ui.horizontal(|ui| {
                        use crate::ui::widgets::{ActionKind, action_button};
                        let size = Vec2::new(0.0, 24.0);
                        if action_button(
                            ui,
                            &p,
                            "folder-open-filled",
                            tr!("ui-options-chat-logs-open"),
                            ActionKind::Flat,
                            size,
                        )
                        .clicked()
                        {
                            r.open_chat_logs = true;
                        }
                        // Legacy Viewer › Network & Files (InstantMessageLogPath):
                        // the files move with the folder
                        if action_button(ui, &p, "folder-filled", tr!("ui-options-change-folder"), ActionKind::Flat, size).clicked() {
                            r.folder = Some((Folder::ChatLogs, FolderChoice::Pick));
                        }
                        if st.chat_log_custom
                            && action_button(ui, &p, "restore", tr!("ui-options-default-folder"), ActionKind::Flat, size).clicked()
                        {
                            r.folder = Some((Folder::ChatLogs, FolderChoice::Default));
                        }
                        if action_button(
                            ui,
                            &p,
                            "trash-filled",
                            tr!("ui-options-chat-logs-erase"),
                            ActionKind::FlatDanger,
                            size,
                        )
                        .clicked()
                        {
                            st.confirm_erase_logs = true;
                        }
                    });
                    if !st.chat_log_status.is_empty() {
                        ui.label(RichText::new(&st.chat_log_status).size(11.5).color(p.muted));
                    }
                });
                false
            },
        );
    });
}

/// Away (Legacy Viewer › General: AvatarSitOnAway, QuitAfterSecondsOfAFK)
/// and Preferences › Privacy › Autoresponse 1 and 2.
fn away_and_replies(cx: &mut Cx) {
    use aurora_i18n::tr;
    // AFKTimeout and QuitAfterSecondsOfAFK both use zero for "never"
    // (indra/newview/skins/default/xui/en/panel_preferences_general.xml).
    fn timeout(cx: &mut Cx, label: &str, hint: &str, f: &super::super::cx::F<u32>) {
        let get = f.get_mut;
        cx.row(label, hint, f, |ui, s| {
            ui.spacing_mut().slider_width = super::super::SLIDER_W;
            ui.add(
                egui::Slider::new(get(s), 0..=60)
                    .custom_formatter(|v, _| {
                        if v == 0.0 {
                            tr!("ui-options-away-disabled").to_owned()
                        } else {
                            format!("{v:.0} min")
                        }
                    })
                    .custom_parser(|text| {
                        let text = text.trim();
                        if text.eq_ignore_ascii_case(tr!("ui-options-away-disabled")) {
                            Some(0.0)
                        } else {
                            aurora_i18n::parse_number(text.trim_end_matches("min").trim())
                        }
                    })
                    .trailing_fill(true),
            )
            .changed()
        });
    }
    cx.group(tr!("ui-options-away"), |cx| {
        timeout(
            cx,
            tr!("ui-options-away-after"),
            tr!("ui-options-become-away-after-this-time-without-keyboard-or-mouse"),
            &field!(autoresponse.away_after_minutes),
        );
        cx.toggle(
            tr!("ui-options-sit-when-away"),
            tr!("ui-options-sit-when-away-help"),
            &field!(autoresponse.sit_when_away),
        );
        timeout(
            cx,
            tr!("ui-options-quit-after-away"),
            tr!("ui-options-quit-after-away-help"),
            &field!(autoresponse.quit_after_away_minutes),
        );
    });
    /// A label over a multi-line response.
    fn text(cx: &mut Cx, label: &str, f: &super::super::cx::F<String>) {
        let get = f.get_mut;
        cx.below().row(label, "", f, |ui, s| {
            ui.add(egui::TextEdit::multiline(get(s)).desired_rows(2).desired_width(f32::INFINITY))
                .changed()
        });
    }
    cx.group(tr!("ui-options-auto-replies"), |cx| {
        cx.note(tr!("ui-options-enable-these-modes-in-communicate-online-status"));
        text(cx, tr!("ui-options-in-do-not-disturb-mode"), &field!(autoresponse.dnd_text));
        text(
            cx,
            tr!("ui-options-to-everyone-in-auto-reply-mode"),
            &field!(autoresponse.autorespond_text),
        );
        text(
            cx,
            tr!("ui-options-in-auto-respond-to-non-friends-mode"),
            &field!(autoresponse.nonfriends_text),
        );
        // each switch over the text it sends
        cx.wide().toggle(
            tr!("ui-options-reply-when-away"),
            tr!("ui-options-send-the-text-below-when-i-am-away"),
            &field!(autoresponse.send_away),
        );
        text(cx, tr!("ui-options-when-i-am-away"), &field!(autoresponse.away_text));
        cx.wide().toggle(
            tr!("ui-options-reply-to-blocked-residents"),
            tr!("ui-options-send-the-text-below-to-blocked-people-their-im"),
            &field!(autoresponse.send_muted),
        );
        text(cx, tr!("ui-options-to-blocked-people"), &field!(autoresponse.muted_text));
    });
    // the modes themselves, chosen in Communicate › Online status: the
    // page's reset turns them off as it did
    cx.track(&fields!(
        autoresponse.autorespond,
        autoresponse.autorespond_nonfriends,
        autoresponse.reject_teleports,
        autoresponse.reject_group_invites,
        autoresponse.reject_friendship,
    ));
    cx.group(tr!("ui-options-automatic-rejections"), |cx| {
        text(
            cx,
            tr!("ui-options-in-reject-teleports-mode"),
            &field!(autoresponse.reject_teleports_text),
        );
        cx.wide().toggle(
            tr!("ui-options-except-my-friends"),
            tr!("ui-options-do-not-reject-or-auto-respond-to-my-friends"),
            &field!(autoresponse.dont_reject_friends_teleports),
        );
        text(
            cx,
            tr!("ui-options-in-reject-friendship-requests-mode"),
            &field!(autoresponse.reject_friendship_text),
        );
    });
}
