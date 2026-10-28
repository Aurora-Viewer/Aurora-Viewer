use super::*;
use crate::ui::bars::{self, BellState, IslandsLook, StatusInfo};
use crate::ui::icons::Icons;
use crate::ui::notifications::{NotifUi, ToastEnv};
use crate::world::notifications::Notifications;
use egui::{Id, LayerId, Pos2};
use uuid::Uuid;

/// Two rectangles share some area (a common edge is not crossing).
fn overlaps(a: Rect, b: Rect) -> bool {
    a.intersect(b).is_positive()
}

const ROOM: Rect = Rect::from_min_max(Pos2::new(8.0, 56.0), Pos2::new(1272.0, 680.0));

fn sizes(n: usize) -> Vec<Vec2> {
    (0..n).map(|i| Vec2::new(200.0 + 10.0 * i as f32, 44.0)).collect()
}

/// At the top the oldest toast stands at the top of the room and the
/// next ones below; at the bottom the oldest stands on its bottom and the
/// next ones above; left, middle and right follow the room's edges.
#[test]
fn toasts_start_from_their_edge_and_stack_away_from_it() {
    let s = sizes(3);
    for at in PresencePosition::ALL {
        let rects: Vec<Rect> = stack(ROOM, at, &s, &[]).into_iter().map(|r| r.expect("fits")).collect();
        for (r, size) in rects.iter().zip(&s) {
            assert_eq!(r.size(), *size, "{at:?}");
            assert!(ROOM.contains_rect(*r), "{at:?} {r:?}");
            match at {
                PresencePosition::TopLeft | PresencePosition::BottomLeft => assert_eq!(r.left(), ROOM.left()),
                PresencePosition::TopRight | PresencePosition::BottomRight => assert_eq!(r.right(), ROOM.right()),
                _ => assert!((r.center().x - ROOM.center().x).abs() <= 0.5, "{at:?} {r:?}"),
            }
        }
        if at.top() {
            assert_eq!(rects[0].top(), ROOM.top(), "{at:?}");
            assert_eq!(rects[1].top(), rects[0].bottom() + GAP, "{at:?}");
            assert_eq!(rects[2].top(), rects[1].bottom() + GAP, "{at:?}");
        } else {
            assert_eq!(rects[0].bottom(), ROOM.bottom(), "{at:?}");
            assert_eq!(rects[1].bottom(), rects[0].top() - GAP, "{at:?}");
            assert_eq!(rects[2].bottom(), rects[1].top() - GAP, "{at:?}");
        }
    }
}

/// The chat lines across the column push the stack above them; beside it
/// (a toast at the right) they change nothing.
#[test]
fn the_chat_lines_shorten_the_room_from_below() {
    let chat = Rect::from_min_max(Pos2::new(8.0, 560.0), Pos2::new(560.0, 674.0));
    let s = sizes(2);
    let left = stack(ROOM, PresencePosition::BottomLeft, &s, &[chat]);
    let first = left[0].expect("fits");
    assert_eq!(first.bottom(), chat.top() - GAP);
    assert!(left.iter().flatten().all(|r| !overlaps(*r, chat)));
    let right = stack(ROOM, PresencePosition::BottomRight, &s, &[chat]);
    assert_eq!(right[0].expect("fits").bottom(), ROOM.bottom());
}

/// A room lower than the stack keeps the newest toasts that fit, never
/// past its edges; one lower than a toast shows none.
#[test]
fn a_low_room_keeps_the_newest_that_fit() {
    let low = Rect::from_min_max(Pos2::new(8.0, 56.0), Pos2::new(1272.0, 160.0));
    for at in PresencePosition::ALL {
        let rects = stack(low, at, &sizes(4), &[]);
        assert_eq!(
            rects.iter().map(Option::is_some).collect::<Vec<_>>(),
            [false, false, true, true],
            "{at:?}"
        );
        assert!(rects.iter().flatten().all(|r| low.contains_rect(*r)), "{at:?}");
    }
    let tiny = Rect::from_min_max(Pos2::new(8.0, 56.0), Pos2::new(1272.0, 90.0));
    assert!(stack(tiny, PresencePosition::TopRight, &sizes(2), &[]).iter().all(Option::is_none));
}

/// While busy a new toast waits; it shows when the place is free if it
/// arrived less than `PRESENCE` before, else it is dropped; a shown one
/// leaves when the place gets busy.
#[test]
fn a_toast_waits_for_the_place_then_shows_or_is_dropped() {
    let toast = |id, arrived| Toast { id, arrived, shown: None };
    let mut t = vec![toast(1, 0.0), toast(2, 0.0)];
    advance(&mut t, 0.0, true);
    assert!(t.iter().all(|t| t.shown.is_none()));
    advance(&mut t, PRESENCE - 0.5, true);
    assert_eq!(t.len(), 2, "still waiting");
    let free = PRESENCE - 0.4;
    advance(&mut t, free, false);
    assert!(t.iter().all(|t| t.shown == Some(free)), "shown once free");
    // a whole PRESENCE from the moment it shows, then the fade
    advance(&mut t, free + PRESENCE, false);
    assert_eq!(t.len(), 2);
    advance(&mut t, free + PRESENCE + PRESENCE_FADE + 0.01, false);
    assert!(t.is_empty());
    // waited too long: dropped without ever showing
    let mut t = vec![toast(3, 10.0)];
    advance(&mut t, 10.0 + PRESENCE, true);
    assert!(t.is_empty());
    // shown, then a card unrolls over its place: it leaves
    let mut t = vec![toast(4, 20.0)];
    advance(&mut t, 20.0, false);
    advance(&mut t, 21.0, true);
    assert!(t.is_empty());
}

/// The place is saved; a settings file from before it was added, or one
/// that never changed it, gets the top right.
#[test]
fn the_place_is_saved_and_defaults_to_the_top_right() {
    use crate::settings::Settings;
    assert_eq!(Settings::default().presence_position, PresencePosition::TopRight);
    let old: Settings = serde_json::from_str("{}").expect("empty settings");
    assert_eq!(old.presence_position, PresencePosition::TopRight);
    let s = Settings {
        presence_position: PresencePosition::BottomLeft,
        ..Settings::default()
    };
    let back: Settings = serde_json::from_str(&serde_json::to_string(&s).expect("serialize")).expect("read back");
    assert_eq!(back.presence_position, PresencePosition::BottomLeft);
    // the reset of Preferences › Chat puts it back
    let mut s = back;
    crate::ui::options::reset_page_settings(crate::ui::options::Page::Chat, &mut s);
    assert_eq!(s.presence_position, PresencePosition::TopRight);
}

/// The top bar's state the geometry test draws the bars with.
fn status(classic: bool) -> StatusInfo<'static> {
    StatusInfo {
        location_hidden: false,
        show_bug_report: true,
        show_huds: true,
        hud_zoom: 1.0,
        region: "Aurora Demo",
        parcel: "Aurora Plaza",
        tp_back: None,
        tp_forward: None,
        tp_session: Default::default(),
        maturity: "",
        position: glam::Vec3::new(132.0, 126.0, 33.0),
        fps: 60.0,
        flying: false,
        seated: false,
        ping_ms: 38,
        balance: Some(1000),
        show_balance: true,
        voice: (crate::voice::VoiceLight::Off, String::new()),
        slurl: String::new(),
        status: Default::default(),
        ban_lines: 0,
        parcel_icons: Vec::new(),
        local_env: false,
        env_keys: Default::default(),
        double_click: Default::default(),
        quickjump: false,
        quiet_snapshots: false,
        marketplace: false,
        parcel_landmark: None,
        can_set_home: false,
        release_notes: false,
        region_console: false,
        motd: false,
        grid_help: None,
        classic_bars: classic,
        islands: IslandsLook {
            stretch: true,
            rights_text: false,
        },
        own_name: "Aurora Demo",
        own_pic: None,
        recent: Vec::new(),
        favorites: Box::leak(Box::default()),
        system_island: Default::default(),
        net_stats: false,
        perf: Box::leak(Box::default()),
        net_loss: 0.0,
        graphics_preset: None,
        camera_preset: Default::default(),
        interface_hidden: false,
        interface_keys: String::new(),
        ik_keys: String::new(),
        edit_particles: false,
        health: Default::default(),
        hover_tips: Default::default(),
        world_overlays: Default::default(),
        show_transparency: false,
        conversation_thumbnails: 0,
    }
}

/// What a frame of the viewer drew where the friends' toasts must not go.
struct Frame {
    /// The bars' own rectangles: Firestorm's panels or the islands' areas.
    bars: Vec<Rect>,
    chat: Option<Rect>,
    toasts: Option<Rect>,
    /// The system and bell islands, and the center's panel (its list or
    /// its cards).
    system: Option<Rect>,
    bell: Option<Rect>,
    panel: Option<Rect>,
}

/// What the center shows besides the friends' toasts.
#[derive(Clone, Copy, PartialEq)]
enum Center {
    Nothing,
    /// A card rolled out under the bell.
    Card,
    /// The bell's list open.
    List,
}

/// A few frames of the viewer's chrome as `App` draws it (the bars, the
/// room they leave, the nearby chat lines, the center) with three friends'
/// toasts, in a window `size` large.
fn frames(size: Vec2, classic: bool, at: PresencePosition) -> Frame {
    frames_with(size, classic, at, Center::Nothing)
}

fn frames_with(size: Vec2, classic: bool, at: PresencePosition, center: Center) -> Frame {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let icons = Icons::default();
    let skin = crate::ui::skin::Layout::default();
    let settings = crate::settings::Settings::default();
    let mut panels = crate::ui::Panels::from_settings(&settings);
    let mut audio = settings.audio.clone();
    let mut audio_ui = crate::ui::audio::AudioUi::default();
    let mut toolbar = crate::ui::toolbar::ToolbarUi::default();
    let mut toolbar_settings = crate::ui::toolbar::ToolbarSettings::default();
    let mut chat_ui = crate::ui::chat::ChatUi::default();
    let gestures = crate::gestures::Manager::default();
    let mut notif = NotifUi::default();
    let mut list = Notifications::default();
    let mut history = crate::world::notification_history::History::default();
    let mut emoji = crate::ui::emoji::Emoji::load();
    let pics = std::collections::HashMap::new();
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    // six chat lines, some of them wrapping: the most the chat shows
    for i in 0..6 {
        world.system_message(format!(
            "Line {i} of the nearby chat, long enough to fill the width of the chat lines and wrap onto a second row of text"
        ));
    }
    let st = status(classic);
    if center == Center::Card {
        list.push(
            crate::world::notifications::Kind::Info,
            "Lune paid you L$50",
            "",
            crate::world::notifications::Data::None,
        );
    }
    notif.open = center == Center::List;
    for i in 0..3u128 {
        list.push_presence(Uuid::from_u128(100 + i), format!("Friend {i} is online."), true);
    }
    // the bars' panels: their ids salt the id of the ui they are shown in
    let mut root = Id::NULL;
    for frame in 0..8 {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(0.1 * f64::from(frame)),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            root = ui.scope_id();
            bars::top_bars(
                ui,
                &p,
                &icons,
                &skin,
                &mut panels,
                &st,
                &mut audio,
                &mut audio_ui,
                Default::default(),
                &mut notif,
                BellState {
                    aurora: true,
                    ..Default::default()
                },
            );
            let (_, _, bottom_top, _) = bars::bottom_bar(
                ui,
                &p,
                &icons,
                &skin,
                &mut toolbar,
                &mut toolbar_settings,
                &mut panels,
                &mut chat_ui,
                &gestures,
                0,
                None,
                false,
                false,
                false,
                &mut 0.0,
                &mut bars::MicButton::default(),
                false,
                !classic,
                Some(&mut emoji),
                &bars::favorites::View::default(),
            );
            // the room the bars leave, as App reports it
            let area = crate::ui::layout::bars_room(ui.available_rect_before_wrap(), bottom_top, classic);
            crate::ui::layout::set_work_area(ui.ctx(), area);
            let (mut names, mut wanted) = Default::default();
            crate::ui::chat::toasts(
                ui.ctx(),
                &p,
                &world,
                &pics,
                &mut names,
                &mut wanted,
                bottom_top,
                20.0,
                true,
                !classic,
                !classic,
                false,
                true,
            );
            let mut env = ToastEnv {
                pics: &pics,
                emoji: &mut emoji,
                hidden: false,
                message_seconds: 30.0,
                restart: None,
                presence_at: at,
                classic_bars: classic,
            };
            crate::ui::notification_center::show(ui.ctx(), &p, &icons, &world, &mut list, &mut history, &mut notif, &mut env, false);
        });
        out.textures_delta.clear();
    }
    let visible = |layer: LayerId| ctx.memory(|m| m.areas().visible_last_frame(&layer).then(|| m.area_rect(layer.id)).flatten());
    let mut bars: Vec<Rect> = ["nav_bar", "bottom_bar"]
        .into_iter()
        .filter_map(|id| egui::containers::panel::PanelState::load(&ctx, root.with(id)))
        .map(|s| s.outer_rect)
        .collect();
    for name in ["menu", "bug", "place", "account", "system", "bell", "chat", "dock"] {
        bars.extend(visible(LayerId::new(egui::Order::Middle, Id::unique(("bars_island", name)))));
    }
    Frame {
        bars,
        chat: visible(LayerId::new(egui::Order::Background, Id::unique("chat_toasts"))),
        toasts: visible(LayerId::new(egui::Order::Foreground, Id::unique("notification_presence"))),
        system: visible(LayerId::new(egui::Order::Middle, Id::unique(("bars_island", "system")))),
        bell: visible(LayerId::new(egui::Order::Middle, Id::unique(("bars_island", "bell")))),
        panel: visible(LayerId::new(egui::Order::Foreground, Id::unique("notification_center"))),
    }
}

/// At each of the six places, with both looks of the bars and at the
/// three widths of the mockup (and in a very low window), the friends'
/// toasts stay on the screen and cross neither a bar nor the nearby chat
/// lines.
#[test]
fn the_toasts_never_cross_a_bar_nor_the_chat_lines() {
    let windows = [
        Vec2::new(1280.0, 720.0),
        Vec2::new(1536.0, 864.0),
        Vec2::new(1920.0, 1080.0),
        // the lowest window the viewer opens, and lower still
        Vec2::new(1280.0, 560.0),
        Vec2::new(1280.0, 300.0),
    ];
    for size in windows {
        for classic in [true, false] {
            for at in PresencePosition::ALL {
                let case = format!("{size:?} classic {classic} {at:?}");
                let f = frames(size, classic, at);
                // the bars were drawn, in the look asked
                let expected = if classic { 2 } else { 8 };
                assert!(f.bars.len() >= expected, "{case}: bars {:?}", f.bars);
                let chat = f.chat.unwrap_or_else(|| panic!("{case}: no chat lines"));
                let low = size.y < 400.0;
                let Some(toasts) = f.toasts else {
                    assert!(low, "{case}: no toast shown");
                    continue;
                };
                // three toasts of ~44 px and their gaps
                assert!(size.y < 700.0 || toasts.height() > 3.0 * 40.0, "{case}: {toasts:?}");
                assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(toasts), "{case}: {toasts:?}");
                for bar in &f.bars {
                    assert!(!overlaps(toasts, *bar), "{case}: {toasts:?} over the bar {bar:?}");
                }
                assert!(!overlaps(toasts, chat), "{case}: {toasts:?} over the chat lines {chat:?}");
            }
        }
    }
}

/// Frames of Aurora's center alone over a 1280 × 720 room, to click on the
/// friends' toasts.
struct Clicks {
    ctx: egui::Context,
    list: Notifications,
    notif: NotifUi,
    history: crate::world::notification_history::History,
    emoji: crate::ui::emoji::Emoji,
    world: crate::world::World,
    time: f64,
}

impl Clicks {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            list: Notifications::default(),
            notif: NotifUi::default(),
            history: Default::default(),
            emoji: crate::ui::emoji::Emoji::load(),
            world: crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load())),
            time: 0.0,
        }
    }

    /// One frame with `events`; the answers of the center.
    fn frame(&mut self, at: PresencePosition, events: Vec<egui::Event>) -> Vec<(u64, crate::world::notifications::Response)> {
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let pics = std::collections::HashMap::new();
        self.time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut answers = Vec::new();
        let mut out = self.ctx.run_ui(input, |ui| {
            crate::ui::layout::set_work_area(ui.ctx(), Rect::from_min_max(Pos2::new(8.0, 52.0), Pos2::new(1272.0, 664.0)));
            // the bell island at the top right, as the bars report it
            self.notif.bell = Some(Rect::from_min_size(Pos2::new(1236.0, 8.0), Vec2::splat(36.0)));
            let mut env = ToastEnv {
                pics: &pics,
                emoji: &mut self.emoji,
                hidden: false,
                message_seconds: 30.0,
                restart: None,
                presence_at: at,
                classic_bars: false,
            };
            answers = crate::ui::notification_center::show(
                ui.ctx(),
                &p,
                &icons,
                &self.world,
                &mut self.list,
                &mut self.history,
                &mut self.notif,
                &mut env,
                false,
            )
            .0;
        });
        out.textures_delta.clear();
        answers
    }

    /// The friends' toasts' area, after a few frames to lay them out.
    fn toasts(&mut self, at: PresencePosition) -> Option<Rect> {
        for _ in 0..3 {
            self.frame(at, Vec::new());
        }
        let layer = LayerId::new(egui::Order::Foreground, Id::unique("notification_presence"));
        self.ctx
            .memory(|m| m.areas().visible_last_frame(&layer).then(|| m.area_rect(layer.id)).flatten())
    }

    /// A click at `pos`: what the center answered.
    fn click(&mut self, at: PresencePosition, pos: Pos2) -> Vec<(u64, crate::world::notifications::Response)> {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut answers = self.frame(at, vec![egui::Event::PointerMoved(pos), button(true)]);
        answers.extend(self.frame(at, vec![button(false)]));
        answers
    }
}

/// The center answered only `Response::Open` for `id`: the IM opens.
fn opens_im(answers: &[(u64, crate::world::notifications::Response)], id: u64) -> bool {
    matches!(answers, [(i, crate::world::notifications::Response::Open)] if *i == id)
}

/// At each of the six places a click on a friend's "online" toast opens an
/// IM with them (`Response::Open`, as on main); an "offline" toast is not
/// clickable. At the top right, a toast that waited under the open list
/// keeps that click once it shows.
#[test]
fn a_click_on_an_online_toast_opens_an_im() {
    for at in PresencePosition::ALL {
        let mut c = Clicks::new();
        let id = c.list.push_presence(Uuid::from_u128(2), "Orion is online.", true);
        let rect = c.toasts(at).unwrap_or_else(|| panic!("{at:?}: no toast"));
        assert!(opens_im(&c.click(at, rect.center()), id), "{at:?}");

        let mut c = Clicks::new();
        c.list.push_presence(Uuid::from_u128(3), "Lune is offline.", false);
        let rect = c.toasts(at).unwrap_or_else(|| panic!("{at:?}: no toast"));
        assert!(c.click(at, rect.center()).is_empty(), "{at:?}: an offline toast is not clickable");
    }
    // waiting under the bell's open list, then shown when it closes
    let at = PresencePosition::TopRight;
    let mut c = Clicks::new();
    c.notif.open = true;
    let id = c.list.push_presence(Uuid::from_u128(2), "Orion is online.", true);
    assert!(c.toasts(at).is_none(), "waits while the list is open");
    c.notif.open = false;
    let rect = c.toasts(at).expect("shown once the list is closed");
    assert!(opens_im(&c.click(at, rect.center()), id));
}

/// With the islands, the center's open list, its cards and the friends'
/// toasts at the top right stand exactly under the system and bell islands:
/// from the system island's left edge to the bell island's right edge,
/// within half a pixel, at the three widths of the mockup.
#[test]
fn the_center_lines_up_with_the_system_and_bell_islands() {
    let close = |a: f32, b: f32| (a - b).abs() <= 0.5;
    for size in [Vec2::new(1280.0, 720.0), Vec2::new(1536.0, 864.0), Vec2::new(1920.0, 1080.0)] {
        for center in [Center::List, Center::Card] {
            let f = frames_with(size, false, PresencePosition::TopRight, center);
            let (system, bell) = (f.system.expect("system island"), f.bell.expect("bell island"));
            let panel = f.panel.unwrap_or_else(|| panic!("{size:?}: no panel"));
            assert!(close(panel.left(), system.left()), "{size:?}: panel {panel:?}, system {system:?}");
            assert!(close(panel.right(), bell.right()), "{size:?}: panel {panel:?}, bell {bell:?}");
        }
        let f = frames(size, false, PresencePosition::TopRight);
        let (system, bell) = (f.system.expect("system island"), f.bell.expect("bell island"));
        let toasts = f.toasts.unwrap_or_else(|| panic!("{size:?}: no toast"));
        assert!(
            close(toasts.left(), system.left()),
            "{size:?}: toasts {toasts:?}, system {system:?}"
        );
        assert!(close(toasts.right(), bell.right()), "{size:?}: toasts {toasts:?}, bell {bell:?}");
        // elsewhere they keep their own width
        let f = frames(size, false, PresencePosition::TopLeft);
        let toasts = f.toasts.expect("toasts at the top left");
        assert!(toasts.width() < system.width() + bell.width(), "{size:?}: {toasts:?}");
    }
}
