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
