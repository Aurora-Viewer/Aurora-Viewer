//! Shared local-preview mark: the same eye as the Local previews window.
//! Asset identity also marks pending images; GPU handles cover generic pictures.

use crate::theme::Palette;
use egui::{Context, Id, Rect, TextureHandle, TextureId, Ui, Vec2};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use uuid::Uuid;

pub(crate) const ICON: &str = "eye-filled";

#[derive(Default, PartialEq)]
struct Sources {
    assets: HashSet<Uuid>,
    images: HashSet<TextureId>,
}

fn key() -> Id {
    Id::unique("local_texture_preview_sources")
}

pub(crate) fn sync(ctx: &Context, assets: impl Iterator<Item = Uuid>, images: &HashMap<Uuid, TextureHandle>) {
    let assets: HashSet<_> = assets.collect();
    let images = assets.iter().filter_map(|asset| images.get(asset).map(TextureHandle::id)).collect();
    let sources = Sources { assets, images };
    ctx.data_mut(|data| {
        if data.get_temp::<Arc<Sources>>(key()).is_none_or(|old| *old != sources) {
            data.insert_temp(key(), Arc::new(sources));
        }
    });
}

pub(crate) fn is_local(ctx: &Context, asset: Uuid) -> bool {
    ctx.data(|data| {
        data.get_temp::<Arc<Sources>>(key())
            .is_some_and(|sources| sources.assets.contains(&asset))
    })
}

pub(crate) fn asset(ui: &Ui, p: &Palette, rect: Rect, asset: Option<Uuid>) {
    if asset.is_some_and(|asset| is_local(ui.ctx(), asset)) {
        paint(ui, p, rect);
    }
}

pub(crate) fn image(ui: &Ui, p: &Palette, rect: Rect, image: TextureId) {
    if ui.ctx().data(|data| {
        data.get_temp::<Arc<Sources>>(key())
            .is_some_and(|sources| sources.images.contains(&image))
    }) {
        paint(ui, p, rect);
    }
}

/// Overlay only: neither layout nor the preview's click/drag target changes.
pub(crate) fn paint(ui: &Ui, p: &Palette, preview: Rect) {
    if !ui.is_rect_visible(preview) || preview.size().min_elem() < 12.0 {
        return;
    }
    let side = (preview.size().min_elem() * 0.4)
        .clamp(16.0, 22.0)
        .min(preview.size().min_elem() - 4.0);
    let end = preview.right_bottom() - Vec2::splat(2.0);
    let rect = Rect::from_min_max(end - Vec2::splat(side), end);
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(preview));
    painter.rect_filled(rect, super::inner_radius(ui.visuals()), p.violet);
    if let Some(icon) = crate::ui::icons::global(ICON) {
        painter.image(
            icon.id(),
            rect.shrink(3.0),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            p.on_accent,
        );
    }
    let hint = aurora_i18n::tr!("local-texture-preview-hint");
    let response = ui.interact(
        rect,
        ui.scope_id()
            .with(("local_texture_badge", rect.min.x.to_bits(), rect.min.y.to_bits())),
        egui::Sense::hover(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, hint));
    response.on_hover_text(hint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_sources_track_pending_images_reloads_and_removal_without_marking_inventory() {
        let ctx = Context::default();
        let local = Uuid::from_u128(1);
        let remote = Uuid::from_u128(2);
        sync(&ctx, [local].into_iter(), &HashMap::new());
        assert!(is_local(&ctx, local));
        assert!(!is_local(&ctx, remote));
        let texture = ctx.load_texture("local", egui::ColorImage::filled([1, 1], egui::Color32::WHITE), Default::default());
        sync(&ctx, [local].into_iter(), &HashMap::from([(local, texture.clone())]));
        assert!(ctx.data(|data| data.get_temp::<Arc<Sources>>(key()).unwrap().images.contains(&texture.id())));
        sync(&ctx, std::iter::empty(), &HashMap::from([(local, texture)]));
        assert!(!is_local(&ctx, local));
        assert!(ctx.data(|data| data.get_temp::<Arc<Sources>>(key()).unwrap().images.is_empty()));
    }

    #[test]
    fn local_badge_stays_inside_small_swatches_and_does_not_intercept_their_clicks() {
        for side in [32.0, 60.0, 132.0] {
            let ctx = Context::default();
            let p = crate::theme::Theme::default().palette();
            let local = Uuid::from_u128(1);
            sync(&ctx, [local].into_iter(), &HashMap::new());
            let mut preview = Rect::NOTHING;
            let mut clicked = false;
            let mut draw = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let response = crate::build::ui::texture_swatch(
                            ui,
                            &p,
                            &HashMap::new(),
                            &mut HashSet::new(),
                            Some(local),
                            "",
                            side,
                            false,
                            true,
                        );
                        preview = response.rect;
                        clicked = response.clicked();
                    },
                );
                output.textures_delta.clear();
                output
            };
            let output = draw(vec![]);
            let badge = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == p.violet => Some(rect.rect),
                    _ => None,
                })
                .expect("local eye background");
            for pressed in [true, false] {
                let _ = draw(vec![
                    egui::Event::PointerMoved(badge.center()),
                    egui::Event::PointerButton {
                        pos: badge.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            assert!(preview.contains_rect(badge));
            assert!(clicked, "badge must not replace the texture button's click target");
        }
    }
}
