//! The interface of avatar marks (`world/avatar_marks.rs`): the "Markers"
//! submenu shared by the radar lists (both People looks, one resident or
//! several) and the mini-maps (menu_fs_radar.xml,
//! menu_fs_radar_multiselect.xml, menu_mini_map.xml), the ring of a marked
//! resident's picture in the People lists, and the custom colour window.
//! The Legacy Viewer offers its five colours only and shows the mark on the
//! maps and the name tags, not in the radar list: the custom colour and the
//! ring are Aurora additions, asked for by the human.

use super::context::{self, CtxAction};
use super::menu;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::avatar_marks::{LEGACY_COLORS, MarkColor, Rgb};
use egui::{Color32, Rect, RichText, Stroke, Vec2};
use uuid::Uuid;

const WINDOW: &str = "avatar_mark_color";
/// Width of the ring around a marked picture.
const RING: f32 = 2.0;

/// A mark's colour as the menus and the lists draw it.
pub fn color(rgb: Rgb) -> Color32 {
    let [r, g, b] = rgb;
    Color32::from_rgb(r, g, b) // theme-guard: content — the colour the user marked a resident with
}

fn label(name: MarkColor) -> &'static str {
    match name {
        MarkColor::Red => aurora_i18n::tr!("ui-minimap-red"),
        MarkColor::Green => aurora_i18n::tr!("ui-minimap-green"),
        MarkColor::Blue => aurora_i18n::tr!("ui-minimap-blue"),
        MarkColor::Purple => aurora_i18n::tr!("ui-minimap-purple"),
        MarkColor::Yellow => aurora_i18n::tr!("ui-minimap-yellow"),
    }
}

/// "Markers ▸": the five colours, "Custom color…", "Clear marker" for
/// `ids`, "Clear all markers" (the MarkAvatar menu). Each choice goes to
/// the app as a `CtxAction`.
pub fn submenu(ui: &mut egui::Ui, p: &Palette, world: &World, ids: &[Uuid]) {
    let marks = &world.avatar_marks;
    menu::submenu(ui, p, "tag-filled", aurora_i18n::tr!("ui-minimap-markers"), !ids.is_empty(), |ui| {
        for (name, rgb) in LEGACY_COLORS {
            if menu::swatch(ui, p, color(rgb), label(name)) {
                context::request(ui.ctx(), CtxAction::MarkAvatars(ids.to_vec(), rgb));
            }
        }
        if menu::item(ui, p, "palette-filled", aurora_i18n::tr!("ui-minimap-custom-marker")) {
            context::request(ui.ctx(), CtxAction::MarkCustom(ids.to_vec()));
        }
        menu::separator(ui, p);
        let marked = ids.iter().any(|id| marks.contains(id));
        if menu::item_if(ui, p, "backspace-filled", aurora_i18n::tr!("ui-minimap-clear-marker"), marked) {
            context::request(ui.ctx(), CtxAction::ClearMarks(ids.to_vec()));
        }
        menu::separator(ui, p);
        if menu::item_if(
            ui,
            p,
            "backspace-filled",
            aurora_i18n::tr!("ui-minimap-clear-all-markers"),
            !marks.is_empty(),
        ) {
            context::request(ui.ctx(), CtxAction::ClearAllMarks);
        }
    });
}

/// The ring of a marked resident around their picture in a People list:
/// `round` for Aurora's discs, the theme's corners for the Legacy table's
/// squares. Nothing for a resident without a mark.
pub fn ring(ui: &egui::Ui, world: &World, id: &Uuid, picture: Rect, round: bool) {
    let Some(rgb) = world.avatar_marks.get(id) else {
        return;
    };
    let stroke = Stroke::new(RING, color(rgb));
    if round {
        ui.painter()
            .circle_stroke(picture.center(), picture.width() / 2.0 + RING / 2.0, stroke);
    } else {
        ui.painter().rect_stroke(
            picture.expand(RING / 2.0),
            ui.visuals().window_corner_radius,
            stroke,
            egui::StrokeKind::Middle,
        );
    }
}

/// The custom colour window: the interface's compact picker
/// (`widgets::color::color_panel`), whose colour marks its residents as it
/// changes.
pub struct CustomMark {
    ids: Vec<Uuid>,
    color: widgets::color::Rgba,
    state: widgets::color::ColorPickState,
    /// Opened from a menu: comes to the front once shown.
    raise: bool,
}

impl CustomMark {
    /// Starts from the colour the residents share, else the last custom
    /// colour of the session, else the menu's first.
    pub fn new(world: &World, ids: Vec<Uuid>, last: Option<Rgb>) -> Self {
        let [r, g, b] = world.avatar_marks.shared(&ids).or(last).unwrap_or(LEGACY_COLORS[0].1);
        let color = [r, g, b, 255];
        CustomMark {
            ids,
            color,
            state: widgets::color::ColorPickState::new(egui::Id::new(WINDOW), color),
            raise: true,
        }
    }

    /// The colour shown now, kept for the next opening.
    pub fn rgb(&self) -> Rgb {
        [self.color[0], self.color[1], self.color[2]]
    }
}

/// What the window's line under the title names: who its colour marks.
fn target(world: &World, ids: &[Uuid]) -> String {
    match ids {
        [one] if world.rlva_can_name(*one) => world.person_name(one).unwrap_or_else(|| "…".into()),
        many => aurora_i18n::tr!("ui-avatar-mark-many", count = many.len()).to_owned(),
    }
}

/// Draws the custom colour window; `picker` becomes `None` once it closed.
pub fn show(ctx: &egui::Context, p: &Palette, world: &mut World, picker: &mut Option<CustomMark>) {
    let Some(mark) = picker else {
        return;
    };
    let size = Vec2::new(280.0, 220.0);
    let mut open = true;
    let who = target(world, &mark.ids);
    Floater::new(
        WINDOW,
        aurora_i18n::tr!("ui-avatar-mark-title"),
        ctx.content_rect().center() - size / 2.0,
        size,
    )
    .auto_height()
    .show(ctx, p, &mut open, |ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(6.0);
        ui.label(RichText::new(who).strong().color(p.ink));
        if widgets::color::color_panel(ui, p, &mut mark.state, &mut mark.color, false) {
            let rgb = mark.rgb();
            world.avatar_marks.set(&mark.ids, rgb);
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-avatar-mark-private"))
                .size(11.0)
                .color(p.muted),
        );
    });
    if std::mem::take(&mut mark.raise) {
        widgets::bring_floater_front(ctx, WINDOW);
    }
    if !open {
        *picker = None;
    }
}

/// `AURORA_DEMO_RADAR_MARKS=1|legacy`: whether the scenario runs, and in
/// the Legacy look of People (its radar table) rather than Aurora's.
pub fn demo_mode() -> Option<bool> {
    static MODE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_RADAR_MARKS").ok()?;
        Some(v.trim() == "legacy")
    })
}

/// The scenario runs in the Legacy look of People.
pub fn demo_legacy() -> bool {
    demo_mode() == Some(true)
}

/// `AURORA_DEMO_RADAR_MARKS`: the demo crowd's first residents marked, one
/// of each Legacy colour and a custom one, so that the lists and the
/// mini-maps show every kind at once.
pub fn demo(world: &mut World, crowd: &[Uuid]) {
    for (id, rgb) in crowd.iter().zip(LEGACY_COLORS.iter().map(|(_, rgb)| *rgb).chain([DEMO_CUSTOM])) {
        world.avatar_marks.set(&[*id], rgb);
    }
}

/// The custom colour of the demo: an orange none of the menu's colours is.
pub const DEMO_CUSTOM: Rgb = [255, 140, 0];

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    #[test]
    fn the_custom_window_starts_from_the_shared_mark_then_the_last_colour() {
        let mut w = world();
        let (a, b) = (Uuid::from_u128(11), Uuid::from_u128(12));
        // nothing marked, nothing chosen before: the menu's first colour
        assert_eq!(CustomMark::new(&w, vec![a], None).rgb(), LEGACY_COLORS[0].1);
        assert_eq!(CustomMark::new(&w, vec![a], Some([1, 2, 3])).rgb(), [1, 2, 3]);
        w.avatar_marks.set(&[a, b], [9, 8, 7]);
        assert_eq!(CustomMark::new(&w, vec![a, b], Some([1, 2, 3])).rgb(), [9, 8, 7]);
    }

    #[test]
    fn the_window_names_one_resident_or_counts_several() {
        let w = world();
        let (a, b) = (Uuid::from_u128(11), Uuid::from_u128(12));
        assert!(target(&w, &[a, b]).contains('2'));
        assert!(!target(&w, &[a]).is_empty());
    }

    #[test]
    fn the_mini_map_dot_and_the_name_tag_take_the_mark_over_a_contact_set() {
        use crate::ui::colors;
        let mut w = world();
        let id = Uuid::from_u128(10);
        let sets = &mut w.contact_sets;
        sets.add_set("Map");
        sets.add_to_set(&[id], "Map", |_| false);
        sets.set_color("Map", [0.2, 0.8, 0.5, 1.0]);
        sets.set_visuals(
            "Map",
            crate::world::contact_sets::Visuals {
                minimap: true,
                ..Default::default()
            },
        );
        let set = colors::set_color([0.2, 0.8, 0.5, 1.0]);
        assert_eq!(colors::map_color(&w, &id, "Example"), set);
        let plain_tag = colors::tag_color(&w, &id, "Example", false);
        assert!(!colors::relation_tag(&w, &id, "Example"));
        let blue = LEGACY_COLORS[2].1;
        w.avatar_marks.set(&[id], blue);
        // the dot of every mini-map and the mouselook arrows: the mark
        let dot = colors::map_color(&w, &id, "Example");
        assert_eq!(dot, colors::c([0, 0, 255, 255]));
        assert_eq!(colors::mark_color(&w, &id), Some(dot));
        // the name tag: the mark too, and no distance colour over it
        assert_ne!(colors::tag_color(&w, &id, "Example", false), plain_tag);
        assert_eq!(colors::tag_color(&w, &id, "Example", false), colors::text([0, 0, 255, 255]));
        assert!(colors::relation_tag(&w, &id, "Example"));
        // a custom colour reaches the dot as chosen
        w.avatar_marks.set(&[id], DEMO_CUSTOM);
        assert_eq!(colors::map_color(&w, &id, "Example"), colors::c([255, 140, 0, 255]));
        // cleared: back to the contact set's colour
        w.avatar_marks.clear(&[id]);
        assert_eq!(colors::map_color(&w, &id, "Example"), set);
        assert_eq!(colors::tag_color(&w, &id, "Example", false), plain_tag);
        assert_eq!(colors::mark_color(&w, &id), None);
    }

    #[test]
    fn the_demo_marks_one_resident_per_colour_and_one_custom() {
        let mut w = world();
        let crowd: Vec<Uuid> = (100..110).map(Uuid::from_u128).collect();
        demo(&mut w, &crowd);
        for (i, (_, rgb)) in LEGACY_COLORS.iter().enumerate() {
            assert_eq!(w.avatar_marks.get(&crowd[i]), Some(*rgb));
        }
        assert_eq!(w.avatar_marks.get(&crowd[5]), Some(DEMO_CUSTOM));
        assert!(!w.avatar_marks.contains(&crowd[6]));
    }
}
