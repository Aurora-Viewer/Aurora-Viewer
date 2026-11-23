//! The nearby list's menu (menu_fs_radar.xml): the script information and
//! the display choices of the avatar's menu in the world, drawn in real
//! egui frames.

use super::*;
use crate::ui::widgets::elide_tests::Frames;

const NOVA: Uuid = Uuid::from_u128(0x7);

fn world() -> World {
    let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    w.agent_id = Uuid::from_u128(1);
    w
}

/// One frame of the menu, with `events`; its texts and what it asked.
fn frame(f: &mut Frames, p: &Palette, w: &World, events: Vec<egui::Event>) -> (Vec<crate::ui::widgets::elide_tests::Text>, Vec<CtxAction>) {
    let texts = f.run(events, |ui| avatar_list_menu(ui, p, w, NOVA, AvatarList::Nearby));
    (texts, take_requests(&f.ctx))
}

#[test]
fn the_nearby_menu_offers_script_information_and_display_choices() {
    let mut f = Frames::new();
    let theme = crate::theme::Theme::default();
    theme.apply(&f.ctx, 1.0);
    let p = theme.palette();
    let w = world();
    let (texts, _) = frame(&mut f, &p, &w, Vec::new());
    let find = |label: &str| texts.iter().find(|t| t.source == label).map(|t| t.rect.center());
    assert!(find(aurora_i18n::tr!("ui-context-display-options")).is_some());
    let at = find(aurora_i18n::tr!("ui-context-script-information")).expect("script information row");
    let mut asked = Vec::new();
    for pressed in [true, false] {
        let click = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        asked.extend(frame(&mut f, &p, &w, click).1);
    }
    assert!(matches!(&asked[..], [CtxAction::ScriptInfo(id)] if *id == NOVA), "{asked:?}");
}

/// The display choices are the in-world menu's, in its order.
#[test]
fn the_display_choices_are_the_world_menus() {
    let modes: Vec<u8> = render_choices().iter().map(|(m, _, _)| *m).collect();
    assert_eq!(modes, [2, 1, 0]);
}

/// Hovers the "Markers" row until its submenu shows `entry`, then clicks it:
/// what the menu asked.
fn mark_choice(body: &mut dyn FnMut(&mut egui::Ui), entry: &str) -> Vec<CtxAction> {
    use crate::ui::widgets::elide_tests::painted;
    let mut f = Frames::new();
    f.run(Vec::new(), &mut *body);
    let texts = f.run(Vec::new(), &mut *body);
    let heading = painted(&texts, aurora_i18n::tr!("ui-minimap-markers")).rect.center();
    let mut at = None;
    for _ in 0..30 {
        let texts = f.run(vec![egui::Event::PointerMoved(heading)], &mut *body);
        at = texts.iter().find(|t| t.source == entry).map(|t| t.rect.center());
        if at.is_some() {
            break;
        }
    }
    let at = at.expect("the Markers submenu opens on hover");
    let _ = take_requests(&f.ctx);
    for pressed in [true, false] {
        let click = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        f.run(click, &mut *body);
    }
    take_requests(&f.ctx)
}

#[test]
fn the_radar_menus_mark_one_resident_or_several() {
    let theme = crate::theme::Theme::default();
    let p = theme.palette();
    let mut w = world();
    let red = crate::world::avatar_marks::LEGACY_COLORS[0].1;
    // one resident (menu_fs_radar.xml's MarkAvatar)
    let asked = mark_choice(
        &mut |ui| avatar_list_menu(ui, &p, &w, NOVA, AvatarList::Nearby),
        aurora_i18n::tr!("ui-minimap-red"),
    );
    assert!(
        matches!(&asked[..], [CtxAction::MarkAvatars(ids, c)] if ids == &[NOVA] && *c == red),
        "{asked:?}"
    );
    let asked = mark_choice(
        &mut |ui| avatar_list_menu(ui, &p, &w, NOVA, AvatarList::Nearby),
        aurora_i18n::tr!("ui-minimap-custom-marker"),
    );
    assert!(
        matches!(&asked[..], [CtxAction::MarkCustom { ids, source: AvatarList::Nearby }] if ids == &[NOVA]),
        "{asked:?}"
    );
    // several (menu_fs_radar_multiselect.xml)
    let several = [NOVA, Uuid::from_u128(0x8)];
    let blue = crate::world::avatar_marks::LEGACY_COLORS[2].1;
    let asked = mark_choice(&mut |ui| avatars_menu(ui, &p, &w, &several), aurora_i18n::tr!("ui-minimap-blue"));
    assert!(
        matches!(&asked[..], [CtxAction::MarkAvatars(ids, c)] if ids == &several && *c == blue),
        "{asked:?}"
    );
    // the clear entries once someone is marked
    w.avatar_marks.set(&several, blue);
    let asked = mark_choice(
        &mut |ui| avatars_menu(ui, &p, &w, &several),
        aurora_i18n::tr!("ui-minimap-clear-marker"),
    );
    assert!(matches!(&asked[..], [CtxAction::ClearMarks(ids)] if ids == &several), "{asked:?}");
    let asked = mark_choice(
        &mut |ui| avatar_list_menu(ui, &p, &w, NOVA, AvatarList::Nearby),
        aurora_i18n::tr!("ui-minimap-clear-all-markers"),
    );
    assert!(matches!(&asked[..], [CtxAction::ClearAllMarks]), "{asked:?}");
}
