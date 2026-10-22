//! Loading (after login) and teleport screens: one shared full-screen
//! "banner" over the last view blurred, or the aurora background. No panel:
//! the place is written large at the bottom left with what is known of it
//! (classification, arrival point, residents), its world map tile and the
//! progress in figures sit at the bottom right, and the progress itself is a
//! line of light along the bottom edge. An Aurora presentation, requested by
//! the human; Firestorm shows a progress bar over a snapshot.

use crate::theme::Palette;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, Shape, Vec2};

pub struct LoadingInfo<'a> {
    pub stage: &'a str,
    pub fraction: f32,
    pub region: &'a str,
    pub objects: usize,
    pub textures: (usize, usize),
    pub terrain: f32,
    pub elapsed: f32,
}

/// What is known of the place being loaded, beyond its name.
#[derive(Default)]
pub struct Place<'a> {
    /// Classification label (`bars::maturity_name`), empty when unknown.
    pub maturity: &'a str,
    /// Arrival point "x, y, z", empty when unknown.
    pub position: &'a str,
    /// Residents in the region.
    pub residents: Option<usize>,
    /// World map tile of the region.
    pub tile: Option<&'a egui::TextureHandle>,
    /// Arrival point on the tile, 0..1 from its south-west corner.
    pub pin: Option<(f32, f32)>,
}

/// The facts of a `Place`, gathered by the app each frame the banner shows.
#[derive(Default)]
pub struct PlaceFacts {
    pub maturity: &'static str,
    pub position: String,
    pub residents: Option<usize>,
    pub tile: Option<egui::TextureHandle>,
    pub pin: Option<(f32, f32)>,
}

impl PlaceFacts {
    pub fn place(&self) -> Place<'_> {
        Place {
            maturity: self.maturity,
            position: &self.position,
            residents: self.residents,
            tile: self.tile.as_ref(),
            pin: self.pin,
        }
    }

    /// What the viewer knows of the region `name`. `arrived`: it is the
    /// region we are in (its handshake and coarse locations are there);
    /// otherwise only the world map may know it, and its residents are asked
    /// for (MapItemRequest, throttled by the map).
    pub fn gather(
        world: &mut crate::world::World,
        tiles: &mut super::map_tiles::MapTiles,
        name: &str,
        position: Option<glam::Vec3>,
        arrived: bool,
    ) -> PlaceFacts {
        let mut facts = PlaceFacts::default();
        if let Some(pos) = position {
            facts.position = format!("{:.0}, {:.0}, {:.0}", pos.x, pos.y, pos.z);
        }
        let main = world.main_region.filter(|_| arrived);
        let grid = match main {
            Some(handle) => Some(((handle >> 32) as u32 / 256, (handle & 0xffff_ffff) as u32 / 256)),
            None => world.map.region_by_name(name),
        };
        let Some(key) = grid else {
            return facts;
        };
        let sim = world.map.sims.get(&key);
        let size = sim.map_or(256.0, |s| s.size_x.max(1) as f32);
        facts.maturity = match main.and_then(|h| world.regions.get(&h)).and_then(|r| r.info.as_ref()) {
            Some(info) => super::bars::maturity_name(info.sim_access),
            None => sim.map_or("", |s| super::bars::maturity_name(s.access)),
        };
        facts.residents = match main {
            Some(handle) => world.coarse.get(&handle).map(Vec::len).filter(|n| *n > 0),
            None => {
                world.map.want_agents(key);
                world
                    .map
                    .agents
                    .get(&key)
                    .map(|spots| spots.iter().map(|(_, _, count)| (*count).max(0) as usize).sum())
            }
        };
        facts.tile = tiles.get(1, key.0, key.1, true).ok();
        facts.pin = position.map(|pos| (pos.x / size, pos.y / size));
        facts
    }
}

/// Split a teleport destination "Region (x, y, z)" into the region and the
/// arrival point; a destination without a point (a landmark, home) is all
/// name.
pub fn split_destination(dest: &str) -> (&str, Option<glam::Vec3>) {
    let Some((name, point)) = dest.rsplit_once('(') else {
        return (dest.trim(), None);
    };
    let mut numbers = point.trim_end_matches(')').split(',').map(|n| n.trim().parse::<f32>());
    match (numbers.next(), numbers.next(), numbers.next(), numbers.next()) {
        (Some(Ok(x)), Some(Ok(y)), Some(Ok(z)), None) => (name.trim(), Some(glam::Vec3::new(x, y, z))),
        _ => (dest.trim(), None),
    }
}

/// Fade-out duration of the loading / teleport screens.
pub const FADE_SECONDS: f32 = 0.7;

/// Background of the loading screens: the last view (blurred) or the
/// aurora background.
pub fn draw_backdrop(ui: &egui::Ui, p: &Palette, backdrop: Option<&super::backdrop::Backdrop>) {
    let rect = ui.ctx().content_rect();
    let painter = ui.ctx().layer_painter(egui::LayerId::background());
    if !backdrop.is_some_and(|b| b.paint(&painter, p, rect, 1.0)) {
        super::login::draw_background(ui, p);
    }
}

/// Rectangle blending two colors: left to right when `horizontal`, else top
/// to bottom.
pub(crate) fn gradient_rect(painter: &egui::Painter, r: Rect, a: Color32, b: Color32, horizontal: bool) {
    let mut m = Mesh::default();
    let v = |pos: Pos2, color: Color32| Vertex { pos, uv: WHITE_UV, color };
    let (right_top, left_bottom) = if horizontal { (b, a) } else { (a, b) };
    m.vertices.push(v(r.left_top(), a));
    m.vertices.push(v(r.right_top(), right_top));
    m.vertices.push(v(r.right_bottom(), b));
    m.vertices.push(v(r.left_bottom(), left_bottom));
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(m));
}

/// The filled part of a progress track. `fraction` None = moving segment
/// (unknown duration).
fn progress_fill(r: Rect, fraction: Option<f32>, t: f32) -> Rect {
    match fraction {
        Some(f) => Rect::from_min_size(r.min, Vec2::new(r.width() * f.clamp(0.0, 1.0), r.height())),
        None => {
            let seg = r.width() * 0.28;
            let u = 0.5 - 0.5 * (t * 2.2).cos();
            Rect::from_min_size(Pos2::new(r.left() + (r.width() - seg) * u, r.top()), Vec2::new(seg, r.height()))
        }
    }
}

/// The lit part of the banner's progress line: the brand gradient, a small
/// aurora rising from it (waving, taller toward the head), a sheen that
/// travels along it and a bright core at its head.
fn progress_light(painter: &egui::Painter, p: &Palette, fill: Rect, k: f32, t: f32, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    gradient_rect(painter, fill, p.violet.gamma_multiply(a), p.teal.gamma_multiply(a), true);

    // aurora curtain: a strip of quads above the line, opaque-ish at its
    // foot and transparent at its waving top
    const SEGMENTS: usize = 64;
    let mut curtain = Mesh::default();
    for i in 0..=SEGMENTS {
        let u = i as f32 / SEGMENTS as f32;
        let x = fill.left() + fill.width() * u;
        // two waves of different lengths drifting against each other
        let wave = 0.62 + 0.38 * (x * 0.021 / k + t * 1.25).sin() * (x * 0.0083 / k - t * 0.7).cos();
        // rises toward the head, then falls into its halo instead of ending on an edge
        let toward_head = u.powf(1.6) * (1.0 - ((u - 0.9) / 0.1).clamp(0.0, 1.0).powi(2));
        let height = k * (8.0 + 44.0 * toward_head) * wave;
        let color = p.violet.lerp_to_gamma(p.teal, u);
        let foot = color.gamma_multiply(a * (0.10 + 0.34 * toward_head));
        let v = |pos: Pos2, color: Color32| Vertex { pos, uv: WHITE_UV, color };
        curtain.vertices.push(v(Pos2::new(x, fill.top()), foot));
        curtain.vertices.push(v(Pos2::new(x, fill.top() - height), Color32::TRANSPARENT));
        if i > 0 {
            let n = (i * 2) as u32;
            curtain.add_triangle(n - 2, n - 1, n);
            curtain.add_triangle(n - 1, n + 1, n);
        }
    }
    painter.add(Shape::mesh(curtain));

    // sheen: a highlight crossing the lit part every few seconds
    let width = (120.0 * k).min(fill.width());
    let travel = fill.width() + width;
    let center = fill.left() - width / 2.0 + travel * (t * 0.32).fract();
    // theme-guard: brand — the light of the sheen, whatever the skin
    let sheen = Color32::WHITE.gamma_multiply(0.6 * a);
    let lit = painter.with_clip_rect(fill.intersect(painter.clip_rect()));
    let left = Rect::from_min_max(Pos2::new(center - width / 2.0, fill.top()), Pos2::new(center, fill.bottom()));
    let right = Rect::from_min_max(Pos2::new(center, fill.top()), Pos2::new(center + width / 2.0, fill.bottom()));
    gradient_rect(&lit, left, Color32::TRANSPARENT, sheen, true);
    gradient_rect(&lit, right, sheen, Color32::TRANSPARENT, true);

    // head: a soft halo breathing around a bright core
    let head = Pos2::new(fill.right(), fill.center().y);
    let breathe = 0.8 + 0.2 * (t * 3.1).sin();
    super::aurora_bg::radial_glow(painter, head, 30.0 * k, p.teal, 0.5 * breathe * a);
    super::aurora_bg::radial_glow(painter, head, 9.0 * k, p.ink, 0.95 * a);
}

/// What starts a fact about the place: an icon or a colored letter.
enum Lead<'a> {
    Icon(&'a str),
    Letter(&'a str, Color32, Color32),
}

/// Paint a fact as a chip from its top left corner; returns its width.
#[allow(clippy::too_many_arguments)]
fn chip(painter: &egui::Painter, p: &Palette, at: Pos2, k: f32, lead: Lead, text: &str, alpha: f32, radius: egui::CornerRadius) -> f32 {
    let height = 24.0 * k;
    let pad = 8.0 * k;
    let mark = 14.0 * k;
    let ink = p.ink.gamma_multiply(alpha);
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.5 * k), ink);
    let width = pad + mark + 6.0 * k + galley.size().x + pad;
    let rect = Rect::from_min_size(at, Vec2::new(width, height));
    let face = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), 168);
    painter.rect_filled(rect, radius, face.gamma_multiply(alpha));
    let mark_rect = Rect::from_center_size(Pos2::new(rect.left() + pad + mark / 2.0, rect.center().y), Vec2::splat(mark));
    match lead {
        Lead::Icon(name) => {
            if let Some(texture) = super::icons::global(name) {
                painter.image(
                    texture.id(),
                    mark_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    p.violet_light.gamma_multiply(alpha),
                );
            }
        }
        Lead::Letter(letter, bg, fg) => {
            painter.rect_filled(
                mark_rect,
                super::widgets::inner_radius(&painter.ctx().global_style().visuals),
                bg.gamma_multiply(alpha),
            );
            let fg = fg.gamma_multiply(alpha);
            let letter = painter.layout_no_wrap(letter.to_owned(), egui::FontId::proportional(mark * 0.74), fg);
            super::widgets::paint_ink_centered(painter, letter, mark_rect.center(), fg);
        }
    }
    let text_at = Pos2::new(mark_rect.right() + 6.0 * k, rect.center().y);
    super::widgets::paint_ink_left(painter, galley, text_at, ink);
    width
}

/// The shared banner over `screen`. Painted by hand so everything fades
/// with `alpha`. Returns the point facing the brand at the top right (for
/// an optional button).
#[allow(clippy::too_many_arguments)]
fn banner(
    ctx: &egui::Context,
    painter: &egui::Painter,
    p: &Palette,
    screen: Rect,
    logo: Option<&egui::TextureHandle>,
    label: &str,
    name: &str,
    stage: &str,
    fraction: Option<f32>,
    place: &Place,
    t: f32,
    alpha: f32,
) -> Pos2 {
    let a = alpha.clamp(0.0, 1.0);
    // sizes follow the window: designed at 900 px of height
    let k = (screen.height() / 900.0).clamp(0.7, 1.5);
    let margin = 48.0 * k;
    let radius = ctx.global_style().visuals.window_corner_radius;
    let navy = |opacity: f32| Color32::from_rgba_unmultiplied(p.navy.r(), p.navy.g(), p.navy.b(), (255.0 * opacity * a) as u8);

    // veils: the bottom one carries the text, a light one keeps the brand readable
    let low = Rect::from_min_max(Pos2::new(screen.left(), screen.top() + screen.height() * 0.42), screen.max);
    gradient_rect(painter, low, navy(0.0), navy(0.9), false);
    let high = Rect::from_min_max(screen.min, Pos2::new(screen.right(), screen.top() + screen.height() * 0.2));
    gradient_rect(painter, high, navy(0.45), navy(0.0), false);

    // brand, top left
    let brand = Pos2::new(screen.left() + margin, screen.top() + margin * 0.85);
    let mut brand_text = brand.x;
    if let Some(l) = logo {
        let size = 40.0 * k;
        let rect = Rect::from_min_size(Pos2::new(brand.x, brand.y - size / 2.0), Vec2::splat(size));
        let breathe = 0.85 + 0.15 * (t * 1.4).sin();
        super::aurora_bg::radial_glow(painter, rect.center(), size * 1.1, p.violet, 0.4 * breathe * a);
        painter.image(
            l.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            super::widgets::PICTURE_TINT.gamma_multiply(a),
        );
        brand_text = rect.right() + 12.0 * k;
    }
    painter.text(
        Pos2::new(brand_text, brand.y),
        egui::Align2::LEFT_CENTER,
        "Aurora Viewer",
        egui::FontId::proportional(15.0 * k),
        p.violet_pale.gamma_multiply(a),
    );

    // progress: a line of light along the bottom edge
    let line_height = (3.0 * k).max(2.0);
    let track = Rect::from_min_max(Pos2::new(screen.left(), screen.bottom() - line_height), screen.max);
    painter.rect_filled(track, 0.0, navy(0.55));
    let fill = progress_fill(track, fraction, t);
    if fill.width() >= 1.0 {
        progress_light(&painter.with_clip_rect(screen), p, fill, k, t, a);
    }

    // bottom right: the region on the world map, and the progress in figures
    let base = track.top() - margin * 0.8;
    let mut right = screen.right() - margin;
    if let Some(tile) = place.tile {
        let side = 132.0 * k;
        let rect = Rect::from_min_size(Pos2::new(right - side, base - side), Vec2::splat(side));
        let mut shadow = ctx.global_style().visuals.window_shadow;
        shadow.color = shadow.color.gamma_multiply(a);
        painter.add(shadow.as_shape(rect, radius));
        painter.add(
            egui::epaint::RectShape::filled(rect, radius, super::widgets::PICTURE_TINT.gamma_multiply(a))
                .with_texture(tile.id(), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
        );
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.0, p.ink.gamma_multiply(0.28 * a)),
            egui::StrokeKind::Inside,
        );
        if let Some((x, y)) = place.pin {
            let at = Pos2::new(
                rect.left() + rect.width() * x.clamp(0.0, 1.0),
                rect.bottom() - rect.height() * y.clamp(0.0, 1.0),
            );
            let pulse = 0.6 + 0.4 * (t * 2.4).sin().abs();
            super::aurora_bg::radial_glow(&painter.with_clip_rect(rect), at, 16.0 * k, p.violet_light, 0.7 * pulse * a);
            painter.circle_filled(at, 3.5 * k, p.ink.gamma_multiply(a));
            painter.circle_stroke(at, 3.5 * k, egui::Stroke::new(1.5 * k, p.violet.gamma_multiply(a)));
        }
        right = rect.left() - 20.0 * k;
    }
    if let Some(f) = fraction {
        let muted = p.muted.gamma_multiply(a);
        let ink = p.ink.gamma_multiply(a);
        let sign = painter.layout_no_wrap(" %".to_owned(), egui::FontId::proportional(16.0 * k), muted);
        let figures = aurora_i18n::number_text(&format!("{:.0}", f.clamp(0.0, 1.0) * 100.0));
        let figures = painter.layout_no_wrap(figures, egui::FontId::proportional(36.0 * k), ink);
        let sign_at = Pos2::new(right - sign.size().x, base - sign.size().y - 3.0 * k);
        let figures_at = Pos2::new(sign_at.x - figures.size().x, base - figures.size().y + 3.0 * k);
        painter.galley(sign_at, sign, muted);
        painter.galley(figures_at, figures, ink);
        right = figures_at.x - 24.0 * k;
    }

    // bottom left, built upwards: stage, facts, name, label
    let left = screen.left() + margin;
    let column = painter.with_clip_rect(Rect::from_min_max(screen.min, Pos2::new(right, screen.bottom())));
    let mut y = base;
    if !stage.is_empty() {
        let muted = p.muted.gamma_multiply(a);
        let galley = column.layout_no_wrap(stage.to_owned(), egui::FontId::proportional(13.0 * k), muted);
        y -= galley.size().y;
        column.galley(Pos2::new(left, y), galley, muted);
        y -= 12.0 * k;
    }
    let mut facts: Vec<(Lead, String)> = Vec::new();
    if let Some((letter, bg, fg)) = super::widgets::maturity_style(p, place.maturity) {
        facts.push((Lead::Letter(letter, bg, fg), place.maturity.to_owned()));
    }
    if !place.position.is_empty() {
        facts.push((Lead::Icon("map-pin-filled"), place.position.to_owned()));
    }
    if let Some(count) = place.residents {
        facts.push((Lead::Icon("users"), aurora_i18n::tr!("loading-residents", count = count)));
    }
    if !facts.is_empty() {
        y -= 24.0 * k;
        let mut x = left;
        for (lead, text) in facts {
            x += chip(&column, p, Pos2::new(x, y), k, lead, &text, a, radius) + 6.0 * k;
        }
        y -= 14.0 * k;
    }
    if !name.is_empty() {
        // a long name shrinks rather than running under the figures
        let ink = p.ink.gamma_multiply(a);
        let room = (right - left).max(120.0);
        let mut size = 44.0 * k;
        let mut galley = column.layout_no_wrap(name.to_owned(), egui::FontId::proportional(size), ink);
        if galley.size().x > room {
            size = (size * room / galley.size().x).max(22.0 * k);
            galley = column.layout_no_wrap(name.to_owned(), egui::FontId::proportional(size), ink);
        }
        y -= galley.size().y;
        column.galley(Pos2::new(left, y), galley, ink);
        y -= 2.0 * k;
    }
    if !label.is_empty() {
        let accent = p.violet_light.gamma_multiply(a);
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &label.to_uppercase(),
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(11.5 * k),
                color: accent,
                extra_letter_spacing: 1.6 * k,
                ..Default::default()
            },
        );
        let galley = column.layout_job(job);
        y -= galley.size().y;
        column.galley(Pos2::new(left, y), galley, accent);
    }
    Pos2::new(screen.right() - margin, screen.top() + margin * 0.85)
}

/// The stage line of the loading screen: the stage alone until the region
/// is known, then with what has arrived of it.
pub fn stage_line(stage: &str, region: &str, terrain: f32, objects: usize, textures: (usize, usize)) -> String {
    if region.is_empty() {
        return stage.to_owned();
    }
    aurora_i18n::tr!(
        "status-terrain-objects-textures",
        value0 = stage.to_string(),
        value1 = aurora_i18n::number_text(&format!("{:.0}", terrain * 100.0)),
        value2 = format!("{objects}"),
        value3 = format!("{}", textures.0),
        value4 = format!("{}", textures.1)
    )
}

/// What a banner says under its title. The app keeps it while the screen
/// fades out, so that the screen leaves as it was: a line or a fact that
/// came or went during the fade made the others jump.
pub struct BannerText {
    pub name: String,
    pub stage: String,
    pub facts: PlaceFacts,
}

/// Loading screen between login and the first rendered frames.
pub fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    logo: Option<&egui::TextureHandle>,
    backdrop: Option<&super::backdrop::Backdrop>,
    info: &LoadingInfo,
    place: &Place,
) -> bool {
    draw_backdrop(ui, p, backdrop);
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("loading_splash")));
    let t = ctx.input(|i| i.time) as f32;
    let detail = stage_line(info.stage, info.region, info.terrain, info.objects, info.textures);
    let corner = banner(
        &ctx,
        &painter,
        p,
        screen,
        logo,
        aurora_i18n::tr!("common-loading-world"),
        info.region,
        &detail,
        Some(info.fraction),
        place,
        t,
        1.0,
    );
    ctx.request_repaint();
    let mut cancel = false;
    if info.elapsed > 20.0 {
        egui::Area::new(egui::Id::unique("loading_cancel"))
            .pivot(egui::Align2::RIGHT_CENTER)
            .fixed_pos(corner)
            .show(&ctx, |ui| {
                let label = aurora_i18n::tr!("common-cancel");
                let kind = super::widgets::ActionKind::Flat;
                if super::widgets::action_button(ui, p, "x", label, kind, Vec2::new(0.0, 28.0)).clicked() {
                    cancel = true;
                }
            });
    }
    cancel
}

/// Teleport screen over the world (and the end of the loading screen fading
/// out): the same banner.
#[allow(clippy::too_many_arguments)]
pub fn teleport_overlay(
    ctx: &egui::Context,
    p: &Palette,
    logo: Option<&egui::TextureHandle>,
    backdrop: Option<&super::backdrop::Backdrop>,
    title: &str,
    dest: &str,
    detail: &str,
    fraction: Option<f32>,
    alpha: f32,
    place: &Place,
) {
    let a = alpha.clamp(0.0, 1.0);
    let rect = ctx.content_rect();
    let t = ctx.input(|i| i.time) as f32;
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("teleport_overlay"));
    egui::Area::new(layer.id)
        .order(layer.order)
        .fixed_pos(rect.min)
        .fade_in(false)
        .interactable(a > 0.5)
        .show(ctx, |ui| {
            // swallow clicks while it covers the screen
            ui.allocate_exact_size(rect.size(), egui::Sense::click_and_drag());
        });
    // Painted on the layer of the area, not through its `Ui`: egui hides a
    // new area for its first frame (sizing pass), and the world showed for
    // that frame between the loading screen and its fade out.
    let painter = ctx.layer_painter(layer).with_clip_rect(rect);
    if !backdrop.is_some_and(|b| b.paint(&painter, p, rect, a)) {
        super::aurora_bg::paint(&painter, p, rect, t, a);
    }
    banner(ctx, &painter, p, rect, logo, title, dest, detail, fraction, place, t, a);
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_splits_into_region_and_arrival_point() {
        assert_eq!(
            split_destination("Lagune Boréale (128, 64, 30)"),
            ("Lagune Boréale", Some(glam::Vec3::new(128.0, 64.0, 30.0)))
        );
        assert_eq!(split_destination("(12, 34, 56)"), ("", Some(glam::Vec3::new(12.0, 34.0, 56.0))));
        // a name with parentheses of its own, a landmark, nothing
        assert_eq!(split_destination("Port (nord)"), ("Port (nord)", None));
        assert_eq!(split_destination("Domicile"), ("Domicile", None));
        assert_eq!(split_destination(""), ("", None));
    }

    #[test]
    fn the_teleport_screen_paints_in_its_first_frame() {
        // the loading screen hands over to it at full opacity: a frame
        // without it shows the world between the two
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                ..Default::default()
            },
            |ui| {
                teleport_overlay(
                    ui.ctx(),
                    &p,
                    None,
                    None,
                    "Loading",
                    "Lagune",
                    "Scene",
                    Some(1.0),
                    1.0,
                    &Place::default(),
                )
            },
        );
        output.textures_delta.clear();
        let painted = output.shapes.iter().any(|clipped| match &clipped.shape {
            Shape::Text(text) => text.galley.text() == "Lagune",
            _ => false,
        });
        assert!(painted, "the region name is not painted in the first frame");
    }
}
