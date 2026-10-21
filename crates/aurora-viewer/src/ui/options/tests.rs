use super::*;
use crate::keybinds::{Action, Input};
use crate::ui::widgets::elide_tests::Frames;
use winit::keyboard::KeyCode;

/// The settings each page of the window showed before the reorganisation
/// (the lists of its former "Reset this section", page by page): every one
/// must still have a row, or be reset with a page.
const BEFORE: &[&str] = &[
    // Graphics
    "draw_distance",
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
    "hide_own_group_title",
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
    "chat_keep_focus",
    "chat_toast_seconds",
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
    "keywords",
    "friend_request_prompt",
    // Controls
    "mouse_sensitivity",
    "invert_mouse",
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
    "plates_hover_expand",
    "plates_friends_only_full",
    "plate_click",
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
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
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
        ..Settings::default()
    };
    s.lookat.style = crate::world::lookat::Style::Legacy;
    s.lookat.private = true;
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
    reset_page_settings(Page::Aurora, &mut s);
    assert_eq!(s.lookat.style, d.lookat.style);
    assert_eq!(s.lookat.head_yaw, 10.0);
    assert_eq!(s.username, "kept");
    assert!(!s.inventory.sort.by_date, "other pages keep both inventory orders");
    assert!(s.inventory.received_sort.folders_by_name);
    reset_page_settings(Page::Avatars, &mut s);
    assert_eq!(s.lookat.head_yaw, d.lookat.head_yaw);
    reset_page_settings(Page::Inventory, &mut s);
    assert_eq!(s.inventory.sort, d.inventory.sort);
    assert_eq!(s.inventory.received_sort, d.inventory.received_sort);
    reset_page_settings(Page::Layout, &mut s);
    assert_eq!(s.ui_scale, d.ui_scale);
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
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| count(ui, &p, s, &d, &mut st, &mut r));
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
    let c = counts(&mut s, "");
    assert_eq!(c.modified(Page::Graphics), 2, "a row behind a cog counts too");
    assert_eq!(c.modified(Page::Chat), 1);
    assert_eq!(c.total_modified(), 3);
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
fn the_name_colors_wait_for_their_switch() {
    // the cog of the colours by relation is dimmed and its rows greyed
    // while the switch is off
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let d = Settings::default();
    let (mut s, mut st, mut r) = (Settings::default(), OptionsUi::default(), OptionsResult::default());
    assert!(!s.colors.relation_names && !s.classic_name_tags);
    let id = egui::Id::new((
        "preferences-row",
        Page::Avatars,
        aurora_i18n::tr!("ui-options-name-tag-color"),
        "",
        aurora_i18n::tr!("ui-options-names-by-relation"),
    ));
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| count(ui, &p, &mut s, &d, &mut st, &mut r));
    assert_eq!(st.subs.get(&id), Some(&(4, true)), "four colours, greyed");
    s.colors.relation_names = true;
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| count(ui, &p, &mut s, &d, &mut st, &mut r));
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
            ctx.memory(|memory| memory.area_rect(egui::Id::new(WINDOW))).expect("preferences")
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
            for scale in [1.0, 1.2, 1.4, 0.8, 1.0] {
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
