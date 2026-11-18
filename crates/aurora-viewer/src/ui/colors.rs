//! Configurable colors (Firestorm color preferences): name tags, minimap
//! markers and chat rings, chat messages, senders and mentions. The logic
//! follows LLViewerChat::getChatColor and LLVOAvatar::getNameTagColor;
//! blocked residents come after friends (LGGContactSets::colorize).

use crate::settings::{ColorSettings, Rgba};
use crate::world::{ChatKind, ChatLine, World};
use aurora_net::ChatType;
use egui::Color32;
use uuid::Uuid;

static GLOBAL: std::sync::RwLock<Option<ColorSettings>> = std::sync::RwLock::new(None);

/// Install the current colors (call when settings change).
pub fn set(c: &ColorSettings) {
    if let Ok(mut g) = GLOBAL.write() {
        *g = Some(*c);
    }
}

pub fn get() -> ColorSettings {
    GLOBAL.read().ok().and_then(|g| *g).unwrap_or_default()
}

/// A configured color as drawn: daltonised for the colour blindness chosen
/// in the preferences (`crate::colorblind::adapt`).
pub fn c(v: Rgba) -> Color32 {
    crate::colorblind::adapt(
        Color32::from_rgba_unmultiplied(v[0], v[1], v[2], v[3]),
        crate::colorblind::current(),
    )
}

/// A configured colour for text on the windows (chat lines, names, links):
/// `c`, then moved in lightness to read on a background chosen in
/// Preferences › Appearance (`crate::interface_colors::on_panel`). The
/// world's and the maps' colours keep `c`.
pub fn text(v: Rgba) -> Color32 {
    crate::interface_colors::on_panel(c(v))
}

/// Contact-set colours are stored in linear RGBA in the Legacy-compatible
/// file. Apply the existing colour-blind adaptation after conversion.
pub fn set_color(v: [f32; 4]) -> Color32 {
    crate::colorblind::adapt(
        Color32::from(egui::Rgba::from_rgba_unmultiplied(v[0], v[1], v[2], v[3])),
        crate::colorblind::current(),
    )
}

// The colours a contact set gives go through `World::rlva_contact_appearance`:
// none for a resident whose name RLVa hides
// (LGGContactSets::hasFriendColorThatShouldShow, lggcontactsets.cpp).

pub fn resident_name(world: &World, id: &Uuid, fallback: Color32) -> Color32 {
    world
        .rlva_contact_appearance(id)
        .ui_name
        .map(|v| crate::interface_colors::on_panel(set_color(v)))
        .unwrap_or(fallback)
}

pub fn contact_chat_color(line: &ChatLine, world: &World) -> Option<Color32> {
    matches!(line.kind, ChatKind::Local(_) | ChatKind::Im | ChatKind::Own(_))
        .then(|| world.rlva_contact_appearance(&line.source).chat_text.map(set_color))
        .flatten()
        .map(crate::interface_colors::on_panel)
}

pub fn contact_sender_color(line: &ChatLine, world: &World) -> Option<Color32> {
    matches!(line.kind, ChatKind::Local(_) | ChatKind::Im | ChatKind::Own(_))
        .then(|| {
            let a = world.rlva_contact_appearance(&line.source);
            a.chat_name.or(a.ui_name).map(set_color)
        })
        .flatten()
        .map(crate::interface_colors::on_panel)
}

/// Linden Lab staff ("First Linden" / "first.linden").
pub fn is_linden(name: &str) -> bool {
    let n = name.trim();
    n.ends_with(" Linden") || n.to_lowercase().ends_with(".linden")
}

/// Linden by the legacy name of the cached avatar name when known (a
/// display name can say anything), else by the name given.
fn is_linden_avatar(world: &World, id: &Uuid, name: &str) -> bool {
    let names = &world.social.avatar_names;
    match names.get(id) {
        Some(n) => is_linden(&n.user_name_for_display(&names.options)),
        None => is_linden(name),
    }
}

fn is_friend(world: &World, id: &Uuid) -> bool {
    world.social.friends.iter().any(|f| f.id == *id)
}

/// Who a resident is to us, as far as a colour may tell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Relation {
    own: bool,
    linden: bool,
    friend: bool,
    blocked: bool,
}

/// The relation the colours of `id` show. `linden`: what its name says;
/// `im`: for a line of a private conversation. A resident whose name RLVa
/// hides is nobody in particular, or a colour would name them:
/// LGGContactSets::colorize (lggcontactsets.cpp) looks at nothing under
/// `rlv_shownames`, and keeps the friend colour in IMs alone.
fn relation(world: &World, id: &Uuid, linden: bool, im: bool) -> Relation {
    let named = world.rlva_can_name(*id);
    Relation {
        own: *id == world.agent_id,
        linden: named && linden,
        friend: (named || im) && is_friend(world, id),
        blocked: named && world.is_avatar_blocked(id),
    }
}

/// The keyword colour of a line that holds a keyword while "Change the
/// colour" is on (Preferences › Chat › Keyword alerts). Ours and the debug
/// channel's never hold one (chatContainsKeyword); a conversation's lines
/// are IMs, the rest nearby chat.
pub fn keyword_color(line: &ChatLine) -> Option<Color32> {
    if matches!(
        line.kind,
        ChatKind::Own(_) | ChatKind::Object(ChatType::Debug) | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_)
    ) {
        return None;
    }
    crate::keywords::color(&line.from, &line.text, line.kind != ChatKind::Im).map(text)
}

/// Text color of a chat line (LLViewerChat::getChatColor): the keyword
/// colour for a line that holds a keyword, else the sender's.
pub fn chat_color(line: &ChatLine, world: &World) -> Color32 {
    if let Some(color) = contact_chat_color(line, world) {
        return color;
    }
    if let Some(color) = keyword_color(line) {
        return color;
    }
    let k = get();
    let v = match line.kind {
        ChatKind::Own(_) => k.chat_mine,
        ChatKind::System | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_) => k.chat_system,
        ChatKind::ObjectIm => k.chat_object_im,
        ChatKind::Object(t) => match t {
            ChatType::Debug => k.chat_errors,
            ChatType::OwnerSay => k.chat_owner,
            ChatType::Other(9) => k.chat_direct,
            _ => k.chat_objects,
        },
        ChatKind::Local(_) | ChatKind::Im => {
            let linden = is_linden_avatar(world, &line.source, &line.from);
            let r = relation(world, &line.source, linden, line.kind == ChatKind::Im);
            if r.linden {
                k.chat_linden
            } else if r.friend {
                k.chat_friends
            } else if r.blocked {
                k.chat_muted
            } else {
                k.chat_others
            }
        }
    };
    text(v)
}

/// How opaque a nearby line is drawn: Preferences › Chat's factor for a
/// line said beyond the say range, whole otherwise. LLViewerChat::getChatColor
/// (indra/newview/llviewerchat.cpp) gives that factor as the line's alpha,
/// which FSChatHistory::appendMessage and the console apply to the whole
/// line, the sender's name included; so does Aurora, in every chat style.
pub fn beyond_range_alpha(line: &ChatLine) -> f32 {
    line_alpha(line, get().chat_beyond_range)
}

fn line_alpha(line: &ChatLine, factor: f32) -> f32 {
    if line.beyond_range { factor.clamp(0.0, 1.0) } else { 1.0 }
}

/// Text of a script error (ScriptErrorColor), the Aurora red by default:
/// the same in every style of the chat (#1263).
pub fn script_error_color() -> Color32 {
    text(get().chat_errors)
}

/// Color of a sender's name in the conversation headers.
pub fn sender_color(line: &ChatLine) -> Color32 {
    let k = get();
    match line.kind {
        ChatKind::Object(_) | ChatKind::ObjectIm => text(k.sender_object),
        _ => text(k.sender_avatar),
    }
}

/// Color of a distance to us: within the chat, within a shout, farther.
/// The radar's ranges and names, and the name tags' distance
/// (FSTagShowDistance, FSTagShowDistanceColors in LLVOAvatar::idleUpdateNameTagText,
/// indra/newview/llvoavatar.cpp). The Legacy Viewer's tags draw the bands
/// green, yellow and red (NameTagChatDistanceColor…); Aurora keeps the
/// radar's theme colors for both, so that a distance reads the same
/// everywhere. Its whisper band has the chat's color there too.
pub fn range_color(p: &crate::theme::Palette, range: Option<f32>) -> Color32 {
    range_color_in(p, range, crate::world::radar::chat_ranges())
}

pub fn range_color_in(p: &crate::theme::Palette, range: Option<f32>, r: crate::world::radar::ChatRanges) -> Color32 {
    match range {
        Some(d) if d <= r.say => p.success,
        Some(d) if d <= r.shout => p.ink,
        _ => p.muted,
    }
}

/// Whether `tag_color` gives a resident a color of its relation (Linden,
/// friend, blocked) rather than the match / mismatch one: the Legacy
/// Viewer's tags colored by distance keep such a color
/// (`special_color_override` in LLVOAvatar::idleUpdateNameTagText).
/// A marked resident keeps their mark's color too, their name hidden by
/// RLVa or not: `LLNetMap::hasAvatarMarkColor` is the one term of
/// `special_color_override` that tests no name.
pub fn relation_tag(world: &World, id: &Uuid, name: &str) -> bool {
    let r = relation(world, id, is_linden(name), false);
    world.avatar_marks.contains(id) || (get().relation_names && (r.linden || r.friend || r.blocked))
}

/// The colour of a resident's mark (`world::avatar_marks`) as the maps draw
/// it, daltonised like the other configured colours. None for a resident
/// whose name RLVa hides (`World::rlva_shown_mark`); the name tag keeps it
/// (`tag_color`).
pub fn mark_color(world: &World, id: &Uuid) -> Option<Color32> {
    world.rlva_shown_mark(id).map(|[r, g, b]| c([r, g, b, 255]))
}

/// Name tag color of an avatar (own, Linden, friend while the names are
/// colored by relation, else match / mismatch); a mark's color over all of
/// them, for a resident whose name RLVa hides as for any other
/// (LLVOAvatar::getNameTagColor, indra/newview/llvoavatar.cpp, ends with
/// LLNetMap::getAvatarMarkColor and tests no name: the relation and the
/// contact set are dropped before it, the mark is not).
pub fn tag_color(world: &World, id: &Uuid, name: &str, custom_display_name: bool) -> Color32 {
    if let Some([r, g, b]) = world.avatar_marks.get(id) {
        return text([r, g, b, 255]);
    }
    let k = get();
    let r = relation(world, id, is_linden(name), false);
    let by_relation = k.relation_names;
    let v = if by_relation && r.own {
        k.tag_me
    } else if by_relation && r.linden {
        k.tag_linden
    } else if by_relation && r.friend {
        k.tag_friend
    } else if by_relation && r.blocked {
        k.tag_muted
    } else if custom_display_name {
        k.tag_mismatch
    } else {
        k.tag_match
    };
    // on plates drawn in the windows' colours
    text(v)
}

/// Minimap marker color of an avatar: its mark first, then its contact
/// set's, then its relation's (LLNetMap::getAvatarColor: the mark is applied
/// last, over the contact sets). The others' colour for a resident whose
/// name RLVa hides, whoever it is and whatever mark it has (LLNetMap::draw,
/// llnetmap.cpp: `canShowName(…) ? color : map_avatar_color`).
pub fn map_color(world: &World, id: &Uuid, name: &str) -> Color32 {
    if let Some(color) = mark_color(world, id) {
        return color;
    }
    if let Some(color) = world.rlva_contact_appearance(id).minimap {
        return set_color(color);
    }
    let k = get();
    let r = relation(world, id, is_linden_avatar(world, id, name), false);
    let v = if r.own {
        k.map_me
    } else if r.linden {
        k.map_linden
    } else if r.friend {
        k.map_friend
    } else if r.blocked {
        k.map_muted
    } else {
        k.map_other
    };
    c(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Origin;

    fn line(kind: ChatKind, beyond_range: bool) -> ChatLine {
        ChatLine {
            time: std::time::SystemTime::UNIX_EPOCH,
            from: "Jane".into(),
            text: "hello".into(),
            kind,
            source: Uuid::from_u128(7),
            origin: Origin::Live,
            blocked: false,
            beyond_range,
        }
    }

    #[test]
    fn only_a_line_from_beyond_the_say_range_takes_the_factor() {
        // the Legacy default, for a resident's and an object's line
        assert_eq!(line_alpha(&line(ChatKind::Local(ChatType::Shout), true), 0.8), 0.8);
        assert_eq!(line_alpha(&line(ChatKind::Object(ChatType::Shout), true), 0.8), 0.8);
        // within range, or an IM (no position): whole
        assert_eq!(line_alpha(&line(ChatKind::Local(ChatType::Normal), false), 0.8), 1.0);
        assert_eq!(line_alpha(&line(ChatKind::Im, false), 0.3), 1.0);
        // the slider's ends, and a value out of them from a hand-edited file
        assert_eq!(line_alpha(&line(ChatKind::Local(ChatType::Normal), true), 1.0), 1.0);
        assert_eq!(line_alpha(&line(ChatKind::Local(ChatType::Normal), true), 0.0), 0.0);
        assert_eq!(line_alpha(&line(ChatKind::Local(ChatType::Normal), true), 1.7), 1.0);
        assert_eq!(line_alpha(&line(ChatKind::Local(ChatType::Normal), true), -0.5), 0.0);
    }

    #[test]
    fn the_factor_defaults_to_the_legacy_viewers() {
        assert_eq!(ColorSettings::default().chat_beyond_range, 0.8);
    }

    use crate::world::contact_sets::Visuals;

    const ME: Uuid = Uuid::from_u128(1);
    const FRIEND: Uuid = Uuid::from_u128(7);

    fn rlva(world: &mut World, command: &str) {
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Colour fixture".into(),
            source_id: Uuid::from_u128(4),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: command.into(),
        }));
    }

    /// A friend in a contact set that colours everything.
    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = ME;
        world.social.friends.push(crate::world::social::Friend {
            id: FRIEND,
            online: true,
            rights_given: 0,
            rights_has: 0,
        });
        let sets = &mut world.contact_sets;
        sets.add_set("Team");
        sets.add_to_set(&[FRIEND], "Team", |_| false);
        sets.set_color("Team", [0.6, 0.2, 0.8, 1.0]);
        sets.set_visuals(
            "Team",
            Visuals {
                ui_name: true,
                chat_name: true,
                chat_text: true,
                minimap: true,
                ..Default::default()
            },
        );
        world.rlva_configure(true);
        world
    }

    fn rlva_line(kind: ChatKind) -> ChatLine {
        ChatLine {
            time: std::time::SystemTime::now(),
            from: "Teagan Linden".into(),
            text: "Hello".into(),
            kind,
            source: FRIEND,
            origin: crate::world::Origin::Live,
            blocked: false,
            beyond_range: false,
        }
    }

    #[test]
    fn rlva_hidden_names_show_no_relation_but_a_friend_in_an_im() {
        let mut world = world();
        let friend = Relation {
            own: false,
            linden: true,
            friend: true,
            blocked: false,
        };
        assert_eq!(relation(&world, &FRIEND, true, false), friend);
        rlva(&mut world, "@shownames=n");
        let nobody = Relation {
            own: false,
            linden: false,
            friend: false,
            blocked: false,
        };
        assert_eq!(relation(&world, &FRIEND, true, false), nobody);
        assert_eq!(
            relation(&world, &FRIEND, true, true),
            Relation { friend: true, ..nobody },
            "the Legacy Viewer keeps the friend colour of IMs"
        );
        assert!(relation(&world, &ME, false, false).own, "our own name is never hidden");
        rlva(&mut world, &format!("@shownames:{FRIEND}=add"));
        assert_eq!(relation(&world, &FRIEND, true, false), friend, "an exception is named again");
    }

    #[test]
    fn rlva_hidden_names_lose_their_contact_set_and_map_colours() {
        let mut world = world();
        let local = rlva_line(ChatKind::Local(ChatType::Normal));
        let set = set_color([0.6, 0.2, 0.8, 1.0]);
        let stranger = Uuid::from_u128(99);
        assert_eq!(map_color(&world, &FRIEND, "Teagan Linden"), set);
        assert!(contact_chat_color(&local, &world).is_some() && contact_sender_color(&local, &world).is_some());
        assert_ne!(resident_name(&world, &FRIEND, Color32::PLACEHOLDER), Color32::PLACEHOLDER);
        rlva(&mut world, "@shownames=n");
        // the dot of anyone else, whatever the name and the friendship say
        assert_eq!(
            map_color(&world, &FRIEND, "Teagan Linden"),
            map_color(&world, &stranger, "Somebody Else")
        );
        for kind in [ChatKind::Local(ChatType::Normal), ChatKind::Im] {
            assert_eq!(contact_chat_color(&rlva_line(kind), &world), None);
            assert_eq!(contact_sender_color(&rlva_line(kind), &world), None);
        }
        assert_eq!(resident_name(&world, &FRIEND, Color32::PLACEHOLDER), Color32::PLACEHOLDER);
        rlva(&mut world, &format!("@shownames:{FRIEND}=add"));
        assert_eq!(map_color(&world, &FRIEND, "Teagan Linden"), set);
        assert!(contact_chat_color(&local, &world).is_some());
    }

    /// LLVOAvatar::getNameTagColor ends with the mark whatever RLVa hides,
    /// and LLNetMap::draw gives a hidden resident the others' dot.
    #[test]
    fn rlva_hidden_names_keep_their_mark_on_the_name_tag_and_lose_it_on_the_map() {
        let mut world = world();
        let stranger = Uuid::from_u128(99);
        let unmarked = tag_color(&world, &stranger, "Somebody Else", false);
        world.avatar_marks.set(&[FRIEND], [186, 0, 31]);
        let mark = text([186, 0, 31, 255]);
        assert_eq!(tag_color(&world, &FRIEND, "Teagan Linden", false), mark);
        assert_eq!(mark_color(&world, &FRIEND), Some(c([186, 0, 31, 255])));
        rlva(&mut world, "@shownames=n");
        // the tag: the mark still, and no distance colour over it
        assert_eq!(tag_color(&world, &FRIEND, "Teagan Linden", false), mark);
        assert!(relation_tag(&world, &FRIEND, "Teagan Linden"));
        // the map: the dot of anyone else
        assert_eq!(mark_color(&world, &FRIEND), None);
        assert_eq!(
            map_color(&world, &FRIEND, "Teagan Linden"),
            map_color(&world, &stranger, "Somebody Else")
        );
        // no mark: a hidden friend's tag tells nothing, as before
        world.avatar_marks.clear(&[FRIEND]);
        assert_eq!(tag_color(&world, &FRIEND, "Teagan Linden", false), unmarked);
        assert!(!relation_tag(&world, &FRIEND, "Teagan Linden"));
    }
}
