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

pub fn resident_name(world: &World, id: &Uuid, fallback: Color32) -> Color32 {
    world
        .contact_sets
        .appearance(id)
        .ui_name
        .map(|v| crate::interface_colors::on_panel(set_color(v)))
        .unwrap_or(fallback)
}

pub fn contact_chat_color(line: &ChatLine, world: &World) -> Option<Color32> {
    matches!(line.kind, ChatKind::Local(_) | ChatKind::Im | ChatKind::Own)
        .then(|| world.contact_sets.appearance(&line.source).chat_text.map(set_color))
        .flatten()
        .map(crate::interface_colors::on_panel)
}

pub fn contact_sender_color(line: &ChatLine, world: &World) -> Option<Color32> {
    matches!(line.kind, ChatKind::Local(_) | ChatKind::Im | ChatKind::Own)
        .then(|| {
            let a = world.contact_sets.appearance(&line.source);
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

/// The keyword colour of a line that holds a keyword while "Change the
/// colour" is on (Preferences › Chat › Keyword alerts). Ours and the debug
/// channel's never hold one (chatContainsKeyword); a conversation's lines
/// are IMs, the rest nearby chat.
pub fn keyword_color(line: &ChatLine) -> Option<Color32> {
    if matches!(
        line.kind,
        ChatKind::Own | ChatKind::Object(ChatType::Debug) | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_)
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
        ChatKind::Own => k.chat_mine,
        ChatKind::System | ChatKind::Invite(_) | ChatKind::InventoryOffer(_) | ChatKind::Payment(_) => k.chat_system,
        ChatKind::ObjectIm => k.chat_object_im,
        ChatKind::Object(t) => match t {
            ChatType::Debug => k.chat_errors,
            ChatType::OwnerSay => k.chat_owner,
            ChatType::Other(9) => k.chat_direct,
            _ => k.chat_objects,
        },
        ChatKind::Local(_) | ChatKind::Im => {
            if is_linden_avatar(world, &line.source, &line.from) {
                k.chat_linden
            } else if is_friend(world, &line.source) {
                k.chat_friends
            } else if world.is_avatar_blocked(&line.source) {
                k.chat_muted
            } else {
                k.chat_others
            }
        }
    };
    text(v)
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

/// Name tag color of an avatar (own, Linden, friend while the names are
/// colored by relation, else match / mismatch).
pub fn tag_color(world: &World, id: &Uuid, name: &str, custom_display_name: bool) -> Color32 {
    let k = get();
    let relation = k.relation_names;
    let v = if relation && *id == world.agent_id {
        k.tag_me
    } else if relation && is_linden(name) {
        k.tag_linden
    } else if relation && is_friend(world, id) {
        k.tag_friend
    } else if relation && world.is_avatar_blocked(id) {
        k.tag_muted
    } else if custom_display_name {
        k.tag_mismatch
    } else {
        k.tag_match
    };
    // on plates drawn in the windows' colours
    text(v)
}

/// Minimap marker color of an avatar.
pub fn map_color(world: &World, id: &Uuid, name: &str) -> Color32 {
    if let Some(color) = world.contact_sets.appearance(id).minimap {
        return set_color(color);
    }
    let k = get();
    let v = if *id == world.agent_id {
        k.map_me
    } else if is_linden_avatar(world, id, name) {
        k.map_linden
    } else if is_friend(world, id) {
        k.map_friend
    } else if world.is_avatar_blocked(id) {
        k.map_muted
    } else {
        k.map_other
    };
    c(v)
}
