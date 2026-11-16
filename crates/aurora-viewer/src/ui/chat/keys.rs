//! The keyboard of the Conversations window's tabs (#651).
//!
//! Ctrl + Tab shows the conversation below in the list, Ctrl + Shift + Tab
//! the one above, round the list, while the keyboard is in the window.
//! Alt + Up / Left and Alt + Down / Right do the same, as in the Legacy
//! Viewer (LLFloaterIMContainer::handleKeyHere,
//! indra/newview/llfloaterimcontainer.cpp; LLTabContainer::handleKeyHere
//! and selectNextTab, which goes round, indra/llui/lltabcontainer.cpp,
//! originally LGPL 2.1). On macOS Option + Left / Right stay in the field
//! to move by word, so only Option + Up / Down keep cycling there. Its field
//! takes the keyboard, so the next press goes on; elsewhere these keys stay
//! the camera's. Contacts is not a conversation and is left out.
//!
//! Ctrl+W, the window being the frontmost (`close_key`), closes the
//! conversation shown, and the window itself when that tab cannot be closed
//! (the nearby chat, Contacts) or the window is minimized
//! (LLMultiFloater::closeDockedFloater, indra/llui/llmultifloater.cpp). The
//! conversation shown next is the one below the closed one, or above it at
//! the end of the list (LLTabContainer::removeTabPanel): pressed again, it
//! closes the conversations one after the other, then the window.
//! Ctrl+Shift+W closes the window with the others (Close All Windows),
//! not one conversation.

use super::{ChatUi, ConvAction};
use crate::world::World;
use uuid::Uuid;

/// The Conversations floater's id.
const WINDOW: &str = "conversations";

/// A conversation of the list (None: the nearby chat).
pub(super) type Tab = Option<Uuid>;

/// The conversation one step from `current` in `order`, round the list;
/// from none of them (Contacts shown) the first or the last. None when
/// there is no other.
pub(super) fn step(order: &[Tab], current: Option<Tab>, forward: bool) -> Option<Tab> {
    let n = order.len();
    let at = current.and_then(|c| order.iter().position(|t| *t == c));
    let to = match at {
        Some(i) if forward => (i + 1) % n,
        Some(i) => (i + n - 1) % n,
        None if forward => 0,
        None => n.checked_sub(1)?,
    };
    order.get(to).copied().filter(|t| Some(*t) != current)
}

/// What shows once `closed` is closed: the conversation below it in
/// `order`, else the one above, else the nearby chat.
pub(super) fn after_close(order: &[Tab], closed: Uuid) -> Tab {
    let i = order.iter().position(|t| *t == Some(closed))?;
    order
        .get(i + 1)
        .or_else(|| i.checked_sub(1).and_then(|i| order.get(i)))
        .copied()
        .flatten()
}

/// Whether a pressed key cycles the conversations, and which way: `true`
/// goes to the one below in the list, `false` to the one above.
///
/// Ctrl + Tab and Ctrl + Shift + Tab work everywhere. The arrows are the
/// Legacy Viewer's: Alt + Up / Left and Alt + Down / Right. On macOS
/// Option + Left / Right belong to the text field (word moves), so only
/// Option + Up / Down keep cycling there.
pub(super) fn cycle(key: egui::Key, modifiers: egui::Modifiers, mac: bool) -> Option<bool> {
    if key == egui::Key::Tab && modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd {
        // Ctrl+Tab next, Ctrl+Shift+Tab previous.
        return Some(!modifiers.shift);
    }
    if modifiers.alt && !modifiers.ctrl && !modifiers.shift && !modifiers.command && !modifiers.mac_cmd {
        return match key {
            egui::Key::ArrowUp => Some(false),
            egui::Key::ArrowDown => Some(true),
            egui::Key::ArrowLeft if !mac => Some(false),
            egui::Key::ArrowRight if !mac => Some(true),
            _ => None,
        };
    }
    None
}

/// The keys of this frame, before the window's field reads them. `order`:
/// the conversations as the list shows them.
pub(super) fn handle(ctx: &egui::Context, st: &mut ChatUi, order: &[Tab], actions: &mut Vec<ConvAction>) {
    let current = (!st.contacts).then_some(st.selected);
    let window = egui::Id::unique(WINDOW);
    // the cross of the window otherwise (`widgets::Floater`)
    if !crate::ui::close_key::closes_every_window(ctx)
        && let Some(Some(closed)) = current
        && !crate::ui::widgets::floater_minimized(ctx, WINDOW)
        && crate::ui::close_key::take(ctx, window)
    {
        let next = after_close(order, closed);
        st.select_tab(next);
        st.reveal = next;
        actions.push(ConvAction::Close(closed));
        return;
    }
    let layer = egui::LayerId::new(egui::Order::Middle, window);
    let in_window = ctx
        .memory(|m| m.focused())
        .and_then(|field| ctx.read_response(field))
        .is_some_and(|field| field.layer_id == layer);
    if !in_window {
        return;
    }
    // (held down, each repeat goes one further, as in a list)
    let mac = cfg!(target_os = "macos");
    let mut forward = None;
    ctx.input_mut(|i| {
        i.events.retain(|event| {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
                && let Some(next) = cycle(*key, *modifiers, mac)
            {
                forward.get_or_insert(next);
                false
            } else {
                true
            }
        });
    });
    if let Some(tab) = forward.and_then(|forward| step(order, current, forward)) {
        st.select_tab(tab);
        st.reveal = tab;
    }
}

/// The conversations of the Legacy style's list, from top to bottom.
pub(super) fn legacy_order(world: &World, st: &ChatUi) -> Vec<Tab> {
    std::iter::once(None)
        .chain(st.reorder.sessions(world).into_iter().map(|(id, _)| Some(id)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);
    const C: Uuid = Uuid::from_u128(3);

    #[test]
    fn ctrl_tab_cycles_everywhere_and_option_left_right_only_off_mac() {
        use egui::Modifiers;
        let ctrl = Modifiers::CTRL;
        let ctrl_shift = ctrl | Modifiers::SHIFT;
        let alt = Modifiers::ALT;
        let cmd = Modifiers::COMMAND;
        // Physical Control on macOS reaches egui as Ctrl + Command.
        let mac_ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        for mac in [true, false] {
            assert_eq!(cycle(egui::Key::Tab, ctrl, mac), Some(true));
            assert_eq!(cycle(egui::Key::Tab, ctrl_shift, mac), Some(false));
            assert_eq!(cycle(egui::Key::Tab, mac_ctrl, mac), Some(true));
            assert_eq!(cycle(egui::Key::Tab, cmd, mac), None, "Command+Tab is the system's");
            assert_eq!(cycle(egui::Key::ArrowUp, alt, mac), Some(false));
            assert_eq!(cycle(egui::Key::ArrowDown, alt, mac), Some(true));
        }
        assert_eq!(cycle(egui::Key::ArrowLeft, alt, true), None, "Option+Left moves by word");
        assert_eq!(cycle(egui::Key::ArrowRight, alt, true), None, "Option+Right moves by word");
        assert_eq!(cycle(egui::Key::ArrowLeft, alt, false), Some(false));
        assert_eq!(cycle(egui::Key::ArrowRight, alt, false), Some(true));
        assert_eq!(cycle(egui::Key::ArrowLeft, alt | Modifiers::SHIFT, true), None);
        assert_eq!(cycle(egui::Key::ArrowLeft, alt | Modifiers::SHIFT, false), None);
        assert_eq!(cycle(egui::Key::ArrowLeft, alt | ctrl, false), None);
        assert_eq!(cycle(egui::Key::Tab, ctrl | alt, true), None);
    }

    #[test]
    fn alt_arrows_go_round_the_list() {
        let order = [None, Some(A), Some(B)];
        assert_eq!(step(&order, Some(None), true), Some(Some(A)));
        assert_eq!(step(&order, Some(Some(B)), true), Some(None));
        assert_eq!(step(&order, Some(None), false), Some(Some(B)));
        assert_eq!(step(&order, Some(Some(A)), false), Some(None));
        // from Contacts: the first or the last
        assert_eq!(step(&order, None, true), Some(None));
        assert_eq!(step(&order, None, false), Some(Some(B)));
        // a conversation the search hides: from the top
        assert_eq!(step(&[Some(A), Some(B)], Some(None), true), Some(Some(A)));
        // nothing else to show
        assert_eq!(step(&[None], Some(None), true), None);
        assert_eq!(step(&[], None, true), None);
    }

    #[test]
    fn ctrl_w_shows_the_conversation_below_then_above() {
        let order = [None, Some(A), Some(B), Some(C)];
        assert_eq!(after_close(&order, A), Some(B));
        assert_eq!(after_close(&order, C), Some(B));
        assert_eq!(after_close(&[None, Some(A)], A), None);
        assert_eq!(after_close(&[Some(A)], A), None);
        // closed while the search hid it
        assert_eq!(after_close(&[None, Some(B)], A), None);
    }
}
