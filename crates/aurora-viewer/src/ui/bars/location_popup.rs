//! The SLURL button opens its own anchored field. An integrated navigation
//! control keeps its independent placement and visibility (LLLocationInputCtrl).

use super::{BarAction, StatusInfo};
use crate::{
    theme::Palette,
    ui::{
        Panels,
        toolbar::{Bar, ToolbarUi},
        widgets,
    },
};
use egui::{Id, Rect, Vec2};

#[derive(Clone, Copy)]
struct Anchor {
    rect: Rect,
    bar: Bar,
}

#[derive(Clone)]
struct State {
    anchor: Option<Anchor>,
    text: Option<String>,
    fresh: bool,
}

fn state_id() -> Id {
    Id::unique("toolbar_slurl_popup")
}
fn anchor_id() -> Id {
    Id::unique("toolbar_slurl_anchor")
}
pub(crate) fn field_id() -> Id {
    Id::unique("toolbar_slurl_popup_field")
}

pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Option<State>>(state_id())).flatten().is_some()
}

fn close(ctx: &egui::Context) {
    let was_open = is_open(ctx);
    ctx.data_mut(|data| data.insert_temp(state_id(), None::<State>));
    ctx.memory_mut(|memory| memory.surrender_focus(field_id()));
    if was_open {
        ctx.request_repaint();
    }
}

pub(crate) fn prepare(ctx: &egui::Context, editing: bool) {
    if editing {
        close(ctx);
    }
    ctx.data_mut(|data| data.insert_temp(anchor_id(), None::<Anchor>));
}

pub(crate) fn register(response: &egui::Response, bar: Bar) {
    if response.interact_rect.is_positive() {
        response
            .ctx
            .data_mut(|data| data.insert_temp(anchor_id(), Some(Anchor { rect: response.rect, bar })));
    }
}

pub(crate) fn toggle(ctx: &egui::Context) {
    if is_open(ctx) {
        close(ctx);
    } else {
        let anchor = ctx.data(|data| data.get_temp::<Option<Anchor>>(anchor_id())).flatten();
        ctx.data_mut(|data| {
            data.insert_temp(
                state_id(),
                Some(State {
                    anchor,
                    text: None,
                    fresh: true,
                }),
            )
        });
        ctx.request_repaint();
    }
}

fn position(screen: Rect, anchor: Anchor, size: Vec2) -> egui::Pos2 {
    let start = match anchor.bar {
        Bar::Top => anchor.rect.left_bottom() + Vec2::new(0.0, 8.0),
        Bar::Bottom => anchor.rect.left_top() - Vec2::new(0.0, 8.0 + size.y),
        Bar::Left => anchor.rect.right_top() + Vec2::new(8.0, 0.0),
        Bar::Right => anchor.rect.left_top() - Vec2::new(8.0 + size.x, 0.0),
    };
    egui::pos2(
        start
            .x
            .clamp(screen.left() + 8.0, (screen.right() - size.x - 8.0).max(screen.left() + 8.0)),
        start
            .y
            .clamp(screen.top() + 8.0, (screen.bottom() - size.y - 8.0).max(screen.top() + 8.0)),
    )
}

pub(crate) fn show(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, st: &StatusInfo) -> Option<BarAction> {
    let ctx = ui.ctx().clone();
    let mut state = ctx.data(|data| data.get_temp::<Option<State>>(state_id())).flatten()?;
    if ToolbarUi::editing(&ctx) || ToolbarUi::dragging(&ctx) {
        close(&ctx);
        return None;
    }
    if let Some(anchor) = ctx.data(|data| data.get_temp::<Option<Anchor>>(anchor_id())).flatten() {
        state.anchor = Some(anchor);
    }
    let screen = ctx.content_rect();
    let anchor = state.anchor.unwrap_or(Anchor {
        rect: Rect::from_min_size(screen.min + Vec2::splat(8.0), Vec2::splat(28.0)),
        bar: Bar::Top,
    });
    let inset = egui::Margin::same(6);
    let margin = widgets::overlay_frame(&ctx, p).inner_margin(inset).total_margin().sum();
    let size = Vec2::new(
        600.0_f32.min((screen.width() - 16.0).max(80.0)),
        ctx.global_style().spacing.interact_size.y + margin.y,
    );
    let at = position(screen, anchor, size);
    // under @showloc too: the SLURL is the hidden one, as the navigation
    // bar's field keeps it (LLLocationInputCtrl::changeLocationPresentation)
    if state.text.is_none() {
        state.text = Some(st.slurl.clone());
    }
    let mut open = true;
    let mut action = None;
    let shown = egui::Area::new(state_id())
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .constrain(false)
        .fade_in(false)
        .show(&ctx, |ui| {
            ui.set_max_size(size);
            widgets::overlay(ui, p, inset, |ui| {
                ui.set_width(size.x - margin.x);
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 52.0 - ui.spacing().item_spacing.x * 2.0).max(1.0);
                    let saved_text = std::mem::replace(&mut panels.nav_edit, state.text.take());
                    let saved_fresh = std::mem::replace(&mut panels.nav_edit_new, state.fresh);
                    action = super::location_field_scoped(ui, p, panels, st, &st.slurl, width, field_id());
                    state.text = std::mem::replace(&mut panels.nav_edit, saved_text);
                    state.fresh = std::mem::replace(&mut panels.nav_edit_new, saved_fresh);
                    // the hidden SLURL is copied too, like the place menu's
                    // Copy (LLStatusBar::onContextMenuItemClicked)
                    let copy = super::place::slurl_copy_enabled(st);
                    if widgets::action_button_if(ui, p, "copy-filled", "", widgets::ActionKind::Flat, Vec2::splat(26.0), copy)
                        .on_hover_text(aurora_i18n::tr!("common-copy"))
                        .clicked()
                        && copy
                    {
                        ctx.copy_text(st.slurl.clone());
                    }
                    if widgets::action_button(ui, p, "x", "", widgets::ActionKind::Flat, Vec2::splat(26.0))
                        .on_hover_text(aurora_i18n::tr!("common-close"))
                        .clicked()
                    {
                        open = false;
                    }
                });
            });
        });
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape)) && !egui::Popup::is_any_open(&ctx);
    let outside = ctx.input(|input| input.pointer.any_pressed())
        && ctx.pointer_interact_pos().is_some_and(|pos| {
            !shown.response.rect.contains(pos)
                && !anchor.rect.contains(pos)
                && ctx.layer_id_at(pos) != Some(egui::LayerId::new(egui::Order::Foreground, field_id().with("suggestions")))
        })
        && !egui::Popup::is_any_open(&ctx);
    if !open || escape || outside {
        close(&ctx);
    } else {
        ctx.data_mut(|data| data.insert_temp(state_id(), Some(state)));
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_rendering_keeps_integrated_text_and_fresh_state_in_its_own_scope() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        panels.nav_edit = Some("independent integrated edit".into());
        panels.nav_edit_new = true;
        toggle(&ctx);
        for _ in 0..3 {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui,
                        &theme.palette(),
                        &mut panels,
                        &super::super::islands::tests::status(true, false),
                    );
                },
            );
            assert_eq!(panels.nav_edit.as_deref(), Some("independent integrated edit"));
            assert!(panels.nav_edit_new);
            assert!(ctx.read_response(field_id()).is_some());
        }
    }
    #[test]
    fn address_popup_opens_towards_the_viewport_on_every_edge() {
        let screen = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 700.0));
        for (bar, point) in [
            (Bar::Top, egui::pos2(500.0, 8.0)),
            (Bar::Bottom, egui::pos2(500.0, 656.0)),
            (Bar::Left, egui::pos2(8.0, 350.0)),
            (Bar::Right, egui::pos2(956.0, 350.0)),
        ] {
            let anchor = Anchor {
                rect: Rect::from_min_size(point, Vec2::splat(36.0)),
                bar,
            };
            let rect = Rect::from_min_size(position(screen, anchor, Vec2::new(400.0, 44.0)), Vec2::new(400.0, 44.0));
            assert!(screen.shrink(8.0).contains_rect(rect));
            assert!(match bar {
                Bar::Top => rect.top() > anchor.rect.bottom(),
                Bar::Bottom => rect.bottom() < anchor.rect.top(),
                Bar::Left => rect.left() > anchor.rect.right(),
                Bar::Right => rect.right() < anchor.rect.left(),
            });
        }
    }
}
