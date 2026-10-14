//! "Reset this section" of the preferences: the settings each page shows,
//! put back to their defaults without touching the other pages. The list of
//! a page is written once, below; a test reads the code of every page and
//! fails when a setting it shows is missing from its list (or listed on the
//! wrong page), so a new row cannot escape the reset of its section.

use super::{TAB_AUDIO, TAB_AURORA, TAB_CAMERA, TAB_CHAT, TAB_COMMANDS, TAB_PRIVACY};
use crate::settings::Settings;

/// Shortcuts page (index of its tab).
pub const TAB_SHORTCUTS: usize = 8;

macro_rules! sections {
    ($( $tab:pat => [ $( $($field:ident).+ ),* $(,)? ] ),* $(,)?) => {
        /// Puts the settings shown by the page `tab` back to their defaults.
        #[allow(clippy::clone_on_copy)]
        pub fn reset(tab: usize, s: &mut Settings) {
            let d = Settings::default();
            match tab {
                $( $tab => { $( s.$($field).+ = d.$($field).+.clone(); )* } )*
                #[allow(unreachable_patterns)]
                _ => {}
            }
            if tab == TAB_AUDIO {
                // the push-to-talk keys are drawn on the Sound and voice page
                let action = crate::keybinds::Action::PushToTalk;
                for (slot, binding) in d.keybinds.get(action).into_iter().enumerate() {
                    s.keybinds.set(action, slot, binding);
                }
            }
            if tab == 0 {
                s.shadows = s.shadow_quality > 0;
            }
        }

        /// The settings of each page, as written above (tests).
        #[cfg(test)]
        pub(super) fn listed() -> Vec<(usize, Vec<String>)> {
            let mut out = Vec::new();
            for tab in 0..super::tab_count() {
                let mut fields: Vec<String> = Vec::new();
                $( if matches!(tab, $tab) { $( fields.push(stringify!($($field).+).replace(' ', "")); )* } )*
                out.push((tab, fields));
            }
            out
        }
    };
}

sections! {
    0 => [
        draw_distance, lod_factor, max_avatars, max_complexity, max_attachment_area, loading_avatars,
        complexity_mode, occlusion_culling, max_particles, texture_budget_auto, texture_budget_mb, anisotropy,
        shadow_quality, shadows, projector_shadows, ssao, time_of_day, env_manual_transition, env_persist,
        exposure, sharpen, glow, water_reflections, mirrors, ssr, reflection_probes, probe_count, antialiasing,
        tonemapper, dof, vsync, fps_cap, fps_limit, background_fps_cap, background_fps_limit,
    ],
    1 => [
        language, language_is_public, update_mode, ui_scale, use_pie_menu, snap_windows, show_perf,
        show_minimap, show_bug_report, place_bar_stretch, place_bar_rights_text, standalone_place_details, loading_backdrop, reduce_motion, confirm_teleports,
        confirm_no_transfer_settings, confirm_object_bonus, region_check_texture_heights, region_restart,
        confirm_payments, payment_confirm_threshold,
        name_tag_distance, use_display_names, show_usernames, legacy_username_format, trim_resident,
        display_name_notices, tag_complexity, tag_complexity_too_complex_only, tag_complexity_own,
        inventory.sort, inventory.show_library, inventory.show_recent, inventory.show_worn,
        inventory.show_favorites, inventory.search_trash, inventory.search_library, inventory.search_outfits,
        inventory.separate_searches, inventory.double_click_add_objects, inventory.double_click_add_clothes,
        inventory.warn_task_move, inventory.warn_scripted_task_move,
    ],
    2 => [font, font_scale],
    TAB_CHAT => [
        chat_timestamps, chat_toast_seconds, toast_friends_online, presence_position, toast_im, toast_group_im, im_toast_seconds,
        warn_external_links, show_chat, log_im, log_nearby,
        log_show_history, log_dated_names, autoresponse, mute_all_groups, mute_groups_without_notices,
        report_blocks,
    ],
    4 => [
        mouse_sensitivity, invert_mouse, arrows_strafe, double_click, lookat.head_yaw, lookat.head_pitch,
        lookat.static_eyes,
    ],
    5 => [colors],
    6 => [bandwidth_kbps, cache_size_mb],
    TAB_AUDIO => [audio, media],
    TAB_SHORTCUTS => [keybinds],
    9 => [debug],
    TAB_CAMERA => [camera],
    TAB_COMMANDS => [chat_commands],
    TAB_PRIVACY => [
        lookat.show_targets, lookat.show_names, lookat.hide_own, lookat.lines, lookat.private,
        lookat.private_local, lookat.limit_distance, lookat.max_distance,
    ],
    TAB_AURORA => [
        chat_style, classic_name_tags, lookat.style, notifications_style, classic_bars, world_map_style, classic_people, classic_profiles, photo_window,
        classic_about_land, classic_region_estate, classic_pay_window, classic_controls, classic_restart_alert,
        lookat.aim, plates_hover_expand, plates_friends_only_full, plate_click,
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The body of `fn name` in `src`, braces included.
    fn body<'a>(src: &'a str, name: &str) -> &'a str {
        let start = src.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("fn {name} not found"));
        let open = start + src[start..].find('{').expect("body");
        let mut depth = 0;
        for (i, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[open..=open + i];
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced fn {name}")
    }

    /// The arms of the `match st.tab` of `content`, by tab (`_` is the
    /// network page, the last one).
    fn arms(content: &str) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        let starts: Vec<(usize, usize)> = content
            .match_indices("\n        ")
            .filter_map(|(i, _)| {
                let line = &content[i + 9..];
                let head = line.split(" => {").next()?;
                let tab = if head == "_" { 6 } else { head.parse().ok()? };
                line.contains(" => {").then_some((tab, i))
            })
            // (a Windows checkout ends its lines with \r\n)
            .filter(|(_, i)| content[i + 9..].split('\n').next().is_some_and(|l| l.trim_end().ends_with(" => {")))
            .collect();
        for (tab, i) in starts {
            let open = i + content[i..].find('{').expect("arm");
            let mut depth = 0;
            for (j, c) in content[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            out.push((tab, &content[open..=open + j]));
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// The settings a piece of page code reads or writes: `var.field`, and
    /// `var.field.sub` when `sub` is not a method call.
    fn accessed(code: &str, var: &str, prefix: &str) -> BTreeSet<String> {
        let ident = |s: &str| -> String { s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect() };
        let mut out = BTreeSet::new();
        let needle = format!("{var}.");
        for (i, _) in code.match_indices(&needle) {
            if code[..i]
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            {
                continue;
            }
            let rest = &code[i + needle.len()..];
            let first = ident(rest);
            if first.is_empty() {
                continue;
            }
            let after = &rest[first.len()..];
            let mut path = format!("{prefix}{first}");
            if let Some(more) = after.strip_prefix('.') {
                let second = ident(more);
                if !second.is_empty() && !more[second.len()..].starts_with('(') {
                    path = format!("{path}.{second}");
                }
            }
            out.insert(path);
        }
        out
    }

    /// What each page shows, read from its code.
    fn shown() -> Vec<(usize, BTreeSet<String>)> {
        let options = include_str!("../options.rs");
        let mut pages: Vec<(usize, BTreeSet<String>)> = (0..super::super::tab_count()).map(|t| (t, BTreeSet::new())).collect();
        for (tab, code) in arms(body(options, "content")) {
            pages[tab].1.extend(accessed(code, "s", ""));
        }
        pages[TAB_CHAT].1.extend(accessed(body(options, "chat_page_head"), "s", ""));
        pages[TAB_CHAT].1.extend(accessed(body(options, "chat_logs_group"), "s", ""));
        // the inventory block of the Interface page edits `prefs`
        let inventory = include_str!("../inventory/controls.rs");
        pages[1].1.remove("inventory");
        pages[1].1.extend(accessed(body(inventory, "preferences"), "prefs", "inventory."));
        pages[TAB_AUDIO].1.extend(accessed(body(options, "audio_page"), "s", ""));
        pages[TAB_SHORTCUTS].1.extend(accessed(body(options, "keys_page"), "s", ""));
        pages[9].1.extend(accessed(body(options, "debug_page"), "s", ""));
        pages[TAB_CAMERA].1.extend(accessed(include_str!("camera.rs"), "s", ""));
        pages[TAB_COMMANDS].1.insert("chat_commands".into());
        pages[TAB_PRIVACY].1.extend(accessed(include_str!("privacy.rs"), "s", "lookat."));
        pages[TAB_AURORA]
            .1
            .extend(accessed(body(include_str!("aurora.rs"), "page"), "s", ""));
        pages
    }

    fn covered(listed: &[String], path: &str) -> bool {
        listed
            .iter()
            .any(|l| path == l || path.starts_with(&format!("{l}.")) || l.starts_with(&format!("{path}.")))
    }

    #[test]
    fn every_setting_a_page_shows_is_reset_with_its_section() {
        let listed = listed();
        for (tab, paths) in shown() {
            let list = &listed[tab].1;
            assert!(!paths.is_empty(), "page {tab}: nothing found in its code");
            for path in paths {
                // the audio page's push-to-talk keys are reset by hand above
                if tab == TAB_AUDIO && path == "keybinds" {
                    continue;
                }
                assert!(covered(list, &path), "page {tab} shows `{path}`, missing from its list");
            }
        }
    }

    #[test]
    fn every_listed_setting_is_shown_by_its_page() {
        let shown = shown();
        for (tab, list) in listed() {
            for path in &list {
                let paths = &shown[tab].1;
                let found = paths.iter().any(|p| covered(std::slice::from_ref(path), p));
                // the shadow switch follows the quality slider of the page
                assert!(found || path == "shadows", "page {tab} lists `{path}` but does not show it");
            }
        }
    }

    #[test]
    fn a_section_reset_leaves_the_other_pages_alone() {
        let mut s = Settings {
            draw_distance: 300.0,
            shadow_quality: 0,
            shadows: false,
            ui_scale: 1.4,
            ..Settings::default()
        };
        s.lookat.style = crate::world::lookat::Style::Legacy;
        s.lookat.private = true;
        s.lookat.head_yaw = 10.0;
        s.username = "kept".into();
        reset(0, &mut s);
        let d = Settings::default();
        assert_eq!(s.draw_distance, d.draw_distance);
        assert_eq!((s.shadow_quality, s.shadows), (d.shadow_quality, d.shadows));
        assert_eq!(s.ui_scale, 1.4);
        assert_eq!(s.lookat.style, crate::world::lookat::Style::Legacy);
        reset(TAB_PRIVACY, &mut s);
        assert_eq!(s.lookat.private, d.lookat.private);
        assert_eq!(s.lookat.style, crate::world::lookat::Style::Legacy);
        assert_eq!(s.lookat.head_yaw, 10.0);
        reset(TAB_AURORA, &mut s);
        assert_eq!(s.lookat.style, d.lookat.style);
        assert_eq!(s.lookat.head_yaw, 10.0);
        assert_eq!(s.username, "kept");
    }

    #[test]
    fn the_audio_section_puts_push_to_talk_back_only() {
        use crate::keybinds::{Action, Binding, Input};
        let mut s = Settings::default();
        let other = Binding::captured(Input::key(winit::keyboard::KeyCode::F9), Default::default());
        s.keybinds.set(Action::PushToTalk, 0, Some(other.clone()));
        s.keybinds.set(Action::Fly, 0, Some(other));
        reset(TAB_AUDIO, &mut s);
        let d = Settings::default();
        assert_eq!(s.keybinds.get(Action::PushToTalk), d.keybinds.get(Action::PushToTalk));
        assert_ne!(s.keybinds.get(Action::Fly), d.keybinds.get(Action::Fly));
    }
}
