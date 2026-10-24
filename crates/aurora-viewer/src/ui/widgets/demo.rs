//! Offline pointer gestures on the real inventory title bar and rows.

pub fn rez_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_REZ").ok()).as_deref()
}

/// Record the real row; scripted tests drag this widget, not a fabricated cargo.
pub fn rez_row(response: &egui::Response, item: uuid::Uuid) {
    let Some(mode) = rez_mode() else { return };
    let wanted = uuid::Uuid::from_u128(if mode == "nocopy" { 8109 } else { 8100 });
    if item == wanted {
        response
            .ctx
            .data_mut(|d| d.insert_temp(egui::Id::unique("demo_rez_row"), response.rect.center()));
    }
}

pub fn rez_events(ctx: &egui::Context, frame: u64, destination: egui::Pos2) -> Vec<egui::Event> {
    let Some(mode) = rez_mode().filter(|mode| *mode != "1") else {
        return Vec::new();
    };
    let modifiers = egui::Modifiers {
        ctrl: mode == "contents",
        ..Default::default()
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    };
    // Keep the world target clear of the user's persisted demo window layout,
    // using its real title bar rather than changing production window defaults.
    let window = egui::Id::unique("inventory");
    let moving = window.with("demo_rez_move");
    if frame == 640
        && let Some(title) = ctx.read_response(window.with("title_double_click"))
        && let Some(rect) = ctx.memory(|m| m.area_rect(window))
    {
        let from = title.rect.left_center() + egui::vec2(20.0, 0.0);
        let to = from + egui::vec2(20.0 - rect.left(), 140.0 - rect.top());
        ctx.data_mut(|d| d.insert_temp(moving, (from, to)));
        return vec![egui::Event::PointerMoved(from), button(from, true)];
    }
    if let Some((_, to)) = ctx.data(|d| d.get_temp::<(egui::Pos2, egui::Pos2)>(moving)) {
        if frame == 641 {
            return vec![egui::Event::PointerMoved(to)];
        }
        if frame == 642 {
            return vec![button(to, false)];
        }
    }
    let Some(source) = ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("demo_rez_row"))) else {
        return Vec::new();
    };
    match frame {
        680 => vec![egui::Event::PointerMoved(source), button(source, true)],
        681 => vec![egui::Event::PointerMoved(source + egui::vec2(-15.0, 0.0))],
        682..=685 => vec![egui::Event::PointerMoved(destination)],
        686 if mode == "cancel" => vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        686 => vec![button(destination, false)],
        687 if mode == "cancel" => vec![button(destination, false)],
        _ => Vec::new(),
    }
}

/// `AURORA_DEMO_WORN=inventory|library|sound`: the inventory rows it
/// double-clicks, (first frame, item) (`demo::worn::double_clicks`).
fn worn_clicks() -> &'static [(u64, uuid::Uuid)] {
    static CLICKS: std::sync::OnceLock<Vec<(u64, uuid::Uuid)>> = std::sync::OnceLock::new();
    CLICKS.get_or_init(|| {
        std::env::var("AURORA_DEMO_WORN")
            .map(|mode| crate::demo::worn::double_clicks(&mode))
            .unwrap_or_default()
    })
}

/// Record the real row of an item `worn_events` double-clicks.
pub fn worn_row(response: &egui::Response, item: uuid::Uuid) {
    if worn_clicks().iter().any(|(_, wanted)| *wanted == item) {
        response
            .ctx
            .data_mut(|d| d.insert_temp(egui::Id::unique(("demo_worn_row", item)), response.rect.center()));
    }
}

/// Two clicks on the real row of each item of `worn_clicks`, over its
/// first four frames: press, release, press, release.
pub fn worn_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Some(&(start, item)) = worn_clicks().iter().find(|(start, _)| (*start..*start + 4).contains(&frame)) else {
        return Vec::new();
    };
    let Some(pos) = ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique(("demo_worn_row", item)))) else {
        return Vec::new();
    };
    if frame == start {
        log::info!("demo worn inventory: double-click on the item row at frame {frame}");
    }
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: (frame - start).is_multiple_of(2),
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

pub fn events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_WINDOWS") else {
        return Vec::new();
    };
    let id = egui::Id::unique("inventory").with("title_double_click");
    let Some(title) = ctx.read_response(id) else {
        return Vec::new();
    };
    let pos = title.rect.center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    if let Some(spot) = mode.strip_prefix("reach-") {
        return reach_events(ctx, frame, id.with("demo_reach"), pos, spot);
    }
    if matches!(frame, 660 | 664) || mode != "1" && matches!(frame, 1000 | 1004) {
        return vec![egui::Event::PointerMoved(pos), button(pos, true)];
    }
    if matches!(frame, 661 | 665) || mode != "1" && matches!(frame, 1001 | 1005) {
        return vec![button(pos, false)];
    }
    if mode == "drag" {
        let drag_id = id.with("demo_drag");
        if frame == 780 {
            ctx.data_mut(|d| d.insert_temp(drag_id, pos));
            return vec![egui::Event::PointerMoved(pos), button(pos, true)];
        }
        if let Some(start) = ctx.data_mut(|d| d.get_temp::<egui::Pos2>(drag_id)) {
            let target = start + egui::vec2(100.0, 60.0);
            if frame == 781 {
                return vec![egui::Event::PointerMoved(target)];
            }
            if frame == 782 {
                return vec![button(target, false)];
            }
        }
    }
    Vec::new()
}

/// The middle of the widest gap between the bars' islands of the top row
/// (`top`) or of the bottom one, at the height of the row.
fn island_gap(ctx: &egui::Context, top: bool) -> Option<egui::Pos2> {
    let screen = ctx.content_rect();
    let names = ["menu", "bug", "place", "system", "account", "bell", "chat", "dock"];
    let mut row: Vec<egui::Rect> = names
        .iter()
        .filter_map(|name| ctx.memory(|m| m.area_rect(egui::Id::unique(("bars_island", *name)))))
        .filter(|r| (r.center().y < screen.center().y) == top)
        .collect();
    row.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let y = row.first()?.center().y;
    row.windows(2)
        .map(|w| (w[1].left() - w[0].right(), (w[0].right() + w[1].left()) / 2.0))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, x)| egui::pos2(x, y))
}

/// `AURORA_DEMO_WINDOWS=reach-top|reach-bottom|reach-right|reach-left|reach-down`:
/// the real title bar of the inventory is dragged at frames 700–709 into
/// the widest gap between the top islands, between the bottom ones, or far
/// past the right, left or bottom edge of the screen, and released there.
fn reach_events(ctx: &egui::Context, frame: u64, id: egui::Id, pos: egui::Pos2, spot: &str) -> Vec<egui::Event> {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    if frame == 700 {
        let target = match spot {
            "top" => island_gap(ctx, true),
            "bottom" => island_gap(ctx, false),
            "left" => Some(egui::pos2(ctx.content_rect().left() - 2000.0, pos.y)),
            "down" => Some(egui::pos2(pos.x, ctx.content_rect().bottom() + 2000.0)),
            _ => Some(egui::pos2(ctx.content_rect().right() + 2000.0, pos.y)),
        };
        let Some(target) = target else {
            return Vec::new();
        };
        ctx.data_mut(|d| d.insert_temp(id, (pos, target)));
        return vec![egui::Event::PointerMoved(pos), button(pos, true)];
    }
    let Some((start, target)) = ctx.data(|d| d.get_temp::<(egui::Pos2, egui::Pos2)>(id)) else {
        return Vec::new();
    };
    match frame {
        701..=708 => vec![egui::Event::PointerMoved(start.lerp(target, (frame - 700) as f32 / 8.0))],
        709 => vec![button(target, false)],
        _ => Vec::new(),
    }
}

/// AURORA_DEMO_MINIMIZED=1|expand: the real title bar of Conversations is
/// double-clicked at frames 600–605, the window is minimized while the demo
/// chat goes on (its bar lights up, the toolbar button keeps its dot);
/// `expand` then clicks the real Conversations toolbar button at frames
/// 900–901, which expands the window instead of closing it (#269).
pub fn minimized_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_MINIMIZED") else {
        return Vec::new();
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let title = egui::Id::unique("conversations").with("title_double_click");
    let at = |id: egui::Id| ctx.read_response(id).map(|r| r.rect.left_center() + egui::vec2(30.0, 0.0));
    match frame {
        600 | 604 => at(title).map_or(Vec::new(), |pos| vec![egui::Event::PointerMoved(pos), button(pos, true)]),
        601 | 605 => at(title).map_or(Vec::new(), |pos| vec![button(pos, false)]),
        900 | 901 if mode == "expand" => {
            let Some(pos) = ctx
                .read_response(crate::ui::bars::tool_button_id("conversations"))
                .map(|r| r.rect.center())
            else {
                return Vec::new();
            };
            vec![egui::Event::PointerMoved(pos), button(pos, frame == 900)]
        }
        _ => Vec::new(),
    }
}

/// AURORA_DEMO_RESIZE=corner|edge|drag|sweep|sweep-h|sweep-fast[,<window>]: the resize
/// marks of a real window (`conversations` by default; the id of another
/// floater, such as `about_land_aurora`, or `minimap`) from frame 600: the
/// pointer rests in its bottom-right corner (`corner`) or on its right edge
/// (`edge`), or presses that corner and holds it 40 x 30 further out
/// (`drag`) or drags it in then out over the frames (`sweep` the width,
/// `sweep-h` the height: `sweep_offset`; `sweep-fast` the width, in and
/// out quickly: `fast_offset`; `sweep-right` / `sweep-bottom` the same as
/// `sweep` / `sweep-h` by the right or bottom edge), released at frame 1000.
pub fn resize_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Ok(value) = std::env::var("AURORA_DEMO_RESIZE") else {
        return Vec::new();
    };
    if frame < 600 {
        return Vec::new();
    }
    let (mode, window) = value.split_once(',').unwrap_or((value.as_str(), "conversations"));
    let Some(rect) = ctx.memory(|m| m.area_rect(egui::Id::unique(window.trim()))) else {
        return Vec::new();
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    // inside the corner's and the edge's grab areas (`resize::Edge::hit_rect`)
    let corner = rect.max - egui::vec2(6.0, 6.0);
    let (right, bottom) = (
        egui::pos2(rect.right() - 1.0, rect.center().y),
        egui::pos2(rect.center().x, rect.bottom() - 1.0),
    );
    match mode.trim() {
        "corner" => vec![egui::Event::PointerMoved(corner)],
        "edge" => vec![egui::Event::PointerMoved(right)],
        "drag" => {
            let start = egui::Id::unique("demo_resize_start");
            let Some(from) = ctx.data(|d| d.get_temp::<egui::Pos2>(start)) else {
                ctx.data_mut(|d| d.insert_temp(start, corner));
                return vec![egui::Event::PointerMoved(corner), button(corner, true)];
            };
            let to = from + egui::vec2(40.0, 30.0);
            match frame {
                ..1000 => vec![egui::Event::PointerMoved(to)],
                1000 => vec![button(to, false)],
                _ => Vec::new(),
            }
        }
        "sweep" | "sweep-h" | "sweep-fast" | "sweep-right" | "sweep-bottom" => {
            let mode = mode.trim();
            let grip = match mode {
                "sweep-right" => right,
                "sweep-bottom" => bottom,
                _ => corner,
            };
            let start = egui::Id::unique("demo_resize_start");
            let Some(from) = ctx.data(|d| d.get_temp::<egui::Pos2>(start)) else {
                ctx.data_mut(|d| d.insert_temp(start, grip));
                return vec![egui::Event::PointerMoved(grip), button(grip, true)];
            };
            let to = from
                + if mode == "sweep-fast" {
                    fast_offset(frame)
                } else {
                    sweep_offset(frame, matches!(mode, "sweep-h" | "sweep-bottom"))
                };
            match frame {
                ..1000 => vec![egui::Event::PointerMoved(to)],
                1000 => vec![button(to, false)],
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// `sweep`: the corner held from frame 600 goes 500 points in (down to the
/// window's minimum) by frame 700, then 900 points out by frame 900, a few
/// points a frame as a hand drags it; `sweep-h` does the same on the height
/// (400 in, then 600 out).
fn sweep_offset(frame: u64, height: bool) -> egui::Vec2 {
    let (inward, outward) = if height { (400.0, 600.0) } else { (500.0, 900.0) };
    let d = match frame {
        ..=600 => 0.0,
        601..=700 => -inward * (frame - 600) as f32 / 100.0,
        701..=900 => -inward + outward * (frame - 700) as f32 / 200.0,
        _ => outward - inward,
    };
    if height { egui::vec2(0.0, d) } else { egui::vec2(d, 0.0) }
}

/// `sweep-fast`: the corner held from frame 600 goes 520 points in and back
/// out in 24 frames (about 45 points a frame, a quick flick), five times,
/// then rests at its start.
fn fast_offset(frame: u64) -> egui::Vec2 {
    let t = frame.saturating_sub(600);
    let d = if t >= 120 {
        0.0
    } else {
        let k = (t % 24) as f32 / 12.0;
        -520.0 * if k <= 1.0 { k } else { 2.0 - k }
    };
    egui::vec2(d, 0.0)
}

/// AURORA_DEMO_SNAP=1|group|size|free|detach: Conversations dragged by its
/// title next to the People window, on the real windows. Frame 700: the
/// title is pressed; until 900 it is held 6 px short of the right side of
/// People and 5 px under the line of its top, with Alt held (the guide
/// shows); 900: released, the window snaps and the two are glued. `group`
/// then drags People by 70 x 50 from frame 1000 to 1100: Conversations
/// follows. `size` drags the bottom edge of Conversations to 4 px from
/// the line of the bottom of People instead: the edge snaps under the
/// pointer. `free` omits Alt and then moves People alone. `detach` attaches
/// first, then drags People with Alt so that Conversations stays behind.
pub fn snap_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_SNAP") else {
        return Vec::new();
    };
    let modifiers = egui::Modifiers {
        alt: (700..=900).contains(&frame) && mode != "free" || (1000..=1100).contains(&frame) && mode == "detach",
        ..Default::default()
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    };
    let (chat, people) = (egui::Id::unique("conversations"), egui::Id::unique("people"));
    let rect = |window: egui::Id| ctx.memory(|m| m.area_rect(window));
    let title = |window: egui::Id| {
        ctx.read_response(window.with("title_double_click"))
            .map(|r| r.rect.left_center() + egui::vec2(60.0, 0.0))
    };
    // (where the pointer is pressed, where it is held)
    let path = egui::Id::unique("demo_snap_path");
    let begin = |from: egui::Pos2, to: egui::Pos2| {
        ctx.data_mut(|d| d.insert_temp(path, (from, to)));
        vec![egui::Event::PointerMoved(from), button(from, true)]
    };
    let held = ctx.data(|d| d.get_temp::<(egui::Pos2, egui::Pos2)>(path));
    let gap = crate::ui::layout::GAP;
    let mut events = match (frame, mode.as_str()) {
        (700, _) => {
            let (Some(from), Some(c), Some(r)) = (title(chat), rect(chat), rect(people)) else {
                return Vec::new();
            };
            begin(from, from + (egui::pos2(r.right() + gap + 6.0, r.top() + 5.0) - c.min))
        }
        (1000, "group" | "groupe" | "free" | "detach") => match title(people) {
            Some(from) => begin(from, from + egui::vec2(70.0, 50.0)),
            None => Vec::new(),
        },
        (1000, "size" | "taille") => {
            let (Some(c), Some(r)) = (rect(chat), rect(people)) else {
                return Vec::new();
            };
            let from = c.center_bottom();
            begin(from, egui::pos2(from.x, r.bottom() + 4.0))
        }
        (701..=899 | 1001..=1099, _) => held.map_or(Vec::new(), |(_, to)| vec![egui::Event::PointerMoved(to)]),
        (900 | 1100, _) => match held {
            Some((_, to)) => {
                ctx.data_mut(|d| d.remove::<(egui::Pos2, egui::Pos2)>(path));
                vec![egui::Event::PointerMoved(to), button(to, false)]
            }
            None => Vec::new(),
        },
        _ => Vec::new(),
    };
    if matches!(frame, 700..=901 | 1000..=1101) {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
    }
    events
}
