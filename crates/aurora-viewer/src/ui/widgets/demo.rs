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
            .data_mut(|d| d.insert_temp(egui::Id::new("demo_rez_row"), response.rect.center()));
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
    let window = egui::Id::new("inventory");
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
    let Some(source) = ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::new("demo_rez_row"))) else {
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

pub fn events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_WINDOWS") else {
        return Vec::new();
    };
    let id = egui::Id::new("inventory").with("title_double_click");
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

/// AURORA_DEMO_SNAP=1|groupe|taille: the Conversations window dragged by its
/// title next to the People window, on the real windows. Frame 700: the
/// title is pressed; until 900 it is held 6 px short of the right side of
/// People and 5 px under the line of its top (the guide and the Alt hint
/// show); 900: released, the window snaps and the two are glued. `groupe`
/// then drags People by 70 x 50 from frame 1000 to 1100: Conversations
/// follows. `taille` drags the bottom edge of Conversations to 4 px from
/// the line of the bottom of People instead: the edge snaps under the
/// pointer.
pub fn snap_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_SNAP") else {
        return Vec::new();
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (chat, people) = (egui::Id::new("conversations"), egui::Id::new("people"));
    let rect = |window: egui::Id| ctx.memory(|m| m.area_rect(window));
    let title = |window: egui::Id| {
        ctx.read_response(window.with("title_double_click"))
            .map(|r| r.rect.left_center() + egui::vec2(60.0, 0.0))
    };
    // (where the pointer is pressed, where it is held)
    let path = egui::Id::new("demo_snap_path");
    let begin = |from: egui::Pos2, to: egui::Pos2| {
        ctx.data_mut(|d| d.insert_temp(path, (from, to)));
        vec![egui::Event::PointerMoved(from), button(from, true)]
    };
    let held = ctx.data(|d| d.get_temp::<(egui::Pos2, egui::Pos2)>(path));
    let gap = crate::ui::layout::GAP;
    match (frame, mode.as_str()) {
        (700, _) => {
            let (Some(from), Some(c), Some(r)) = (title(chat), rect(chat), rect(people)) else {
                return Vec::new();
            };
            begin(from, from + (egui::pos2(r.right() + gap + 6.0, r.top() + 5.0) - c.min))
        }
        (1000, "groupe") => match title(people) {
            Some(from) => begin(from, from + egui::vec2(70.0, 50.0)),
            None => Vec::new(),
        },
        (1000, "taille") => {
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
    }
}
