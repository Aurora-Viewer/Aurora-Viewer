use super::*;
use crate::keybinds::{Action, Input};
use crate::ui::widgets::elide_tests::Frames;
use winit::keyboard::KeyCode;

#[test]
fn opening_preferences_focuses_search_without_taking_it_back() {
    let mut frames = Frames::new();
    let ctx = frames.ctx.clone();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let p = theme.palette();
    let icons = Icons::load(&ctx, None);
    let mut settings = Settings::default();
    let mut st = OptionsUi::default();
    let other_id = egui::Id::unique("other_preferences_field");
    let mut other = String::new();
    let mut frame = |st: &mut OptionsUi, mut open, events| {
        frames.run(events, |ui| {
            ui.add(egui::TextEdit::singleline(&mut other).id(other_id));
            show(ui.ctx(), &p, &icons, &mut settings, st, &mut open);
        });
    };
    for _ in 0..3 {
        frame(&mut st, true, Vec::new());
    }
    frame(&mut st, true, vec![egui::Event::Text("volume".into())]);
    assert_eq!(st.query, "volume");
    let search = ctx.memory(|m| m.focused()).expect("search focused");
    assert_ne!(search, other_id);
    ctx.memory_mut(|m| m.request_focus(other_id));
    frame(&mut st, true, Vec::new());
    frame(&mut st, true, vec![egui::Event::Text("elsewhere".into())]);
    assert!(ctx.memory(|m| m.has_focus(other_id)));
    assert_eq!(st.query, "volume");
    frame(&mut st, false, Vec::new());
    frame(&mut st, true, Vec::new());
    assert!(ctx.memory(|m| m.has_focus(search)), "reopening restores search focus");
    frame(&mut st, true, vec![egui::Event::Text("!".into())]);
    assert_eq!(st.query, "volume!");
    crate::ui::widgets::minimize_floater(&ctx, WINDOW);
    frame(&mut st, true, Vec::new());
    ctx.memory_mut(|m| m.request_focus(other_id));
    frame(&mut st, true, Vec::new());
    crate::ui::widgets::expand_floater(&ctx, WINDOW);
    for _ in 0..3 {
        frame(&mut st, true, Vec::new());
    }
    assert!(ctx.memory(|m| m.has_focus(search)), "expanding restores search focus");
    assert_eq!(other, "elsewhere");
}

/// The settings each page of the window showed before the reorganisation
/// (the lists of its former "Reset this section", page by page): every one
/// must still have a row, or be reset with a page.
const BEFORE: &[&str] = &[
    // Graphics
    "draw_distance",
    "parcel_visibility.mode",
    "parcel_visibility.same_owner",
    "parcel_visibility.same_group",
    "parcel_visibility.parcels",
    "parcel_visibility.always_visible",
    "lod_factor",
    "max_avatars",
    "max_complexity",
    "max_attachment_area",
    "loading_avatars",
    "complexity_mode",
    "avatar_physics",
    "avatar_physics_detail",
    "occlusion_culling",
    "max_particles",
    "texture_budget_auto",
    "texture_budget_mb",
    "anisotropy",
    "shadow_quality",
    "shadows",
    "projector_shadows",
    "contact_shadows",
    "ssao",
    "ao_method",
    "gtao_quality",
    "screen_indirect_light",
    "screen_indirect_intensity",
    "time_of_day",
    "env_manual_transition",
    "env_persist",
    "exposure",
    "sharpen",
    "glow",
    "bloom",
    "bloom_strength",
    "water_reflections",
    "mirrors",
    "ssr",
    "reflection_probes",
    "probe_count",
    "antialiasing",
    "specular_aa",
    "tonemapper",
    "tonemap_mix",
    "dof",
    "vsync",
    "fps_cap",
    "fps_limit",
    "background_fps_cap",
    "background_fps_limit",
    // Interface
    "language",
    "language_is_public",
    "update_mode",
    "ui_scale",
    "use_pie_menu",
    "snap_windows",
    "windows_stop_left",
    "windows_stop_right",
    "windows_stop_bottom",
    "show_perf",
    "show_minimap",
    "show_bug_report",
    "place_bar_stretch",
    "place_bar_rights_text",
    "favorites_access",
    "show_net_stats",
    "show_balance",
    "standalone_place_details",
    "loading_backdrop",
    "reduce_motion",
    "confirm_teleports",
    "confirm_no_transfer_settings",
    "confirm_object_bonus",
    "confirm_return_to_owner",
    "confirm_attachment_drop",
    "confirm_script_modify",
    "region_check_texture_heights",
    "sim_version_alert",
    "script_count_alert",
    "script_count_threshold",
    "region_restart",
    "collisions",
    "confirm_payments",
    "payment_confirm_threshold",
    "name_tag_distance",
    "show_own_name_tag",
    "show_friend_name_tags",
    "show_other_name_tags",
    "temporary_name_tags",
    "name_tag_show_seconds",
    "use_display_names",
    "show_usernames",
    "legacy_username_format",
    "trim_resident",
    "group_titles",
    "display_name_notices",
    "tag_complexity",
    "tag_complexity_too_complex_only",
    "tag_complexity_own",
    "hover_tips.enabled",
    "hover_tips.form",
    "hover_tips.all_objects",
    "hover_tips.land",
    "hover_tips.group_titles",
    "hover_tips.delay",
    "inventory.sort",
    "inventory.received_sort",
    "inventory.show_library",
    "inventory.show_recent",
    "inventory.recent_hours",
    "inventory.show_worn",
    "inventory.show_favorites",
    "inventory.search_trash",
    "inventory.search_library",
    "inventory.search_outfits",
    "inventory.separate_searches",
    "inventory.double_click_add_objects",
    "inventory.double_click_add_clothes",
    "inventory.warn_task_move",
    "inventory.warn_scripted_task_move",
    // Fonts
    "font",
    "font_scale",
    // Chat
    "chat_timestamps",
    "show_chat_channel",
    "compact_automated_chat",
    "compact_im_system_lines",
    "chat_keep_focus",
    "chat_gesture_autocomplete",
    "chat_name_prediction",
    "colon_emote",
    "chat_mention_autocomplete",
    "autohide_chat_bar",
    "chat_toast_seconds",
    "chat_bubbles",
    "chat_bubbles_hide_console",
    "typing_bubbles",
    "chat_bubble_opacity",
    "toast_friends_online",
    "friends_online_in_chat",
    "presence_position",
    "toast_im",
    "toast_group_im",
    "ims_in_notification_center",
    "im_toast_seconds",
    "flash_on_message",
    "flash_on_object_im",
    "flash_on_script_dialog",
    "system_notifications",
    "system_notifications_when_active",
    "warn_external_links",
    "show_chat",
    "show_contacts_tab",
    "send_typing_state",
    "play_typing_anim",
    "type_during_emote",
    "typing_in_name_tags",
    "typing_chevron",
    "flash_unread_conversations",
    "flash_conversations_button",
    "log_im",
    "log_nearby",
    "log_show_history",
    "reopen_ims",
    "log_dated_names",
    "autoresponse",
    "mute_all_groups",
    "mute_groups_without_notices",
    "report_blocks",
    "show_joined_group_invitations",
    "link_previews",
    "keywords",
    "friend_request_prompt",
    "region_message_prompt",
    // Controls
    "mouse_sensitivity",
    "invert_mouse",
    "world_mouse_walk",
    "arrows_strafe",
    "arrow_keys_always_move",
    "letter_keys_move",
    "walk_backwards_without_turning",
    "double_click",
    "lookat.head_yaw",
    "lookat.head_pitch",
    "crouch_toggle",
    "fly_after_teleport",
    "block_click_sit",
    "lookat.static_eyes",
    // Colors, network and cache, sound and voice, shortcuts, camera, commands
    "colors",
    "bandwidth_kbps",
    "download_connections",
    "no_neighbour_regions",
    "cache_size_mb",
    "cache_location",
    "audio",
    "media",
    "keybinds",
    "camera",
    "chat_commands",
    // Privacy
    "lookat.show_targets",
    "lookat.show_names",
    "lookat.hide_own",
    "lookat.lines",
    "lookat.private",
    "lookat.private_local",
    "lookat.private_point_at",
    "lookat.limit_distance",
    "lookat.max_distance",
    "object_im_filter",
    // Aurora
    "water_effects",
    "sort_received_items",
    "received_items_since",
    "chat_style",
    "classic_chat_lines",
    "classic_name_tags",
    "lookat.style",
    "notifications_style",
    "classic_bars",
    "world_map_style",
    "classic_people",
    "classic_profiles",
    "photo_window",
    "snapshot.viewfinder.controls",
    "classic_about_land",
    "classic_search",
    "classic_region_estate",
    "classic_places",
    "classic_inventory",
    "classic_pay_window",
    "legacy_money_tracker",
    "classic_controls",
    "classic_restart_alert",
    "classic_rez_effect",
    "classic_hover_tips",
    "lookat.aim",
    "predict_own_motion",
    "synchronize_animations",
    "ground_avatar_feet",
    "ground_feet_while_moving",
    "ground_feet_max_height",
    "ground_feet_max_avatars",
    "plates_hover_expand",
    "plates_friends_only_full",
    "plates_friend_tint",
    "plate_click",
    "plate_shift_click",
    "plate_ctrl_click",
    // Accessibility
    "color_blindness",
    "color_blindness_strength",
];

/// Every setting the pages edit, with its page (the rows and what the
/// pages reset besides them).
fn collected() -> Vec<(Page, String)> {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut s = Settings::default();
    let d = Settings::default();
    let mut st = OptionsUi::default();
    let mut r = OptionsResult::default();
    let mut out = Vec::new();
    run_ui(&ctx, egui::RawInput::default(), |ui| {
        let mut cx = Cx::new(ui, p, &mut s, &d, &mut st, &mut r, Mode::Collect, "");
        for page in Page::ALL {
            run_page(&mut cx, page, None);
        }
        out = cx.collected.iter().map(|(page, path)| (*page, path.replace(' ', ""))).collect();
    });
    out
}

fn covered(paths: &[(Page, String)], before: &str) -> bool {
    paths
        .iter()
        .any(|(_, p)| p == before || p.starts_with(&format!("{before}.")) || before.starts_with(&format!("{p}.")))
}

#[test]
fn no_setting_of_the_former_pages_is_lost() {
    let paths = collected();
    for before in BEFORE {
        assert!(covered(&paths, before), "`{before}` has no row any more");
    }
}

#[test]
fn rlva_settings_live_on_their_own_page() {
    let paths = collected();
    let rlva = |path: &str| path == "rlva_enabled" || path == "rlva" || path.starts_with("rlva.");
    let on_rlva: Vec<_> = paths
        .iter()
        .filter(|(page, _)| *page == Page::Rlva)
        .map(|(_, p)| p.as_str())
        .collect();
    assert!(on_rlva.contains(&"rlva_enabled"), "the global switch: {on_rlva:?}");
    assert!(on_rlva.iter().filter(|p| p.starts_with("rlva.")).count() >= 20, "{on_rlva:?}");
    assert!(on_rlva.iter().all(|p| rlva(p)), "only RLVa on its page: {on_rlva:?}");
    let elsewhere: Vec<_> = paths.iter().filter(|(page, p)| *page != Page::Rlva && rlva(p)).collect();
    assert!(elsewhere.is_empty(), "RLVa rows left on another page: {elsewhere:?}");
    assert!(Page::Rlva.experimental() && !Page::Protections.experimental());
    let mut s = Settings::default();
    assert_eq!(counts(&mut s, "RLVa").hits(Page::Protections), 0);
    s.rlva_enabled = !s.rlva_enabled;
    s.anti_spam.enabled = !s.anti_spam.enabled;
    reset_page_settings(Page::Protections, &mut s);
    assert_ne!(s.rlva_enabled, Settings::default().rlva_enabled, "Protections reset preserves RLVa");
    reset_page_settings(Page::Rlva, &mut s);
    assert_eq!(s.rlva_enabled, Settings::default().rlva_enabled);
}

#[test]
fn a_setting_has_one_page() {
    let paths = collected();
    for (page, path) in &paths {
        // the push-to-talk keys: Sound and voice and Shortcuts, as before
        if path == "keybinds" {
            continue;
        }
        let elsewhere: Vec<_> = paths
            .iter()
            .filter(|(other, p)| other != page && (p == path || p.starts_with(&format!("{path}.")) || path.starts_with(&format!("{p}."))))
            .collect();
        assert!(elsewhere.is_empty(), "`{path}` of {page:?} is also on {elsewhere:?}");
    }
}

#[test]
fn a_page_reset_leaves_the_other_pages_alone() {
    let mut s = Settings {
        draw_distance: 300.0,
        shadow_quality: 0,
        shadows: false,
        ui_scale: 1.4,
        plate_click: crate::settings::PlateClick::Profile,
        plate_shift_click: crate::settings::PlateClick::InstantMessage,
        plate_ctrl_click: crate::settings::PlateClick::Menu,
        ..Settings::default()
    };
    s.lookat.style = crate::world::lookat::Style::Legacy;
    s.lookat.private = true;
    s.anti_spam.enabled = true;
    s.anti_spam.newlines = 5;
    s.lookat.head_yaw = 10.0;
    s.username = "kept".into();
    s.inventory.sort.by_date = false;
    s.inventory.received_sort.folders_by_name = true;
    reset_page_settings(Page::Graphics, &mut s);
    let d = Settings::default();
    assert_eq!(s.draw_distance, d.draw_distance);
    assert_eq!((s.shadow_quality, s.shadows), (d.shadow_quality, d.shadows));
    assert_eq!(s.ui_scale, 1.4);
    assert_eq!(s.lookat.style, crate::world::lookat::Style::Legacy);
    reset_page_settings(Page::Privacy, &mut s);
    assert_eq!(s.lookat.private, d.lookat.private);
    assert_eq!(s.lookat.style, crate::world::lookat::Style::Legacy);
    assert_eq!(s.lookat.head_yaw, 10.0);
    assert!(s.anti_spam.enabled, "Privacy reset preserves Protections");
    s.lookat.private = true;
    reset_page_settings(Page::Protections, &mut s);
    assert_eq!(s.anti_spam, d.anti_spam);
    assert!(s.lookat.private, "Protections reset preserves Privacy");
    reset_page_settings(Page::Aurora, &mut s);
    assert_eq!(s.lookat.style, d.lookat.style);
    assert_eq!(s.lookat.head_yaw, 10.0);
    assert_eq!(s.username, "kept");
    assert!(!s.inventory.sort.by_date, "other pages keep both inventory orders");
    assert!(s.inventory.received_sort.folders_by_name);
    assert_eq!(s.plate_click, crate::settings::PlateClick::Profile);
    assert_eq!(s.plate_shift_click, crate::settings::PlateClick::InstantMessage);
    assert_eq!(s.plate_ctrl_click, crate::settings::PlateClick::Menu);
    reset_page_settings(Page::Avatars, &mut s);
    assert_eq!(s.plate_click, d.plate_click);
    assert_eq!(s.plate_shift_click, d.plate_shift_click);
    assert_eq!(s.plate_ctrl_click, d.plate_ctrl_click);
    assert_eq!(s.lookat.head_yaw, d.lookat.head_yaw);
    reset_page_settings(Page::Inventory, &mut s);
    assert_eq!(s.inventory.sort, d.inventory.sort);
    assert_eq!(s.inventory.received_sort, d.inventory.received_sort);
    reset_page_settings(Page::Layout, &mut s);
    assert_eq!(s.ui_scale, d.ui_scale);
}

#[test]
fn joystick_preferences_reset_device_mapping_buttons_and_both_tunings() {
    let mut s = Settings::default();
    s.joystick.enabled = true;
    s.joystick.device = "test-device".into();
    s.joystick.gamepad();
    s.joystick.flycam.gain[3] = 0.7;
    s.joystick.avatar.dead_zone[0] = 0.4;
    s.joystick.auto_level = false;
    s.joystick.jump_button = 7;
    s.joystick.rise_button = 12;
    s.joystick.fall_button = 13;
    s.joystick.direct_zoom = true;
    reset_page_settings(Page::Controls, &mut s);
    assert_eq!(s.joystick, crate::joystick::Settings::default());
}

#[test]
fn automatic_flight_preferences_have_separate_controls_and_reset_with_their_page() {
    let paths = collected();
    for key in ["automatic_fly", "automatic_land"] {
        let pages: Vec<_> = paths.iter().filter(|(_, path)| path == key).map(|(page, _)| *page).collect();
        assert_eq!(pages, [Page::Controls]);
    }
    let mut s = Settings {
        automatic_fly: false,
        automatic_land: false,
        fly_after_teleport: true,
        ..Settings::default()
    };
    reset_page_settings(Page::Graphics, &mut s);
    assert!(!s.automatic_fly && !s.automatic_land);
    reset_page_settings(Page::Controls, &mut s);
    assert!(s.automatic_fly && s.automatic_land);
    assert!(!s.fly_after_teleport);
}

#[test]
fn the_sound_page_puts_push_to_talk_back_only() {
    use crate::keybinds::Binding;
    let mut s = Settings::default();
    let other = Binding::captured(Input::key(KeyCode::F9), Default::default());
    s.keybinds.set(Action::PushToTalk, 0, Some(other.clone()));
    s.keybinds.set(Action::Fly, 0, Some(other));
    reset_page_settings(Page::Sound, &mut s);
    let d = Settings::default();
    assert_eq!(s.keybinds.get(Action::PushToTalk), d.keybinds.get(Action::PushToTalk));
    assert_ne!(s.keybinds.get(Action::Fly), d.keybinds.get(Action::Fly));
    reset_page_settings(Page::Shortcuts, &mut s);
    assert_eq!(s.keybinds.get(Action::Fly), d.keybinds.get(Action::Fly));
}

#[test]
fn the_moved_settings_reset_with_their_new_page() {
    let mut s = Settings::default();
    s.water_effects.enabled = true;
    s.water_effects.bubbles = false;
    s.synchronize_animations = true;
    s.ground_avatar_feet = true;
    s.ground_feet_max_height = 1.0;
    s.ground_feet_max_avatars = 50;
    s.sort_received_items = true;
    s.received_items_since = 123;
    s.reduce_motion = true;
    s.audio.lip_sync = false;
    s.audio.media_autoplay = true;
    s.language_is_public = false;
    reset_page_settings(Page::Graphics, &mut s);
    assert!(!s.water_effects.enabled, "Aurora's water is a graphics setting now");
    assert!(s.water_effects.bubbles);
    reset_page_settings(Page::Aurora, &mut s);
    assert!(!s.synchronize_animations);
    reset_page_settings(Page::Avatars, &mut s);
    assert!(!s.ground_avatar_feet);
    assert_eq!(s.ground_feet_max_height, 0.4);
    assert_eq!(s.ground_feet_max_avatars, 5);
    assert!(s.audio.lip_sync);
    reset_page_settings(Page::Inventory, &mut s);
    assert!(!s.sort_received_items);
    assert_eq!(s.received_items_since, 0);
    reset_page_settings(Page::Accessibility, &mut s);
    assert!(!s.reduce_motion);
    reset_page_settings(Page::Media, &mut s);
    assert!(!s.audio.media_autoplay);
    reset_page_settings(Page::Privacy, &mut s);
    assert!(s.language_is_public);
}

/// The counts of every page for a search (or none).
fn counts(s: &mut Settings, query: &str) -> Counts {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let d = Settings::default();
    let mut st = OptionsUi {
        query: query.to_owned(),
        ..Default::default()
    };
    let mut r = OptionsResult::default();
    run_ui(&ctx, egui::RawInput::default(), |ui| count(ui, &p, s, &d, &mut st, &mut r));
    st.counts
}

#[test]
fn the_search_finds_settings_on_their_pages() {
    let mut s = Settings::default();
    let found = counts(&mut s, &cx::fold(aurora_i18n::tr!("ui-options-shadows")));
    assert!(found.hits(Page::Graphics) >= 3, "the shadows' rows: {:?}", found.hits);
    // a command by its name, a shortcut by its key
    assert!(counts(&mut s, "gtp").hits(Page::Commands) >= 1);
    assert!(counts(&mut s, "Ctrl+I").hits(Page::Shortcuts) >= 1);
    assert!(counts(&mut s, "anti-spam").hits(Page::Protections) >= 8);
    assert!(counts(&mut s, "RLVa").hits(Page::Rlva) >= 1);
    assert!(counts(&mut s, "").hits.is_empty());
    assert!(counts(&mut s, "zzzzzz").hits.is_empty());
}

#[test]
fn the_modified_settings_are_counted_on_their_pages() {
    let mut s = Settings::default();
    assert_eq!(counts(&mut s, "").total_modified(), 0, "nothing differs from the defaults");
    s.draw_distance = 300.0;
    s.chat_timestamps = !s.chat_timestamps;
    s.water_effects.caustics = false;
    s.anti_spam.enabled = true;
    let c = counts(&mut s, "");
    assert_eq!(c.modified(Page::Graphics), 2, "a row behind a cog counts too");
    assert_eq!(c.modified(Page::Chat), 1);
    assert_eq!(c.modified(Page::Protections), 1);
    assert_eq!(c.total_modified(), 4);
}

#[test]
fn the_simulator_version_notice_has_a_switch_off_by_default() {
    let mut s = Settings::default();
    assert!(!s.sim_version_alert);
    let query = cx::fold(aurora_i18n::tr!("ui-options-sim-version-alert"));
    assert!(counts(&mut s, &query).hits(Page::Confirmations) >= 1, "found by the search");
    assert_eq!(counts(&mut s, "").modified(Page::Confirmations), 0);
    // turned on: its row is the one that differs from the defaults
    s.sim_version_alert = true;
    assert_eq!(counts(&mut s, "").modified(Page::Confirmations), 1);
}

#[test]
fn the_pages_are_counted_again_only_when_something_may_have_changed() {
    let mut st = OptionsUi::default();
    assert!(count_due(&st, 10.0, false), "never counted");
    st.counted = Some((10.0, String::new()));
    assert!(!count_due(&st, 10.1, false), "nothing happened");
    assert!(count_due(&st, 10.1, true), "a click or a key this frame");
    st.count_stale = true;
    assert!(count_due(&st, 10.1, false), "a click or a change in the last frame");
    st.count_stale = false;
    st.query = "ombre".into();
    assert!(count_due(&st, 10.1, false), "another search");
    st.query.clear();
    assert!(count_due(&st, 10.0 + COUNT_REFRESH, false), "the app's state behind the rows");
    st.reveal = Some((Page::Graphics, 0, "x"));
    assert!(count_due(&st, 10.1, false), "a row asked by its label");
}

#[test]
fn every_page_names_its_tabs() {
    for page in Page::ALL {
        assert!(!page.tabs().is_empty(), "{page:?}");
        assert!(!page.name().is_empty() && !page.about().is_empty(), "{page:?}");
        assert_eq!(Page::parse(page.key()), Some((page, 0)));
    }
    // the numbers of the former pages still open a page (the docs' demos)
    assert_eq!(Page::parse("0"), Some((Page::Graphics, 0)));
    assert_eq!(Page::parse("13"), Some((Page::Aurora, 0)));
    assert_eq!(Page::parse("9"), None, "the former Debug page is the Dev Toolbox");
    assert_eq!(
        Page::Graphics.tabs()[5..],
        [aurora_i18n::tr!("ui-options-smoothness"), aurora_i18n::tr!("region-parcels")]
    );
}

/// One frame of the rows of a page's tab, `width` wide, laid out as in the
/// window; the window's state and result are kept between frames.
fn page_frame(
    frames: &mut Frames,
    s: &mut Settings,
    st: &mut OptionsUi,
    r: &mut OptionsResult,
    page: Page,
    tab: usize,
    events: Vec<egui::Event>,
) -> Vec<crate::ui::widgets::elide_tests::Text> {
    let p = crate::theme::Theme::default().palette();
    let d = Settings::default();
    frames.run(events, |ui| {
        ui.set_width(700.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        count(ui, &p, s, &d, st, r);
        let mut cx = Cx::new(ui, p, s, &d, st, r, Mode::Show, "");
        run_page(&mut cx, page, Some(tab));
        let changed = cx.changed;
        drop(cx);
        r.changed |= changed;
    })
}

fn click(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    vec![
        vec![egui::Event::PointerMoved(at)],
        vec![button(true)],
        vec![button(false)],
        Vec::new(),
    ]
}

#[test]
fn group_titles_are_chosen_in_one_dropdown_and_reset_together() {
    use crate::world::names::GroupTitles;

    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Avatars, 1, Vec::new());
    }
    let paths = collected();
    assert_eq!(paths.iter().filter(|(_, path)| path == "group_titles").count(), 1);
    assert!(!paths.iter().any(|(_, path)| path == "hide_own_group_title"));
    for mode in [GroupTitles::AllExceptMine, GroupTitles::Mine, GroupTitles::None, GroupTitles::All] {
        let at = texts
            .iter()
            .find(|t| t.source == s.group_titles.label())
            .expect("selected title mode")
            .rect
            .center();
        for events in click(at) {
            texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Avatars, 1, events);
        }
        for choice in GroupTitles::ALL {
            assert!(texts.iter().any(|t| t.source == choice.label()), "dropdown offers {choice:?}");
        }
        let at = texts.iter().find(|t| t.source == mode.label()).expect("title choice").rect.center();
        for events in click(at) {
            texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Avatars, 1, events);
        }
        assert_eq!(s.group_titles, mode);
        assert!(r.changed);
    }
    s.group_titles = GroupTitles::AllExceptMine;
    reset_page_settings(Page::Avatars, &mut s);
    assert_eq!(s.group_titles, GroupTitles::All);
}

#[test]
fn keyboard_scale_survives_an_unrelated_preference_change_and_reload() {
    for key in [egui::Key::Plus, egui::Key::Minus] {
        let mut frames = Frames::new();
        let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
        s.snap_windows = !s.snap_windows;
        let expected = if key == egui::Key::Plus { 1.1 } else { 0.9 };
        let events = [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            })
            .collect();
        frames.run(events, |ui| {
            assert!(scale_keyboard(ui.ctx(), &mut s.ui_scale));
        });
        frames.ctx.set_zoom_factor(s.ui_scale);
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Layout, 0, Vec::new());
        }
        let label = texts
            .iter()
            .find(|t| t.source == aurora_i18n::tr!("ui-options-snap-windows"))
            .expect("the snap windows preference")
            .rect;
        // Reset a real unrelated preference through its modified-setting dot.
        for events in click(egui::pos2(label.left() - 8.0, label.center().y)) {
            page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Layout, 0, events);
        }
        assert!(r.changed);
        assert_eq!(s.snap_windows, Settings::default().snap_windows);
        // App::frame reapplies the saved scale when any preference changes.
        crate::theme::Theme::default().apply(&frames.ctx, s.font_scale);
        frames.ctx.set_zoom_factor(s.ui_scale);
        page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Layout, 0, Vec::new());
        assert_eq!(s.ui_scale, expected);
        assert_eq!(frames.ctx.zoom_factor(), expected);
        let saved = serde_json::to_string(&s).expect("serialize settings");
        let restored: Settings = serde_json::from_str(&saved).expect("reload settings");
        assert_eq!(restored.ui_scale, expected);
    }
}

fn preference_shapes(
    frames: &mut Frames,
    s: &mut Settings,
    st: &mut OptionsUi,
    r: &mut OptionsResult,
    view: (Page, usize, f32),
    events: Vec<egui::Event>,
) -> Vec<egui::Shape> {
    let p = crate::theme::Theme::default().palette();
    let d = Settings::default();
    frames.shapes(events, |ui| {
        ui.set_width(view.2);
        ui.spacing_mut().item_spacing.y = 0.0;
        count(ui, &p, s, &d, st, r);
        let mut cx = Cx::new(ui, p, s, &d, st, r, Mode::Show, "");
        run_page(&mut cx, view.0, Some(view.1));
    })
}

#[test]
fn graphics_profile_save_has_a_button_face_to_the_right_of_the_list() {
    for width in [420.0, 700.0, 900.0] {
        let mut frames = Frames::new();
        let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
        s.graphics_saved
            .insert("Layout profile".into(), crate::graphics_presets::saved::Saved::capture(&s));
        crate::graphics_presets::saved::select(&mut s, "Layout profile");
        let mut shapes = Vec::new();
        for _ in 0..4 {
            shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, (Page::Graphics, 0, width), Vec::new());
        }
        let text_rect = |label: &str| {
            shapes
                .iter()
                .find_map(|shape| match shape {
                    egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing text: {label}"))
        };
        let list = text_rect("Layout profile");
        let save = text_rect(aurora_i18n::tr!("common-save"));
        let save_as = text_rect(aurora_i18n::tr!("graphics-saved-save-as"));
        assert!(save.left() > list.right(), "{width}: Save is right of the list");
        assert!(
            (save.center().y - list.center().y).abs() < 1.0,
            "{width}: Save stays on the list's line"
        );
        assert!(
            save_as.left() > save.right() && save_as.right() <= width,
            "{width}: Save as fits beside Save"
        );
        assert!(
            shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(face)
                if face.rect.contains_rect(save.expand(1.0)) && face.rect.height() < 30.0 && face.rect.width() < 160.0
            )),
            "{width}: Save has an ordinary padded button face"
        );
    }
}

#[test]
fn parcel_visibility_lists_show_full_names_and_regions_with_separate_remove_controls() {
    use crate::world::parcel_visibility::Entry;
    fn frame(
        frames: &mut Frames,
        s: &mut Settings,
        st: &mut OptionsUi,
        view: (f32, Mode),
        events: Vec<egui::Event>,
    ) -> (Vec<egui::Shape>, bool) {
        let p = crate::theme::Theme::default().palette();
        let d = Settings::default();
        let mut r = OptionsResult::default();
        let mut changed = false;
        let shapes = frames.shapes(events, |ui| {
            ui.set_width(view.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut cx = Cx::new(ui, p, s, &d, st, &mut r, view.1, "parc");
            run_page(&mut cx, Page::Graphics, Some(6));
            changed |= cx.changed;
        });
        (shapes, changed)
    }
    fn text<'a>(shapes: &'a [egui::Shape], source: &str) -> &'a egui::epaint::TextShape {
        shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(t) if t.galley.text() == source => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing parcel text: {source}"))
    }
    for width in [420.0, 700.0, 900.0] {
        for mode in [Mode::Show, Mode::Search] {
            let mut frames = Frames::new();
            let (mut s, mut st) = (Settings::default(), OptionsUi::default());
            let entries: Vec<_> = (1..=4)
                .map(|i| Entry {
                    region: uuid::Uuid::from_u128(i as u128),
                    local_id: i,
                    name: format!("Northern Aurora Gardens — Observatory, visitor centre and exhibition pavilion {i}"),
                    region_name: format!("Aurora Demonstration Region {i}"),
                })
                .collect();
            s.parcel_visibility.parcels = entries[..2].to_vec();
            s.parcel_visibility.always_visible = entries[2..].to_vec();
            let view = (width, mode);
            let mut shapes = Vec::new();
            for _ in 0..4 {
                shapes = frame(&mut frames, &mut s, &mut st, view, Vec::new()).0;
            }
            let heading_label = aurora_i18n::tr!("parcel-visibility-always-visible");
            let heading = if mode == Mode::Show {
                let filtering = text(&shapes, &aurora_i18n::tr!("parcel-visibility-filter").to_uppercase()).pos;
                let scope = text(&shapes, &aurora_i18n::tr!("parcel-visibility-parcels").to_uppercase()).pos;
                let exceptions = text(&shapes, &heading_label.to_uppercase()).pos;
                assert!(filtering.y < scope.y && scope.y < exceptions.y, "three separate sections in order");
                exceptions + egui::vec2(14.0, 0.0)
            } else {
                text(&shapes, heading_label).pos
            };
            let mut remove = None;
            let mut right: Option<f32> = None;
            for entry in &entries {
                let name = text(&shapes, &entry.name);
                let region = text(&shapes, &entry.region_name);
                assert!(
                    !name.galley.elided && !region.galley.elided,
                    "{width}, {mode:?}: names and regions must be readable"
                );
                assert!((name.pos.x - heading.x).abs() < 1.0, "a list uses the width below its heading");
                assert!(region.pos.y >= name.pos.y + name.galley.size().y, "the region has its own line");
                let trash = shapes
                    .iter()
                    .filter_map(|shape| match shape {
                        egui::Shape::Rect(r)
                            if (r.rect.width() - 20.0).abs() < 0.1
                                && (r.rect.height() - 20.0).abs() < 0.1
                                && (r.rect.top() - name.pos.y).abs() < 1.0
                                && r.rect.left() > name.pos.x + name.galley.size().x =>
                        {
                            Some(r.rect)
                        }
                        _ => None,
                    })
                    .max_by(|a, b| a.right().total_cmp(&b.right()))
                    .expect("a separate remove control to the right of the name");
                assert!(
                    trash.right() - heading.x > width * 0.8,
                    "{width}, {mode:?}: the list spans the page"
                );
                if let Some(right) = right {
                    assert!((trash.right() - right).abs() < 1.0, "remove controls share a right edge");
                }
                right = Some(trash.right());
                if entry == &entries[2] {
                    remove = Some(trash.center());
                }
            }
            let mut changed = false;
            for events in click(remove.expect("remove the first always-visible entry")) {
                changed |= frame(&mut frames, &mut s, &mut st, view, events).1;
            }
            assert!(changed, "removing a parcel must save the preference");
            assert_eq!(s.parcel_visibility.always_visible, entries[3..]);
            assert_eq!(s.parcel_visibility.parcels, entries[..2], "removal belongs to its own list");
        }
    }
}

/// The full painted rails, excluding the shorter accent fills and handles.
fn slider_rails(frames: &Frames, shapes: &[egui::Shape]) -> Vec<egui::Rect> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(r)
                if [
                    (r.rect.width() - SLIDER_W).abs(),
                    (r.rect.width() - (SLIDER_W - 30.0)).abs(),
                    (r.rect.width() - (SLIDER_W + 40.0)).abs(),
                ]
                .into_iter()
                .any(|delta| delta < 1.0)
                    && r.rect.height() < 12.0
                    && r.fill == frames.ctx.style_of(frames.ctx.theme()).visuals.widgets.inactive.bg_fill =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn preference_slider_rails_stay_in_place_when_values_change() {
    for width in [420.0, 700.0, 900.0] {
        for (page, tab) in [
            (Page::Chat, 4),
            (Page::Graphics, 0),
            (Page::Graphics, 5),
            (Page::Network, 0),
            (Page::Sound, 0),
        ] {
            let mut frames = Frames::new();
            let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
            s.fps_cap = true;
            s.background_fps_cap = true;
            let view = (page, tab, width);
            let mut shapes = Vec::new();
            for _ in 0..3 {
                shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, Vec::new());
            }
            let rails = slider_rails(&frames, &shapes);
            assert!(!rails.is_empty(), "{page:?}/{tab} paints sliders");
            for minutes in [1, 20, 59, 0] {
                s.autoresponse.away_after_minutes = minutes;
                s.autoresponse.quit_after_away_minutes = minutes;
                s.draw_distance = if minutes == 1 { 16.0 } else { 512.0 };
                s.lod_factor = if minutes == 1 { 0.5 } else { 4.0 };
                s.max_particles = if minutes == 1 { 1 } else { 8000 };
                s.loading_avatars = if minutes == 1 { 0 } else { 2 };
                s.fps_limit = if minutes == 1 { 9 } else { 144 };
                s.background_fps_limit = if minutes == 1 { 9 } else { 100 };
                s.bandwidth_kbps = if minutes == 1 { 50 } else { 20000 };
                s.cache_size_mb = if minutes == 1 { 512 } else { 32768 };
                s.audio.volume.fill(minutes as f32 / 60.0);
                for _ in 0..3 {
                    shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, Vec::new());
                    assert_eq!(
                        slider_rails(&frames, &shapes),
                        rails,
                        "{page:?}/{tab}, {width} points, value {minutes}"
                    );
                    for rail in &rails {
                        for shape in &shapes {
                            if let egui::Shape::Text(t) = shape {
                                let text = egui::Rect::from_min_size(t.pos, t.galley.size());
                                assert!(!text.intersects(*rail), "{page:?}/{tab}: text overlaps a rail at {width} points");
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_volume_sliders_line_up() {
    // the master has no enable box: its slider used to start one box
    // further right than the channels' (#463)
    for font in [crate::ui::fonts::INTER, crate::ui::fonts::OPEN_DYSLEXIC] {
        for scale in [1.0, 1.25] {
            for perceptual in [false, true] {
                let mut frames = Frames::new();
                crate::ui::fonts::apply(&frames.ctx, font);
                frames.ctx.set_zoom_factor(scale);
                let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
                s.audio.perceptual_volume = perceptual;
                let view = (Page::Sound, 0, 700.0);
                let mut shapes = Vec::new();
                for _ in 0..3 {
                    shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, Vec::new());
                }
                let rails: Vec<egui::Rect> = slider_rails(&frames, &shapes)
                    .into_iter()
                    .filter(|rail| (rail.width() - (SLIDER_W + 40.0)).abs() < 1.0)
                    .collect();
                let case = format!("{font}, scale {scale}, perceptual {perceptual}");
                assert_eq!(rails.len(), crate::settings::audio_channels().len(), "{case}: one rail per channel");
                for rail in &rails[1..] {
                    assert!((rail.left() - rails[0].left()).abs() <= 0.5, "{case}: {rails:?}");
                    assert!((rail.right() - rails[0].right()).abs() <= 0.5, "{case}: {rail:?} vs {:?}", rails[0]);
                }
            }
        }
    }
}

#[test]
fn away_slider_rails_stay_in_place_during_typing_and_pointer_changes() {
    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    let view = (Page::Chat, 4, 900.0);
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, Vec::new());
    }
    let rails = slider_rails(&frames, &shapes);
    assert_eq!(rails.len(), 2);
    for (index, rail) in rails.iter().enumerate() {
        let field = shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(t)
                    if t.galley.text() == aurora_i18n::tr!("ui-options-away-disabled")
                        && (t.pos.y + t.galley.size().y / 2.0 - rail.center().y).abs() < 8.0 =>
                {
                    Some(t.pos + t.galley.size() / 2.0)
                }
                _ => None,
            })
            .expect("the disabled value beside this rail");
        for events in click(field).into_iter().chain([vec![egui::Event::Text("20".to_owned())]]) {
            shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, events);
            assert_eq!(slider_rails(&frames, &shapes), rails, "typing must not move either rail");
        }
        let value = |s: &Settings| match index {
            0 => s.autoresponse.away_after_minutes,
            _ => s.autoresponse.quit_after_away_minutes,
        };
        assert_eq!(value(&s), 20, "the number is editable");
        for at in [rail.center(), egui::pos2(rail.left() + 1.0, rail.center().y)] {
            for events in click(at) {
                shapes = preference_shapes(&mut frames, &mut s, &mut st, &mut r, view, events);
                assert_eq!(slider_rails(&frames, &shapes), rails, "pointer changes must not move either rail");
            }
        }
        assert_eq!(value(&s), 0, "the pointer can restore Disabled");
    }
}

#[test]
fn the_conversations_choice_switches_the_style() {
    use crate::settings::ChatStyle;
    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Aurora, 0, Vec::new());
    }
    for (label, style) in [
        (aurora_i18n::tr!("ui-options-aurora-compact"), ChatStyle::Compact),
        (aurora_i18n::tr!("ui-options-aurora-bubbles"), ChatStyle::Bubbles),
        (aurora_i18n::tr!("ui-options-aurora-lines"), ChatStyle::Legacy),
        (aurora_i18n::tr!("ui-options-aurora-workspace"), ChatStyle::Aurora),
    ] {
        let at = texts
            .iter()
            .find(|t| t.source == label)
            .unwrap_or_else(|| panic!("{label} is painted"))
            .rect
            .center();
        for events in click(at) {
            texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Aurora, 0, events);
        }
        assert_eq!(s.chat_style, style, "{label}");
        assert!(r.changed, "{label} is saved");
    }
}

#[test]
fn the_dot_of_a_modified_row_puts_it_back() {
    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    s.chat_timestamps = !s.chat_timestamps;
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Chat, 0, Vec::new());
    }
    let label = texts
        .iter()
        .find(|t| t.source == aurora_i18n::tr!("ui-options-show-time"))
        .expect("the row of the time")
        .rect;
    // the dot sits in the gutter left of the label
    let dot = egui::pos2(label.left() - 8.0, label.center().y);
    for events in click(dot) {
        page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Chat, 0, events);
    }
    assert_eq!(s.chat_timestamps, Settings::default().chat_timestamps);
    assert!(r.changed);
}

#[test]
fn the_cog_shows_the_rows_behind_it() {
    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    let detail = aurora_i18n::tr!("ui-options-avatar-physics-detail");
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Graphics, 0, Vec::new());
    }
    assert!(!texts.iter().any(|t| t.source == detail), "behind its cog at first");
    // the cog shows the number of its rows, right of the label
    let row = texts
        .iter()
        .find(|t| t.source == aurora_i18n::tr!("ui-options-avatar-physics"))
        .expect("the physics row")
        .rect;
    let cog = texts
        .iter()
        .find(|t| t.source == "1" && (t.rect.center().y - row.center().y).abs() < 6.0 && t.rect.left() > row.right())
        .expect("its cog")
        .rect;
    for events in click(cog.center() - egui::vec2(6.0, 0.0)) {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Graphics, 0, events);
    }
    for _ in 0..2 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Graphics, 0, Vec::new());
    }
    assert!(texts.iter().any(|t| t.source == detail), "shown once the cog is open");
}

#[test]
fn the_frame_rate_limits_show_under_their_switches() {
    // no cog to open: the limits are what their switches are turned on for
    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Graphics, 5, Vec::new());
    }
    for (switch, limit) in [
        (
            aurora_i18n::tr!("extra-limit-frame-rate"),
            aurora_i18n::tr!("ui-options-frame-limit"),
        ),
        (
            aurora_i18n::tr!("ui-options-limit-when-unfocused"),
            aurora_i18n::tr!("ui-options-frame-limit-unfocused"),
        ),
    ] {
        let switch = texts.iter().find(|t| t.source == switch).expect("the switch").rect;
        // in English the unfocused limit has its switch's name: look below it
        assert!(
            texts.iter().any(|t| t.source == limit && t.rect.top() > switch.bottom()),
            "its limit, shown at once under its switch"
        );
        assert!(
            !texts
                .iter()
                .any(|t| t.source == "1" && (t.rect.center().y - switch.center().y).abs() < 6.0),
            "no cog on the switch"
        );
    }
}

#[test]
fn the_name_colors_wait_for_their_switch() {
    // the cog of the colours by relation is dimmed and its rows greyed
    // while the switch is off
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let d = Settings::default();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    assert!(s.colors.relation_names && !s.classic_name_tags);
    s.colors.relation_names = false;
    let id = egui::Id::unique((
        "preferences-row",
        Page::Avatars,
        aurora_i18n::tr!("ui-options-name-tag-color"),
        "",
        aurora_i18n::tr!("ui-options-names-by-relation"),
    ));
    run_ui(&ctx, egui::RawInput::default(), |ui| count(ui, &p, &mut s, &d, &mut st, &mut r));
    assert_eq!(st.subs.get(&id), Some(&(4, true)), "four colours, greyed");
    s.colors.relation_names = true;
    run_ui(&ctx, egui::RawInput::default(), |ui| count(ui, &p, &mut s, &d, &mut st, &mut r));
    assert_eq!(st.subs.get(&id), Some(&(4, false)));
}

#[test]
fn preferences_keep_their_height_when_text_size_changes() {
    let theme = crate::theme::Theme::default();
    let p = theme.palette();
    for small in [false, true] {
        let ctx = egui::Context::default();
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::INTER);
        theme.apply(&ctx, 1.0);
        let mut settings = Settings::default();
        let mut state = OptionsUi {
            page: Page::Appearance,
            system_fonts: Some(Vec::new()),
            ..Default::default()
        };
        let mut open = true;
        let mut frame = 0;
        let mut draw = |ctx: &egui::Context, state: &mut OptionsUi, settings: &mut Settings| {
            frame += 1;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |_| {
                    show(ctx, &p, &Icons::default(), settings, state, &mut open);
                },
            );
            output.textures_delta.clear();
            ctx.memory(|memory| memory.area_rect(egui::Id::unique(WINDOW)))
                .expect("preferences")
        };
        for _ in 0..12 {
            draw(&ctx, &mut state, &mut settings);
        }
        if small {
            crate::ui::widgets::size_floater(&ctx, WINDOW, Vec2::new(780.0, 440.0));
            for _ in 0..12 {
                draw(&ctx, &mut state, &mut settings);
            }
        }
        let initial = draw(&ctx, &mut state, &mut settings);
        for (page, query) in [
            (Page::Appearance, ""),
            (Page::Shortcuts, ""),
            (Page::Aurora, ""),
            (Page::Graphics, "ombre"),
        ] {
            state.page = page;
            state.query = query.to_owned();
            for scale in [1.0, 1.2, 1.4, 2.0, 0.8, 1.0] {
                settings.font_scale = scale;
                theme.apply(&ctx, scale);
                for _ in 0..30 {
                    let rect = draw(&ctx, &mut state, &mut settings);
                    assert!(
                        (rect.height() - initial.height()).abs() < 0.5 && (rect.width() - initial.width()).abs() < 0.5,
                        "small={small} page={page:?} scale={scale}: {initial:?} -> {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_content_ratings_follow_the_account() {
    use aurora_net::search::{AgentAccess, SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};
    let ratings = |max| {
        maturity_choices(AgentAccess { max, preferred: max })
            .into_iter()
            .map(|(m, _)| m)
            .collect::<Vec<_>>()
    };
    assert_eq!(ratings(SIM_ACCESS_ADULT), [SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG]);
    // not adult: no "Adult"; below Moderate: no choice at all
    assert_eq!(ratings(SIM_ACCESS_MATURE), [SIM_ACCESS_MATURE, SIM_ACCESS_PG]);
    assert!(ratings(SIM_ACCESS_PG).is_empty());
}

#[test]
fn hud_modifier_alone_waits_for_release_but_a_combination_captures_its_key() {
    let mut ui = OptionsUi {
        capture: Some(Capture::Binding(Action::HudDrag, 1)),
        ..Default::default()
    };
    assert_eq!(ui.capture_key(KeyCode::ControlRight, true, false), None);
    assert_eq!(
        ui.capture_key(KeyCode::ControlRight, false, false),
        Some(Some(Input::key(KeyCode::ControlRight)))
    );
    ui.take_capture();
    ui.capture = Some(Capture::Binding(Action::HudDrag, 0));
    assert_eq!(ui.capture_key(KeyCode::ControlLeft, true, false), None);
    assert_eq!(ui.capture_key(KeyCode::KeyB, true, false), Some(Some(Input::key(KeyCode::KeyB))));
    ui.take_capture();
    assert_eq!(ui.capture_key(KeyCode::ControlLeft, false, false), None);
}

#[test]
fn ordinary_capture_still_ignores_modifier_keys_and_escape_cancels() {
    let mut ui = OptionsUi {
        capture: Some(Capture::Binding(Action::Inventory, 0)),
        ..Default::default()
    };
    assert_eq!(ui.capture_key(KeyCode::ControlLeft, true, false), None);
    assert_eq!(ui.capture_key(KeyCode::ControlLeft, false, false), None);
    assert_eq!(ui.capture_key(KeyCode::KeyI, true, true), None);
    assert_eq!(ui.capture_key(KeyCode::KeyI, true, false), Some(Some(Input::key(KeyCode::KeyI))));
    assert_eq!(ui.capture_key(KeyCode::Escape, true, false), Some(None));
}

#[test]
fn the_demo_opens_the_page_asked() {
    let mut st = OptionsUi::default();
    demo(&mut st, "chat/2");
    assert_eq!((st.page, st.tab), (Page::Chat, 2));
    demo(&mut st, "0:tonemap");
    assert_eq!(st.page, Page::Graphics);
    assert!(st.reveal.is_some_and(|(page, tab, _)| page == Page::Graphics && tab == 4));
    demo(&mut st, "graphics:search=ombre");
    assert_eq!(st.query, "ombre");
    demo(&mut st, "7:voice");
    assert_eq!((st.page, st.tab), (Page::Sound, 2));
}

/// The confirmation of a restore, with `untick` clicked first, then its
/// "Restore" button: the settings in the end, and what the app is asked.
fn confirm_restore(s: &mut Settings, untick: Option<&str>) -> Option<backup::Restored> {
    use crate::settings::backup::Backup;
    let other = Settings {
        draw_distance: 300.0,
        window_width: 640,
        ..Settings::default()
    };
    let backup = Backup::parse(&Backup::new(&other, Some("layout".into())).to_bytes().unwrap()).unwrap();
    let mut st = OptionsUi {
        restore: Some(backup::Pending::new(backup, None)),
        ..Default::default()
    };
    let mut r = OptionsResult::default();
    let p = crate::theme::Theme::default().palette();
    let mut frames = Frames::new();
    let mut frame = |s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult, events| {
        frames.run(events, |ui| backup::restore_modal(ui.ctx(), &p, s, st, r, true))
    };
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = frame(s, &mut st, &mut r, Vec::new());
    }
    for label in untick.into_iter().chain([aurora_i18n::tr!("ui-options-backup-restore-confirm")]) {
        let at = texts
            .iter()
            .find(|t| t.source == label)
            .unwrap_or_else(|| panic!("{label} is painted"))
            .rect
            .center();
        for events in click(at) {
            texts = frame(s, &mut st, &mut r, events);
        }
    }
    assert!(st.restore.is_none(), "the confirmation closes");
    r.restored
}

#[test]
fn a_restore_confirmed_takes_the_backup_and_keeps_this_pc() {
    let mut s = Settings {
        window_width: 1234,
        ..Default::default()
    };
    let restored = confirm_restore(&mut s, None).expect("the app is asked for the rest");
    assert_eq!(s.draw_distance, 300.0);
    assert_eq!(s.window_width, 1234, "this PC's window");
    assert!(restored.parts.settings && restored.parts.layout);
}

#[test]
fn a_part_unticked_is_not_restored() {
    let mut s = Settings::default();
    let restored = confirm_restore(&mut s, Some(aurora_i18n::tr!("ui-options-backup-part-settings"))).expect("the layout is");
    assert_eq!(s.draw_distance, Settings::default().draw_distance);
    assert!(!restored.parts.settings && restored.parts.layout);
}

/// Graphics › Fluidity › Graphics card (#894): the startup notice shows
/// under the group's title, in the user's words, only when there is one.
#[test]
fn the_graphics_card_group_shows_the_startup_notice() {
    use aurora_render::adapter_choice::Notice;
    let fluidity = 5;
    let no_discrete = aurora_i18n::tr!("ui-options-graphics-card-no-discrete").to_string();
    let cannot_present = aurora_i18n::tr!("ui-options-graphics-card-cannot-present").to_string();
    let group = aurora_i18n::tr!("ui-options-graphics-card-choice").to_string();
    for (notice, shown) in [
        (Some(Notice::NoDiscrete), Some(&no_discrete)),
        (Some(Notice::DiscreteCannotPresent), Some(&cannot_present)),
        (None, None),
    ] {
        let mut frames = Frames::new();
        let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
        st.gpus = vec!["Intel(R) UHD Graphics".into(), "NVIDIA GeForce RTX 3060".into()];
        st.gpu_in_use = "Intel(R) UHD Graphics".into();
        st.gpu_notice = notice;
        let mut texts = Vec::new();
        for _ in 0..2 {
            texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Graphics, fluidity, Vec::new());
        }
        let shows = |text: &str| texts.iter().any(|t| t.source == text);
        assert!(shows(&group), "the card choice is on the Fluidity tab");
        for text in [&no_discrete, &cannot_present] {
            assert_eq!(shows(text), shown == Some(text), "{text} with {notice:?}");
        }
    }
}

/// Accessibility gathers rows of other pages: each one is drawn there, and
/// only there besides its own page (the search and the counts keep it on
/// its page).
#[test]
fn the_accessibility_page_shows_the_settings_of_other_pages() {
    use aurora_i18n::tr;
    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Accessibility, 0, Vec::new());
    }
    let shows = |key: &str| texts.iter().any(|t| t.source == key);
    for key in [
        tr!("ui-options-interface-scale"),
        tr!("ui-options-text-size"),
        tr!("ui-options-bundled-fonts"),
        tr!("ui-options-high-contrast"),
        tr!("ui-options-name-distance"),
        tr!("ui-options-accent-color"),
        tr!("ui-options-surface-color"),
        tr!("ui-options-text-color"),
        tr!("ui-options-color-vision"),
        tr!("ui-options-chat-bubble-duration"),
        tr!("ui-options-toast-duration"),
        tr!("ui-options-hover-tips-show"),
        tr!("ui-options-hover-tips-delay"),
        tr!("inventory-hover-pictures"),
        tr!("ui-options-reduce-motion"),
        tr!("ui-options-flash-unread"),
        tr!("ui-options-dof-enable"),
        tr!("ui-options-perceptual-volume"),
        tr!("ui-options-interface-sounds"),
        tr!("voice-show-visualizers"),
        tr!("ui-options-video-subtitles"),
    ] {
        assert!(shows(key), "{key} on Accessibility");
    }
    // the other rows of those pages stay on them
    for key in [
        tr!("ui-options-system-fonts"),
        tr!("ui-options-show-time"),
        tr!("ui-options-dof-edit"),
        tr!("voice-show-self"),
        tr!("ui-options-all-interface-colors"),
    ] {
        assert!(!shows(key), "{key} stays on its page");
    }

    // a setting keeps one page for the search and the modified counts
    s.font_scale = 1.2;
    s.audio.perceptual_volume = !s.audio.perceptual_volume;
    let c = counts(&mut s, "");
    assert_eq!(c.modified(Page::Accessibility), 0);
    assert_eq!(c.modified(Page::Appearance), 1);
    assert_eq!(c.modified(Page::Sound), 1);
    let found = counts(&mut s, &cx::fold(tr!("ui-options-text-size")));
    assert_eq!(found.hits(Page::Accessibility), 0);
    assert!(found.hits(Page::Appearance) >= 1);
}

/// A row shown on Accessibility edits its page's setting: its modified dot
/// puts that setting back, and the page's reset leaves it to its page.
#[test]
fn a_borrowed_row_edits_the_setting_of_its_page() {
    let mut frames = Frames::new();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    s.flash_unread_conversations = !s.flash_unread_conversations;
    s.audio.perceptual_volume = !s.audio.perceptual_volume;
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Accessibility, 0, Vec::new());
    }
    for key in [
        aurora_i18n::tr!("ui-options-flash-unread"),
        aurora_i18n::tr!("ui-options-perceptual-volume"),
    ] {
        let label = texts
            .iter()
            .find(|t| t.source == key)
            .unwrap_or_else(|| panic!("the row {key}"))
            .rect;
        for events in click(egui::pos2(label.left() - 8.0, label.center().y)) {
            texts = page_frame(&mut frames, &mut s, &mut st, &mut r, Page::Accessibility, 0, events);
        }
    }
    let d = Settings::default();
    assert_eq!(s.flash_unread_conversations, d.flash_unread_conversations);
    assert_eq!(s.audio.perceptual_volume, d.audio.perceptual_volume);
    assert!(r.changed);

    // the Accessibility reset puts back its own rows only
    s.font_scale = 1.2;
    s.reduce_motion = true;
    reset_page_settings(Page::Accessibility, &mut s);
    assert!(!s.reduce_motion);
    assert_eq!(s.font_scale, 1.2, "the text size is reset with Appearance");
}
