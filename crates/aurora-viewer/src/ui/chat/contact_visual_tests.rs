use super::*;
use crate::world::ChatLine;
use crate::world::contact_sets::Visuals;

#[test]
fn contact_visuals_chat_colours_are_independent_and_reset_to_original() {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let id = uuid::Uuid::from_u128(10);
    let p = crate::theme::Theme::default().palette();
    let family = egui::FontFamily::Proportional;
    let mut line = ChatLine {
        time: std::time::SystemTime::now(),
        from: "Example".into(),
        text: "Hello".into(),
        kind: ChatKind::Local(ChatType::Normal),
        source: id,
        origin: Origin::Live,
    };
    let job = |world: &World, line: &ChatLine, look| line_job(line, &p, world, &mut HashSet::new(), 540.0, 12.0, false, look).job;
    let baseline = job(&world, &line, JobLook::Pill(&family));
    let sets = &mut world.contact_sets;
    sets.add_set("Names");
    sets.add_set("Messages");
    sets.add_to_set(&[id], "Names", |_| false);
    sets.add_to_set(&[id], "Messages", |_| false);
    sets.set_color("Names", [0.6, 0.2, 0.8, 1.0]);
    sets.set_color("Messages", [0.2, 0.8, 0.5, 1.0]);
    sets.set_visuals(
        "Names",
        Visuals {
            chat_name: true,
            ui_name: true,
            ..Default::default()
        },
    );
    sets.set_visuals(
        "Messages",
        Visuals {
            chat_text: true,
            minimap: true,
            ..Default::default()
        },
    );
    let name = crate::ui::colors::set_color([0.6, 0.2, 0.8, 1.0]);
    let text = crate::ui::colors::set_color([0.2, 0.8, 0.5, 1.0]);
    for kind in [ChatKind::Local(ChatType::Normal), ChatKind::Im] {
        line.kind = kind;
        for look in [JobLook::Line, JobLook::Pill(&family)] {
            let rendered = job(&world, &line, look);
            assert_eq!(rendered.sections.first().unwrap().format.color, name);
            assert_eq!(rendered.sections.last().unwrap().format.color, text);
        }
        assert_eq!(crate::ui::colors::chat_color(&line, &world), text);
    }
    assert_eq!(crate::ui::colors::resident_name(&world, &id, p.ink), name);
    assert_eq!(crate::ui::colors::map_color(&world, &id, "Example"), text);
    let classic = crate::ui::colors::tag_color(&world, &id, "Example", false);
    line.kind = ChatKind::Object(ChatType::Normal);
    assert_eq!(crate::ui::colors::contact_chat_color(&line, &world), None);
    assert_eq!(crate::ui::colors::contact_sender_color(&line, &world), None);
    world.contact_sets.set_visuals("Names", Visuals::default());
    world.contact_sets.set_visuals("Messages", Visuals::default());
    assert_eq!(crate::ui::colors::tag_color(&world, &id, "Example", false), classic);
    line.kind = ChatKind::Local(ChatType::Normal);
    let reset = job(&world, &line, JobLook::Pill(&family));
    assert_eq!(reset.text, baseline.text);
    assert_eq!(
        reset.sections.iter().map(|s| s.format.color).collect::<Vec<_>>(),
        baseline.sections.iter().map(|s| s.format.color).collect::<Vec<_>>()
    );
}
