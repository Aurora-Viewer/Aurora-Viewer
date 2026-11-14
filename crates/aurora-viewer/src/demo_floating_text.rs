//! Three separately coloured llSetText prims above a HUD toolbar (#1938).
//! The baseline guides use the reference's 12-unit padding and y=1 draw.

use super::*;

pub const ROOT: u32 = 121_000;
pub const TEXTS: [u32; 3] = [ROOT + 1, ROOT + 2, ROOT + 3];

pub fn scenario() -> bool {
    std::env::var_os("AURORA_DEMO_FLOATING_TEXT").is_some()
}

/// `background`: every floating text on its background
/// (FSHudTextShowBackground 2), fading from 16 m instead of 8.
pub fn apply_preferences(settings: &mut crate::settings::Settings) {
    if std::env::var("AURORA_DEMO_FLOATING_TEXT").is_ok_and(|mode| mode == "background") {
        settings.floating_text_background = crate::settings::FloatingTextBackground::All;
        settings.floating_text_fade_distance = 16.0;
    }
}

pub fn events() -> Vec<NetEvent> {
    let mode = std::env::var("AURORA_DEMO_FLOATING_TEXT").unwrap_or_default();
    if mode == "world" || mode == "background" {
        let start = Vec3::new(132.0, 126.0, 0.0);
        let ahead = Vec3::new(0.8, 0.4, 0.0).normalize();
        let p = start + ahead * 5.0 - Vec3::new(ahead.y, -ahead.x, 0.0) * 3.2;
        let cube = prim(
            ROOT,
            Vec3::new(p.x, p.y, floor_at(p.x, p.y) + 0.25),
            Quat::IDENTITY,
            Vec3::splat(0.5),
            shape(
                aurora_prim::params::LL_PCODE_PATH_LINE,
                aurora_prim::params::LL_PCODE_PROFILE_SQUARE,
                100,
                0,
                0,
            ),
            te([0.3, 0.2, 0.8, 1.0], 1, true, 0.0),
            ExtraParams::default(),
            "First line\nSecond line\nThird line",
        );
        let mut objects = vec![cube];
        if mode == "background" {
            // a dark text farther away: a light background, faded with it
            let p = start + ahead * 13.0 + Vec3::new(ahead.y, -ahead.x, 0.0) * 2.0;
            let mut far = objects[0].clone();
            far.local_id = ROOT + 20;
            far.full_id = u(u128::from(far.local_id));
            far.position = Vec3::new(p.x, p.y, floor_at(p.x, p.y) + 0.25);
            far.text = "Dark text, farther away".to_owned();
            far.text_color = [20, 24, 70, 255];
            objects.push(far);
        }
        return vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }];
    }
    hud_events()
}

pub fn hud_events() -> Vec<NetEvent> {
    let mut root = hud::object(31, false);
    root.local_id = ROOT;
    root.full_id = u(u128::from(ROOT));
    root.name_values = format!("AttachItemID STRING RW SV {}", root.full_id);
    root.position = Vec3::new(0.0, 0.0, 0.08);
    root.scale = Vec3::new(0.012, 0.50, 0.14);
    root.text.clear();
    root.texture_entry = Some(te([0.20, 0.16, 0.09, 0.8], 1, true, 0.0));
    let mut objects = vec![root];
    for (i, (text, color)) in [
        ("====== 1-2 / 2 avatars ====== (2)", [255, 235, 110, 255]),
        ("Visitor Gray (visitor.gray) - 583m - 267us", [0, 150, 255, 255]),
        ("Visitor Two (visitor.two) - 0m - 629us", [0, 230, 255, 255]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut child = hud::object(31, false);
        child.local_id = TEXTS[i];
        child.full_id = u(u128::from(child.local_id));
        child.parent_id = ROOT;
        child.state = 0;
        child.name_values.clear();
        child.position = Vec3::new(-0.02, 0.0, 0.02 - i as f32 * 0.025);
        child.scale = Vec3::new(0.002, 0.48, 0.002);
        child.text = text.to_owned();
        child.text_color = color;
        child.texture_entry = Some(te([0.0; 4], 1, true, 0.0));
        objects.push(child);
    }
    let mut toolbar = objects[1].clone();
    toolbar.local_id = ROOT + 4;
    toolbar.full_id = u(u128::from(toolbar.local_id));
    toolbar.position = Vec3::new(-0.01, 0.0, -0.05);
    toolbar.scale = Vec3::new(0.016, 0.50, 0.04);
    toolbar.text.clear();
    toolbar.texture_entry = Some(te([0.0, 0.0, 0.0, 1.0], 1, true, 0.0));
    objects.push(toolbar);
    for i in 0..12 {
        let mut button = objects[4].clone();
        button.local_id = ROOT + 5 + i;
        button.full_id = u(u128::from(button.local_id));
        button.position = Vec3::new(-0.025, 0.22 - i as f32 * 0.04, -0.05);
        button.scale = Vec3::new(0.008, 0.018, 0.018);
        button.texture_entry = Some(te([0.0, 0.65, 0.95, 1.0], 1, true, 0.0));
        objects.push(button);
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

pub fn guides(ctx: &egui::Context, world: &crate::world::World, scene: &crate::scene::Scene) {
    if !std::env::var("AURORA_DEMO_FLOATING_TEXT").is_ok_and(|mode| mode == "guides") {
        return;
    }
    let Some(view) = scene.lists.hud_view else { return };
    let ppp = ctx.pixels_per_point();
    let painter = ctx.layer_painter(egui::LayerId::background());
    for id in TEXTS {
        let Some(idx) = world.objects.index_of_uuid(&u(u128::from(id))) else {
            continue;
        };
        let Some((pos, _, _)) = crate::scene::Scene::object_transform(world, idx, std::time::Instant::now(), 0) else {
            continue;
        };
        let [x, y] = view.pixel(pos + Vec3::Z * 0.0012);
        // Independent of font layout: LLHUDText's padding remains after
        // all row advances, then hud_render_text adds one UI unit.
        let baseline = y / ppp - 12.0 * view.zoom - 1.0;
        let half = view.unit() * 0.24 / ppp;
        let stroke = egui::Stroke::new(1.0 / ppp, egui::Color32::from_rgb(110, 235, 140)); // theme-guard: content — reference baseline guide.
        for sign in [-1.0, 1.0] {
            let end = x / ppp + sign * half;
            painter.line_segment([egui::pos2(end, baseline), egui::pos2(end - sign * 18.0, baseline)], stroke);
        }
    }
}
