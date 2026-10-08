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

pub fn c(v: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(v[0], v[1], v[2], v[3])
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

/// Text color of a chat line (LLViewerChat::getChatColor).
pub fn chat_color(line: &ChatLine, world: &World) -> Color32 {
    let k = get();
    let v = match line.kind {
        ChatKind::Own => k.chat_mine,
        ChatKind::System => k.chat_system,
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
    c(v)
}

/// Color of a sender's name in the conversation headers.
pub fn sender_color(line: &ChatLine) -> Color32 {
    let k = get();
    match line.kind {
        ChatKind::Object(_) | ChatKind::ObjectIm => c(k.sender_object),
        _ => c(k.sender_avatar),
    }
}

/// Name tag color of an avatar (own, Linden, friend, else match / mismatch).
pub fn tag_color(world: &World, id: &Uuid, name: &str, custom_display_name: bool) -> Color32 {
    let k = get();
    let v = if *id == world.agent_id {
        k.tag_me
    } else if is_linden(name) {
        k.tag_linden
    } else if is_friend(world, id) {
        k.tag_friend
    } else if world.is_avatar_blocked(id) {
        k.tag_muted
    } else if custom_display_name {
        k.tag_mismatch
    } else {
        k.tag_match
    };
    c(v)
}

/// Minimap marker color of an avatar.
pub fn map_color(world: &World, id: &Uuid, name: &str) -> Color32 {
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
