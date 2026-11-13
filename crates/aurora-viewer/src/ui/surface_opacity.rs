//! See-through windows and bars (Preferences › Appearance › Skin). Port of
//! the Legacy Viewer's Floating Window Opacity: ActiveFloaterTransparency and
//! InactiveFloaterTransparency (LLFloater::updateTransparency,
//! indra/llui/llfloater.cpp), FSIMOpacity and FSImActiveOpacityOverride
//! (FSFloaterIMContainer::getCurrentTransparency): `level` has the rules,
//! simpler than the Legacy Viewer's, by the human's choice: Conversations
//! follows its own slider alone, not the lower of it and the inactive one,
//! and the Camera and Movement windows follow the other windows' sliders
//! (no CameraOpacity). A window's background and control faces fade while
//! its text and pictures stay solid. The bars' slider is Aurora's own: the
//! Legacy Viewer's bars are not translucent. Inactive defaults to 1, against
//! the Legacy Viewer's 0.95: Aurora's windows are glass already.
//!
//! The other surfaces over the world have their own: FSMenuBackgroundAlpha
//! (LLMenuGL::draw), ConsoleBackgroundOpacity (LLConsole::draw, 0.5 there;
//! here 0.75, what Aurora's chat cards always had), ChatBubbleOpacity
//! (LLHUDNameTag: the name tags too), FSMiniMapOpacity
//! (LLFloaterMap::getCurrentTransparency), OverridePieColors and
//! PieMenuOpacity (PieMenu::draw).
//!
//! The surfaces are told by their colour (`fade_after`) rather than one by
//! one: every window and bar draws its panes, fields and buttons in the
//! palette's surface colours, so what they draw needs no change. Hovered and
//! selected faces have colours of their own and stay readable.
//!
//! The choice is process-wide (`set`). High contrast ignores it: there the
//! world never shows through (`crate::high_contrast`).

use crate::theme::Palette;
use egui::Color32;
use egui::layers::ShapeIdx;
use std::sync::RwLock;

/// How long a window takes to fade when it gains or loses the focus.
const FOCUS_FADE: f32 = 0.15;

/// The defaults of the surfaces that are not simply the skin's: the look they
/// keep at these values.
pub const CONSOLE: f32 = 0.75;
pub const NAME_TAGS: f32 = 0.5;
pub const MINIMAP: f32 = 0.66;
/// PieMenuOpacity's default.
pub const PIE: f32 = 0.85;

/// The sliders, each 0 (only the text and pictures) to 1 (the skin's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opacity {
    pub active: f32,
    pub inactive: f32,
    pub conversations: f32,
    /// The Conversations window at `active` while it has the focus.
    pub conversations_active_override: bool,
    pub bars: f32,
    pub menus: f32,
    pub console: f32,
    pub name_tags: f32,
    pub minimap: f32,
    /// The pie menu's own opacity (None: the skin's).
    pub pie: Option<f32>,
}

impl Opacity {
    const FULL: Self = Self {
        active: 1.0,
        inactive: 1.0,
        conversations: 1.0,
        conversations_active_override: false,
        bars: 1.0,
        menus: 1.0,
        console: CONSOLE,
        name_tags: NAME_TAGS,
        minimap: MINIMAP,
        pie: None,
    };
}

static CURRENT: RwLock<Opacity> = RwLock::new(Opacity::FULL);

pub fn set(opacity: Opacity) {
    if let Ok(mut c) = CURRENT.write() {
        *c = opacity;
    }
}

/// `AURORA_DEMO_OPACITY=active:1,inactive:0.3,conversations:0.6,bars:0.5,menus:0.8,console:0.3,tags:0.2,minimap:0.4`
/// (demo mode only, any of them over `base`): this run's choice, never saved.
pub fn demo_override(base: Opacity) -> Opacity {
    std::env::var("AURORA_DEMO_OPACITY").map_or(base, |raw| parse(&raw, base))
}

fn parse(raw: &str, mut out: Opacity) -> Opacity {
    for part in raw.split(',') {
        let Some((name, value)) = part.split_once(':') else { continue };
        let Ok(value) = value.trim().parse::<f32>() else { continue };
        let slot = match name.trim() {
            "active" => &mut out.active,
            "inactive" => &mut out.inactive,
            "conversations" => &mut out.conversations,
            "bars" => &mut out.bars,
            "menus" => &mut out.menus,
            "console" => &mut out.console,
            "tags" => &mut out.name_tags,
            "minimap" => &mut out.minimap,
            _ => continue,
        };
        if value.is_finite() {
            *slot = value.clamp(0.0, 1.0);
        }
    }
    out
}

fn current() -> Opacity {
    if crate::high_contrast::current() {
        return Opacity::FULL;
    }
    CURRENT.read().map_or(Opacity::FULL, |c| *c)
}

/// The opacity of the bars.
pub fn bars() -> f32 {
    current().bars
}

pub fn menus() -> f32 {
    current().menus
}

pub fn console() -> f32 {
    current().console
}

pub fn name_tags() -> f32 {
    current().name_tags
}

pub fn minimap() -> f32 {
    current().minimap
}

pub fn pie() -> Option<f32> {
    current().pie
}

/// A surface whose slider has `base` for its usual look, at `opacity`: what
/// its tint is multiplied by, and how much of its blur shows (as `glass`).
pub fn relative(opacity: f32, base: f32) -> (f32, f32) {
    let tint = opacity / base;
    let blur = tint.min(1.0);
    (tint, blur * blur)
}

/// The kinds of windows with a rule of their own.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Plain,
    Conversations,
    /// A conversation or the contacts detached from Conversations.
    Detached,
}

fn kind(id: &str, chat: bool) -> Kind {
    match id {
        "conversations" => Kind::Conversations,
        "contacts" => Kind::Detached,
        _ if chat => Kind::Detached,
        _ => Kind::Plain,
    }
}

/// The level of a window. Conversations stays at its slider even focused,
/// unless told not to: what keeps it out of the way while you type. A
/// detached conversation is a window like the others while focused.
fn level(o: &Opacity, kind: Kind, focused: bool) -> f32 {
    let floater = if focused { o.active } else { o.inactive };
    match kind {
        Kind::Plain => floater,
        Kind::Conversations if focused && o.conversations_active_override => o.active,
        Kind::Conversations => o.conversations,
        Kind::Detached if focused => o.active,
        Kind::Detached => o.conversations,
    }
}

/// What happened to the window this frame, for the focus.
#[derive(Debug, Clone, Copy, Default)]
struct FocusEvents {
    /// It was not shown the frame before.
    opened: bool,
    /// Its field took the keyboard this frame (Enter on the chat bar, a
    /// shortcut), without a click.
    took_keyboard: bool,
    /// A press this frame, and the layer under it (None: the world), when
    /// that layer is not a menu or a dialog over a window.
    pressed: Option<Option<egui::Id>>,
}

/// The window that has the focus after `window`'s events (LLFloater::setFocus):
/// only one at a time. A window just opened or whose field takes the
/// keyboard gets it; a press gives it to the window under the pointer, or to
/// none on the world or a bar.
fn next_holder(holder: Option<egui::Id>, window: egui::Id, events: FocusEvents) -> Option<egui::Id> {
    if events.opened || events.took_keyboard {
        Some(window)
    } else if let Some(layer) = events.pressed {
        layer
    } else {
        holder
    }
}

/// The opacity of the window `id` this frame. Call once per shown window:
/// it follows the focus (`next_holder`).
pub fn window(ctx: &egui::Context, id: &str, chat: bool) -> f32 {
    let window = egui::Id::unique(id);
    let holder_id = egui::Id::unique("active_window");
    let frame = ctx.cumulative_frame_nr();
    let seen = window.with("opacity_seen");
    let had_keyboard = window.with("opacity_had_keyboard");
    let keyboard = super::widgets::floater_holds_keyboard(ctx, id);
    let (opened, took_keyboard) = ctx.data_mut(|d| {
        let opened = d.get_temp::<u64>(seen).is_none_or(|last| last + 1 < frame);
        let took_keyboard = keyboard && !d.get_temp::<bool>(had_keyboard).unwrap_or(false);
        d.insert_temp(seen, frame);
        d.insert_temp(had_keyboard, keyboard);
        (opened, took_keyboard)
    });
    let pressed = ctx
        .input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None })
        .map(|pos| ctx.layer_id_at(pos))
        // a menu or a dialog over a window leaves it active
        .filter(|layer| layer.is_none_or(|l| l.order <= egui::Order::Middle))
        .map(|layer| layer.map(|l| l.id));
    let events = FocusEvents {
        opened,
        took_keyboard,
        pressed,
    };
    let holder = ctx.data_mut(|d| {
        let holder = next_holder(d.get_temp::<Option<egui::Id>>(holder_id).flatten(), window, events);
        d.insert_temp(holder_id, holder);
        holder
    });
    ctx.animate_value_with_time(
        window.with("opacity"),
        level(&current(), kind(id, chat), holder == Some(window)),
        FOCUS_FADE,
    )
}

/// The fill of a window or bar frame: under glass the tint is drawn over the
/// blur (`glass`), so that the frame's own, under it, would stack up as soon
/// as the blur fades.
pub fn frame_fill(p: &Palette, fill: Color32, opacity: f32) -> Color32 {
    if opacity >= 1.0 {
        fill
    } else if p.panel_alpha < 255 {
        Color32::TRANSPARENT
    } else {
        fill.gamma_multiply(opacity)
    }
}

/// The glass of a surface at `opacity`: its blur (None: it adds nothing) and
/// the alpha of its tint. The blur fades faster than the tint: over a sharp
/// world, it is what hides the world most.
pub fn glass(tint_alpha: u8, opacity: f32) -> (Option<f32>, u8) {
    let blur = (opacity > 0.0).then_some(opacity * opacity);
    (blur, (f32::from(tint_alpha) * opacity).round() as u8)
}

/// Fade the surfaces of what `layer` holds after the shape `slot`.
pub fn fade_after(ctx: &egui::Context, layer: egui::LayerId, slot: ShapeIdx, p: &Palette, opacity: f32) {
    if opacity >= 1.0 {
        return;
    }
    let surfaces = [p.panel, p.bar, p.field, p.raised, p.navy];
    ctx.graphics_mut(|graphics| {
        let Some(list) = graphics.get_mut(layer) else { return };
        for index in slot.0 + 1..list.next_idx().0 {
            list.mutate_shape(ShapeIdx(index), |clipped| fade_shape(&mut clipped.shape, &surfaces, opacity));
        }
    });
}

pub(crate) fn fade_shape(shape: &mut egui::Shape, surfaces: &[Color32], opacity: f32) {
    let fade = |fill: &mut Color32| {
        if is_surface(*fill, surfaces) {
            *fill = fill.gamma_multiply(opacity);
        }
    };
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter_mut().for_each(|s| fade_shape(s, surfaces, opacity)),
        // (a rectangle with a brush is a picture)
        egui::Shape::Rect(rect) if rect.brush.is_none() => {
            if rect.blur_width > 0.0 {
                rect.fill = rect.fill.gamma_multiply(opacity);
            } else {
                fade(&mut rect.fill);
            }
        }
        egui::Shape::Circle(circle) => fade(&mut circle.fill),
        egui::Shape::Ellipse(ellipse) => fade(&mut ellipse.fill),
        egui::Shape::Path(path) => fade(&mut path.fill),
        _ => {}
    }
}

/// Whether `fill` is one of the `surfaces`, at any alpha (a side column is
/// `p.bar.gamma_multiply(0.75)`).
fn is_surface(fill: Color32, surfaces: &[Color32]) -> bool {
    surfaces.iter().any(|s| same_colour(fill, *s))
}

/// Whether `a` and `b` are the same colour whatever their alpha (one faded,
/// the other not), within what rounding to 8 bits loses at `a`'s alpha.
pub(crate) fn same_colour(a: Color32, b: Color32) -> bool {
    let alpha = a.a();
    if alpha == 0 {
        return false;
    }
    let margin = 2 + 128 / i32::from(alpha);
    let [ar, ag, ab, _] = a.to_srgba_unmultiplied();
    let [br, bg, bb, _] = b.to_srgba_unmultiplied();
    (i32::from(ar) - i32::from(br)).abs() <= margin
        && (i32::from(ag) - i32::from(bg)).abs() <= margin
        && (i32::from(ab) - i32::from(bb)).abs() <= margin
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Opacity = Opacity::FULL;

    #[test]
    fn the_demo_variable_changes_only_what_it_names_and_stays_in_range() {
        let out = parse("inactive:0.3, bars:7,active:x,other:0.1,nothing", FULL);
        assert_eq!(out, Opacity { inactive: 0.3, ..FULL });
        assert_eq!(parse("active:nan", FULL), FULL);
    }

    /// FSFloaterIMContainer::getCurrentTransparency, Conversations on its
    /// own slider alone.
    #[test]
    fn each_kind_of_window_follows_its_rule() {
        let o = Opacity {
            active: 0.9,
            inactive: 0.7,
            conversations: 0.8,
            ..FULL
        };
        assert_eq!(level(&o, Kind::Plain, true), 0.9);
        assert_eq!(level(&o, Kind::Plain, false), 0.7);
        assert_eq!(level(&o, Kind::Conversations, true), 0.8, "its slider, focused");
        assert_eq!(level(&o, Kind::Conversations, false), 0.8, "not the other windows' 0.7");
        let over = Opacity {
            conversations_active_override: true,
            ..o
        };
        assert_eq!(level(&over, Kind::Conversations, true), 0.9, "the override");
        assert_eq!(level(&over, Kind::Conversations, false), 0.8);
        assert_eq!(level(&o, Kind::Detached, true), 0.9);
        assert_eq!(level(&o, Kind::Detached, false), 0.8);
    }

    /// LLFloater::setFocus: one window has the focus at a time.
    #[test]
    fn one_window_has_the_focus_at_a_time() {
        let a = egui::Id::unique("a");
        let b = egui::Id::unique("b");
        let none = FocusEvents::default();
        assert_eq!(next_holder(Some(a), b, none), Some(a), "nothing happened");
        let opened = FocusEvents { opened: true, ..none };
        assert_eq!(next_holder(Some(a), b, opened), Some(b), "a window just opened");
        // a click on a then Enter on b's field: b alone, not both
        let typed = FocusEvents {
            took_keyboard: true,
            ..none
        };
        assert_eq!(next_holder(Some(a), b, typed), Some(b));
        let on_a = FocusEvents {
            pressed: Some(Some(a)),
            ..none
        };
        assert_eq!(next_holder(Some(b), b, on_a), Some(a), "a click on another window");
        let on_world = FocusEvents {
            pressed: Some(None),
            ..none
        };
        assert_eq!(next_holder(Some(b), b, on_world), None, "a click on the world");
    }

    #[test]
    fn the_windows_are_told_apart_by_their_ids() {
        assert_eq!(kind("conversations", true), Kind::Conversations);
        assert_eq!(kind("contacts", false), Kind::Detached);
        assert_eq!(kind("a_popped_out_chat", true), Kind::Detached);
        assert_eq!(kind(crate::ui::controls::CLASSIC_CAMERA, false), Kind::Plain);
        assert_eq!(kind("inventory", false), Kind::Plain);
    }

    #[test]
    fn a_surface_is_recognised_at_any_alpha_and_other_colours_are_not() {
        let raised = Color32::from_rgb(52, 48, 70); // theme-guard: content — a test's colour
        let accent = Color32::from_rgb(124, 58, 237); // theme-guard: content — a test's colour
        for alpha in [1.0, 0.75, 0.3, 0.1] {
            assert!(is_surface(raised.gamma_multiply(alpha), &[raised]), "{alpha}");
            assert!(!is_surface(accent.gamma_multiply(alpha), &[raised]), "{alpha}");
        }
        assert!(!is_surface(Color32::TRANSPARENT, &[Color32::TRANSPARENT]));
    }

    #[test]
    fn only_surfaces_fade_in_a_shape_nested_or_not() {
        let raised = Color32::from_rgb(52, 48, 70); // theme-guard: content — a test's colour
        let accent = Color32::from_rgb(124, 58, 237); // theme-guard: content — a test's colour
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(10.0));
        let mut shape = egui::Shape::Vec(vec![
            egui::Shape::rect_filled(rect, 0.0, raised),
            egui::Shape::rect_filled(rect, 0.0, accent),
        ]);
        fade_shape(&mut shape, &[raised], 0.5);
        let egui::Shape::Vec(shapes) = shape else { panic!("still a list") };
        let fill = |shape: &egui::Shape| match shape {
            egui::Shape::Rect(rect) => rect.fill,
            _ => panic!("a rectangle"),
        };
        assert_eq!(fill(&shapes[0]), raised.gamma_multiply(0.5));
        assert_eq!(fill(&shapes[1]), accent);
    }

    #[test]
    fn a_surface_keeps_its_look_at_its_base() {
        assert_eq!(relative(0.5, 0.5), (1.0, 1.0));
        assert_eq!(relative(1.0, 0.5), (2.0, 1.0), "denser, the blur already full");
        assert_eq!(relative(0.25, 0.5), (0.5, 0.25));
        assert_eq!(relative(0.0, 0.5), (0.0, 0.0));
    }

    #[test]
    fn glass_is_as_the_skin_at_one_and_nothing_at_zero() {
        assert_eq!(glass(230, 1.0), (Some(1.0), 230));
        assert_eq!(glass(230, 0.0), (None, 0));
        let (blur, tint) = glass(230, 0.5);
        assert_eq!(blur, Some(0.25));
        assert_eq!(tint, 115);
    }

    #[test]
    fn a_frame_keeps_its_fill_at_one_and_leaves_it_to_the_glass_below() {
        let fill = Color32::from_rgb(20, 20, 30); // theme-guard: content — a test's colour
        let glass = Palette {
            panel_alpha: 230,
            ..crate::theme::Theme::default().palette()
        };
        let opaque = Palette { panel_alpha: 255, ..glass };
        assert_eq!(frame_fill(&glass, fill, 1.0), fill);
        assert_eq!(frame_fill(&glass, fill, 0.5), Color32::TRANSPARENT);
        assert_eq!(frame_fill(&opaque, fill, 0.5), fill.gamma_multiply(0.5));
    }
}
