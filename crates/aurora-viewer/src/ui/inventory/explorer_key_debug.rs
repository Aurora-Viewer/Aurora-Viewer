//! Opt-in diagnostics for the path from a recognized Explorer accelerator to a tab.
//! Record only shortcut keys, modifier bits and opaque egui layer numbers.
use egui::{Context, Event, Id, Key, LayerId};
use std::sync::{
    OnceLock,
    atomic::{AtomicU32, Ordering},
};

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AURORA_DEBUG_EXPLORER_KEYS").as_deref() == Ok("1"))
}

pub(crate) fn start(
    ctx: &Context,
    key: Key,
    physical: &str,
    mods: [bool; 3],
    explorer: bool,
    stock_claim: bool,
    skipped: bool,
    queued: &[Event],
) {
    static PRESS: AtomicU32 = AtomicU32::new(0);
    let n = PRESS.fetch_add(1, Ordering::Relaxed);
    if n >= 32 {
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(Id::unique("explorer-key-trace"), (frame, n, key)));
    log::info!(
        "explorer-key #{n} frame={frame} native physical={physical} logical={key:?} ctrl/shift/alt={mods:?} explorer={explorer} stock_focus_claim={stock_claim} skipped_egui={skipped}"
    );
    phase(ctx, "native-queue", None, Some(queued));
}

pub(crate) fn phase(ctx: &Context, stage: &str, pane: Option<LayerId>, raw: Option<&[Event]>) {
    if !enabled() {
        return;
    }
    let Some((start, n, key)) = ctx.data(|d| d.get_temp::<(u64, u32, Key)>(Id::unique("explorer-key-trace"))) else {
        return;
    };
    let frame = ctx.cumulative_frame_nr();
    if frame > start + 2 {
        return;
    }
    let event = |events: &[Event]| {
        events.iter().find_map(|e| match e {
            Event::Key {
                key: k,
                pressed: true,
                modifiers: m,
                repeat,
                ..
            } if *k == key => Some([m.ctrl, m.shift, m.alt, m.command, *repeat]),
            _ => None,
        })
    };
    let seen = raw.map_or_else(|| ctx.input(|i| event(&i.events)), event);
    let front = crate::ui::close_key::frontmost_window(ctx);
    let marked = front.and_then(|layer| ctx.data(|d| d.get_temp::<u64>(layer.id.with("organized-keys"))));
    let (focus, modal) = ctx.memory(|m| (m.focused(), m.top_modal_layer().is_some()));
    let other = focus
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| Some(r.layer_id) != front);
    let other_text =
        other && focus.is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some() || crate::ui::scripts::editor_focused(ctx));
    let popup = egui::Popup::is_any_open(ctx);
    let front = front.map(|l| l.id.value());
    let pane = pane.map(|l| l.id.value());
    log::info!(
        "explorer-key #{n} frame={frame} stage={stage} key={key:?} seen_ctrl/shift/alt/cmd/repeat={seen:?} front={front:?} marker_frame={marked:?} pane={pane:?} external_text={other_text} popup={popup} modal={modal}"
    );
}
