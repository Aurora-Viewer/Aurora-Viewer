//! RLVa privacy of the name tags and the floating texts, as the Legacy
//! Viewer's LLVOAvatar::idleUpdateNameTag and LLHUDText::setString.

use super::*;
use crate::world::bubble_chat::{BubbleChat, Utterance, Voice};
use std::collections::HashSet;
use uuid::Uuid;

fn demo_world() -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::tags::events()) {
        world.apply(event);
    }
    world.rlva_configure(true);
    world
}

fn restrict(world: &mut World, command: &str) {
    world.rlva_console_input(command);
    world.refresh_rlva_privacy();
}

/// A world object (not an avatar, not worn) and its id, given `text`.
fn texted(world: &mut World, text: &str) -> Uuid {
    let (idx, id) = world
        .objects
        .iter()
        .find(|(idx, o)| !o.is_avatar() && o.parent_id == 0 && world.hud_root(*idx).is_none())
        .map(|(idx, o)| (idx, o.full_id))
        .expect("the demo has objects");
    world.objects.get_mut(idx).expect("object").text = text.to_owned();
    id
}

/// A resident around us, with the display name RlvUtil::filterNames looks
/// for: a cached one (LLAvatarNameCache::get), given here when the demo has
/// none yet.
fn near(world: &mut World) -> (Uuid, String) {
    let me = world.agent_id;
    let id = crate::world::radar::sightings(world)
        .into_iter()
        .map(|s| s.id)
        .find(|id| *id != me)
        .expect("the demo has residents around");
    if world.social.avatar_names.raw(&id).is_none() {
        world.social.avatar_names.on_legacy(id, "Near", "Tester");
    }
    let name = world.social.avatar_names.raw(&id).expect("cached").display_name.clone();
    (id, name)
}

/// LLVOAvatar::idleUpdateNameTag (llvoavatar.cpp): render_name =
/// visible_chat || visible_typing || (fRlvShowAvTag && ...): a resident who
/// speaks shows their tag under @shownametags.
#[test]
fn a_bubble_shows_its_tag_under_shownametags() {
    let mut world = demo_world();
    let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let icons = super::super::icons::Icons::default();
    let proj = Projector {
        view_proj: Mat4::from_scale(Vec3::splat(1.0 / 512.0)),
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    };
    let eye = Vec3::new(128.0, 128.0, 5.0);
    let empty = HashMap::new();
    let now = Instant::now();
    let settings = crate::settings::Settings {
        name_tag_distance: 128.0,
        ..Default::default()
    };
    let mut visibility = NameTagVisibility::default();
    let mut voice_tags = super::super::voice_tag::VoiceTags::default();
    let mut frame = |world: &World, settings: &crate::settings::Settings, chat: Option<&BubbleChat>, time| {
        let mut hits = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                hits = draw(
                    ui.ctx(),
                    &p,
                    &icons,
                    world,
                    &scene,
                    &proj,
                    eye,
                    true,
                    AvatarTags::Shown,
                    settings,
                    &mut visibility,
                    &empty,
                    &settings.audio,
                    &mut voice_tags,
                    None,
                    None,
                    None,
                    chat.map(|chat| crate::ui::chat_bubbles::Frame {
                        chat,
                        typing: false,
                        own_typing: false,
                        opacity: 0.5,
                        now,
                    }),
                    None,
                );
            },
        );
        output.textures_delta.clear();
        hits.into_iter().map(|tag| tag.id).collect::<HashSet<_>>()
    };
    let all = frame(&world, &settings, None, 0.0);
    let mut chat = BubbleChat::default();
    for id in &all {
        chat.add(
            *id,
            Utterance {
                text: "Hello".into(),
                from: "Someone".into(),
                voice: Voice::Normal,
                said: now,
            },
        );
    }
    let others = |tags: HashSet<Uuid>, own: Uuid| tags.into_iter().filter(|id| *id != own).collect::<HashSet<_>>();
    let own = world.agent_id;
    // who shows a bubble: the tags left with every name tag switched off
    let tags_off = crate::settings::Settings {
        show_own_name_tag: false,
        show_friend_name_tags: false,
        show_other_name_tags: false,
        ..settings.clone()
    };
    let speaking = others(frame(&world, &tags_off, Some(&chat), 1.0), own);
    assert!(!speaking.is_empty(), "fixture: residents near enough for a bubble");
    assert!(speaking.len() < others(all, own).len(), "fixture: residents too far for a bubble");
    restrict(&mut world, "@shownametags=n");
    assert_eq!(
        frame(&world, &settings, None, 2.0),
        HashSet::from([own]),
        "the tags of the silent ones hidden"
    );
    assert_eq!(
        others(frame(&world, &settings, Some(&chat), 3.0), own),
        speaking,
        "a resident who speaks keeps their tag, the others not"
    );
}

/// RlvUtil::filterNames (rlvcommon.cpp) ends with filterMentions: in a
/// floating text a hidden resident's mention link becomes "@" and their
/// stand-in.
#[test]
fn a_floating_text_mention_of_a_hidden_resident_is_frozen() {
    let mut world = demo_world();
    let (resident, _) = near(&mut world);
    let text = format!("ask secondlife:///app/agent/{resident}/mention");
    let object = texted(&mut world, &text);
    assert_eq!(hover_text(&world, object, false, &text), text);
    restrict(&mut world, "@shownames=n");
    let shown = hover_text(&world, object, false, &text);
    assert!(!shown.contains("/mention"), "{shown}");
    assert!(shown.starts_with("ask @"), "{shown}");
}

/// LLHUDText::setString filters a text when it comes, and
/// LLHUDText::refreshAllObjectText again only when a restriction it follows
/// comes or goes: another rule changes nothing, a new text is filtered.
#[test]
fn a_floating_text_is_filtered_when_it_comes_and_when_a_followed_restriction_toggles() {
    let mut world = demo_world();
    let (_, name) = near(&mut world);
    let text = format!("Hello {name}");
    let object = texted(&mut world, &text);
    assert_eq!(hover_text(&world, object, false, &text), text);
    // the names to hide changed, no followed restriction did: the text
    // keeps what it showed
    world.rlva_hover_privacy = world.rlva_text_filter(true);
    assert_eq!(hover_text(&world, object, false, &text), text, "filtered only when it came");
    restrict(&mut world, "@fly=n");
    world.rlva_hover_privacy = world.rlva_text_filter(true);
    assert_eq!(hover_text(&world, object, false, &text), text, "@fly is not followed");
    // a new text is filtered as it comes
    let other = format!("{text}!");
    assert!(!hover_text(&world, object, false, &other).contains(&name));
    // @shownearby comes: every text filtered again
    restrict(&mut world, "@shownearby=n");
    assert!(!hover_text(&world, object, false, &text).contains(&name));
    restrict(&mut world, "@shownearby=y");
    assert_eq!(hover_text(&world, object, false, &text), text, "and again when it goes");
}

/// The panels that name an object by its floating text (the photo
/// isolation, the build replace tool) show what the text shows: nothing
/// under @showhovertextworld (RlvActions::canShowHoverText), the names
/// hidden under @shownames.
#[test]
fn an_object_named_by_its_floating_text_follows_the_hover_text_restrictions() {
    let mut world = demo_world();
    let (_, name) = near(&mut world);
    let object = texted(&mut world, &format!("{name} stall\nsecond line"));
    let tool = crate::build::BuildTool::default();
    assert_eq!(crate::build::replace::object_name(&tool, &world, object), format!("{name} stall"));
    restrict(&mut world, "@shownames=n");
    let shown = crate::build::replace::object_name(&tool, &world, object);
    assert!(!shown.contains(&name), "{shown}");
    restrict(&mut world, "@showhovertextworld=n");
    assert_eq!(hover_text_title(&world, object), None);
    assert_eq!(
        crate::build::replace::object_name(&tool, &world, object),
        aurora_i18n::tr!("replace-unnamed")
    );
}
