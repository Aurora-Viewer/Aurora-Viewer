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
