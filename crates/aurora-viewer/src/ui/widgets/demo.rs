//! Offline pointer gestures on the real inventory title bar.

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
