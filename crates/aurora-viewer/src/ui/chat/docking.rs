//! Title-bar drops extend LLFloater::onClickTearOff (indra/llui/llfloater.cpp)
//! with the same reattachment path. Only a real primary-button title drag
//! can dock; moving/resizing content or clicking an overlapping title cannot.

use crate::theme::Palette;

fn target_id() -> egui::Id {
    egui::Id::unique("conversation_docking_target")
}

/// Publish only this pass's expanded, visible container. Contacts draws
/// after chat, so a hidden/closed container can never retain a stale target.
pub(super) fn target(ctx: &egui::Context, visible: bool) {
    let rect = visible
        .then(|| ctx.memory(|m| m.area_rect(egui::Id::unique("conversations"))))
        .flatten();
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(target_id(), (pass, rect)));
}

/// Called before drawing the source, while its title's response still has
/// the geometry at pointer-down, even for a fast drag in a single frame.
pub(crate) fn title_drop(ctx: &egui::Context, p: &Palette, source: &str) -> bool {
    let window = egui::Id::unique(source);
    let layer = egui::LayerId::new(egui::Order::Middle, window);
    let armed_id = window.with("conversation_title_grab");
    let (pressed, down, released, escape) = ctx.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if pressed {
        let armed = ctx.input(|i| i.pointer.press_origin()).is_some_and(|pos| {
            ctx.layer_id_at(pos) == Some(layer)
                && ctx
                    .read_response(window.with("title_double_click"))
                    .is_some_and(|r| r.rect.contains(pos))
        });
        ctx.data_mut(|d| d.insert_temp(armed_id, armed));
    }
    let armed = ctx.data(|d| d.get_temp::<bool>(armed_id)).unwrap_or(false) && !escape;
    if !down || escape {
        ctx.data_mut(|d| d.remove::<bool>(armed_id));
    }
    let moving = ctx.dragged_id() == Some(window.with("move"));
    let stopped = ctx.drag_stopped_id() == Some(window.with("move"));
    if !armed || !(moving || (released && stopped)) {
        return false;
    }
    let target = ctx.data(|d| d.get_temp::<(u64, Option<egui::Rect>)>(target_id()));
    let Some((_, Some(rect))) = target.filter(|(pass, _)| *pass == ctx.cumulative_pass_nr()) else {
        return false;
    };
    let Some(pos) = ctx.pointer_interact_pos().filter(|pos| rect.contains(*pos)) else {
        return false;
    };
    let host = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("conversations"));
    if ctx.layer_id_at(pos).is_some_and(|top| top != layer && top != host) {
        return false;
    }
    crate::ui::widgets::attention_mark(
        &ctx.layer_painter(host),
        p,
        rect,
        ctx.global_style().visuals.window_corner_radius,
        0.08,
    );
    released && stopped
}

/// Native offline captures of the actual title gesture, using the existing
/// detached-IM/group fixture. `dock-hold` leaves the target highlighted.
pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let Some(mode) = MODE.get_or_init(|| std::env::var("AURORA_DEMO_CONV_POPOUT").ok()).as_deref() else {
        return Vec::new();
    };
    if !matches!(mode, "dock" | "dock-hold") || !(700..=741).contains(&frame) {
        return Vec::new();
    }
    let gesture = egui::Id::unique("conversation_demo_docking");
    if frame == 700 {
        let source = egui::Id::unique(super::popout::floater_id(Some(crate::demo::DEMO_FRIEND)));
        let Some(title) = ctx.read_response(source.with("title_double_click")) else {
            return Vec::new();
        };
        let Some(host) = ctx.memory(|m| m.area_rect(egui::Id::unique("conversations"))) else {
            return Vec::new();
        };
        ctx.data_mut(|d| d.insert_temp(gesture, (title.rect.center(), host.center())));
    }
    let Some((from, to)) = ctx.data(|d| d.get_temp::<(egui::Pos2, egui::Pos2)>(gesture)) else {
        return Vec::new();
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    match frame {
        700 => vec![egui::Event::PointerMoved(from)],
        701 => vec![button(from, true)],
        702..=740 => vec![egui::Event::PointerMoved(to)],
        741 if mode == "dock" => vec![button(to, false)],
        _ => Vec::new(),
    }
}
