//! Keyboard forwarding to media plugins. The CEF plugin only reads the
//! native key data of its system. On Windows (Dullahan
//! nativeKeyboardEventWin) we rebuild the WM_KEYDOWN / WM_KEYUP / WM_CHAR
//! messages Windows would have sent. On Linux (nativeKeyboardEventSDL2 for
//! keys, nativeKeyboardEvent for text) we send what the Legacy Viewer's SDL2
//! window sends: SDL keycodes, their Windows virtual keys and the typed
//! UTF-16 units (LLWindowSDL::getNativeKeyData, LLKeyboardSDL::mapSDL2toWin).
//! On macOS (nativeKeyboardEventOSX) we send what the Legacy Viewer's Cocoa
//! window reads from its NSEvent: key code, Cocoa modifier flags and
//! characters (LLWindowMacOSX::getNativeKeyData). The helpers of every
//! system are built everywhere, so that their tests run on any.

use aurora_media::{EditCommand, MediaPlugin, Modifiers};
use winit::keyboard::KeyCode;

#[cfg_attr(any(target_os = "linux", target_os = "macos"), allow(dead_code))]
pub const WM_KEYDOWN: u32 = 0x0100;
#[cfg_attr(any(target_os = "linux", target_os = "macos"), allow(dead_code))]
pub const WM_KEYUP: u32 = 0x0101;
#[cfg_attr(any(target_os = "linux", target_os = "macos"), allow(dead_code))]
pub const WM_CHAR: u32 = 0x0102;

/// Windows virtual-key code of a physical key.
pub fn virtual_key(code: KeyCode) -> Option<u32> {
    use KeyCode::*;
    let vk = match code {
        Backspace => 0x08,
        Tab => 0x09,
        Enter | NumpadEnter => 0x0D,
        ShiftLeft => 0xA0,
        ShiftRight => 0xA1,
        ControlLeft => 0xA2,
        ControlRight => 0xA3,
        AltLeft => 0xA4,
        AltRight => 0xA5,
        Pause => 0x13,
        CapsLock => 0x14,
        Escape => 0x1B,
        Space => 0x20,
        PageUp => 0x21,
        PageDown => 0x22,
        End => 0x23,
        Home => 0x24,
        ArrowLeft => 0x25,
        ArrowUp => 0x26,
        ArrowRight => 0x27,
        ArrowDown => 0x28,
        Insert => 0x2D,
        Delete => 0x2E,
        Digit0 => 0x30,
        Digit1 => 0x31,
        Digit2 => 0x32,
        Digit3 => 0x33,
        Digit4 => 0x34,
        Digit5 => 0x35,
        Digit6 => 0x36,
        Digit7 => 0x37,
        Digit8 => 0x38,
        Digit9 => 0x39,
        KeyA => 0x41,
        KeyB => 0x42,
        KeyC => 0x43,
        KeyD => 0x44,
        KeyE => 0x45,
        KeyF => 0x46,
        KeyG => 0x47,
        KeyH => 0x48,
        KeyI => 0x49,
        KeyJ => 0x4A,
        KeyK => 0x4B,
        KeyL => 0x4C,
        KeyM => 0x4D,
        KeyN => 0x4E,
        KeyO => 0x4F,
        KeyP => 0x50,
        KeyQ => 0x51,
        KeyR => 0x52,
        KeyS => 0x53,
        KeyT => 0x54,
        KeyU => 0x55,
        KeyV => 0x56,
        KeyW => 0x57,
        KeyX => 0x58,
        KeyY => 0x59,
        KeyZ => 0x5A,
        Numpad0 => 0x60,
        Numpad1 => 0x61,
        Numpad2 => 0x62,
        Numpad3 => 0x63,
        Numpad4 => 0x64,
        Numpad5 => 0x65,
        Numpad6 => 0x66,
        Numpad7 => 0x67,
        Numpad8 => 0x68,
        Numpad9 => 0x69,
        NumpadMultiply => 0x6A,
        NumpadAdd => 0x6B,
        NumpadSubtract => 0x6D,
        NumpadDecimal => 0x6E,
        NumpadDivide => 0x6F,
        F1 => 0x70,
        F2 => 0x71,
        F3 => 0x72,
        F4 => 0x73,
        F5 => 0x74,
        F6 => 0x75,
        F7 => 0x76,
        F8 => 0x77,
        F9 => 0x78,
        F10 => 0x79,
        F11 => 0x7A,
        F12 => 0x7B,
        NumLock => 0x90,
        ScrollLock => 0x91,
        Semicolon => 0xBA,
        Equal => 0xBB,
        Comma => 0xBC,
        Minus => 0xBD,
        Period => 0xBE,
        Slash => 0xBF,
        Backquote => 0xC0,
        BracketLeft => 0xDB,
        Backslash => 0xDC,
        BracketRight => 0xDD,
        Quote => 0xDE,
        IntlBackslash => 0xE2,
        _ => return None,
    };
    Some(vk)
}

#[cfg_attr(any(target_os = "linux", target_os = "macos"), allow(dead_code))]
/// lParam of a key message: repeat count 1, scan code, extended bit, and
/// for a release the previous-state and transition bits.
pub fn key_lparam(scancode: u32, up: bool, repeat: bool) -> u32 {
    let extended = scancode & 0xE000 == 0xE000;
    let mut l = 1u32 | ((scancode & 0xFF) << 16);
    if extended {
        l |= 1 << 24;
    }
    if up {
        l |= (1 << 30) | (1 << 31);
    } else if repeat {
        l |= 1 << 30;
    }
    l
}

/// The Edit menu command a key is, or None. With Control held (Command
/// counts as Control on macOS, LLKeyboardMacOSX::updateModifiers), the Legacy
/// Viewer gives the key to the menus before the focused page
/// (LLViewerWindow::handleKey, indra/newview/llviewerwindow.cpp), and the
/// Edit menu's shortcuts (menu_edit.xml) send their command to the focused
/// media, its edit handler (LLViewerMediaImpl::paste and the others,
/// indra/newview/llviewermedia.cpp). The modifiers must match exactly, as in
/// LLMenuItemGL::handleAcceleratorKey: Ctrl+Shift+V is not Paste. The key is
/// the layout's letter, as the Legacy Viewer's SDL keycode (`event.key.key`
/// in LLWindowSDL::handleEvent, indra/llwindow/llwindowsdl.cpp), so
/// Ctrl+Z is Undo on QWERTZ and AZERTY too; the physical key stands in on a
/// layout without Latin letters, as egui-winit does. A layout's ASCII key
/// that is not a letter (`;` where QWERTY has Z, on Dvorak) is no shortcut.
pub fn edit_shortcut(letter: Option<char>, code: KeyCode, mods: Modifiers) -> Option<EditCommand> {
    if !(mods.control || mods.command) || mods.alt || mods.shift {
        return None;
    }
    let letter = letter.filter(char::is_ascii).map(|c| c.to_ascii_lowercase()).or(match code {
        KeyCode::KeyZ => Some('z'),
        KeyCode::KeyY => Some('y'),
        KeyCode::KeyX => Some('x'),
        KeyCode::KeyC => Some('c'),
        KeyCode::KeyV => Some('v'),
        KeyCode::KeyA => Some('a'),
        _ => None,
    })?;
    Some(match letter {
        'z' => EditCommand::Undo,
        'y' => EditCommand::Redo,
        'x' => EditCommand::Cut,
        'c' => EditCommand::Copy,
        'v' => EditCommand::Paste,
        'a' => EditCommand::SelectAll,
        _ => return None,
    })
}

/// The letter a key gives on the active layout (winit's logical key, which
/// Control does not change), or None for a named or multi-character key.
pub fn layout_letter(key: &winit::keyboard::Key) -> Option<char> {
    let winit::keyboard::Key::Character(s) = key else {
        return None;
    };
    let mut chars = s.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// Whether a held shortcut runs its command again: the items with
/// `allow_key_repeat` in menu_edit.xml (Undo, Redo, Paste).
fn edit_repeats(cmd: EditCommand) -> bool {
    matches!(cmd, EditCommand::Undo | EditCommand::Redo | EditCommand::Paste)
}

/// Whether the page reaches the system clipboard through the viewer: on
/// Linux, where `edit_paste` finds nothing to paste and `edit_cut` or
/// `edit_copy` reach no other program. Under Wayland, Chromium picks its
/// Wayland backend and the page, drawn off screen, never has the keyboard
/// focus that a Wayland client needs to read or set the clipboard; the
/// viewer's window has that focus. Under X11 the plugin's Chromium neither
/// reads the X clipboard nor answers for it, so a page's cut or copy would
/// leave every other program's paste waiting on it until it gives up (the
/// viewer's own for 4 s); a compositor that serves the X clipboard only to
/// a focused X window (Hyprland) leaves it nothing either. So the viewer
/// reads the clipboard for a Paste (`paste_script`) and sets it from the
/// page's selection for a Cut or Copy (`copy_script`).
pub fn clipboard_through_viewer() -> bool {
    cfg!(target_os = "linux")
}

/// Whether a key press is Paste and the viewer reads the clipboard for it
/// (`clipboard_through_viewer`): read it then, and pass it to `edit`.
pub fn wants_clipboard(letter: Option<char>, code: KeyCode, pressed: bool, repeat: bool, mods: Modifiers) -> bool {
    pressed
        && edit_shortcut(letter, code, mods).is_some_and(|cmd| cmd == EditCommand::Paste && (!repeat || edit_repeats(cmd)))
        && clipboard_through_viewer()
}

/// Run an Edit menu shortcut on a page instead of sending it the key: the
/// command on the press (and on a repeat when it allows one). True when the
/// key press is a shortcut, which the page then never gets, on every system;
/// the release still goes to the page, as in the Legacy Viewer, whose menus
/// take only the key down (LLMenuItemCallGL::handleAcceleratorKey,
/// indra/llui/llmenugl.cpp). `clipboard` is the text the viewer read for a
/// Paste (`wants_clipboard`): `paste_script` puts it into the page instead
/// of `edit_paste`. Where the viewer holds the clipboard
/// (`clipboard_through_viewer`), Cut and Copy run `copy_script` instead of
/// `edit_cut` and `edit_copy`; its answer comes back as
/// `MediaEvent::ScriptReply`, for `copied_text`.
pub fn edit(
    p: &mut MediaPlugin,
    letter: Option<char>,
    code: KeyCode,
    pressed: bool,
    repeat: bool,
    mods: Modifiers,
    clipboard: Option<&str>,
) -> bool {
    let Some(cmd) = edit_shortcut(letter, code, mods) else {
        return false;
    };
    if !pressed {
        return false;
    }
    if !repeat || edit_repeats(cmd) {
        match (cmd, clipboard) {
            // held while a Cut or Copy's answer is awaited, so the paste
            // gets what it gives (MediaEvent::PasteDue, `due_paste`)
            (EditCommand::Paste, Some(text)) => {
                if !p.paste_after_replies(text) {
                    p.execute_javascript(&paste_script(text));
                }
            }
            (EditCommand::Cut | EditCommand::Copy, _) if clipboard_through_viewer() => {
                let tag = copy_tag();
                p.execute_javascript_for_reply(&copy_script(&tag, cmd == EditCommand::Cut), &tag);
            }
            _ => p.edit(cmd),
        }
    }
    true
}

/// A fresh tag for a copy's answer (`copy_script`): random, so neither the
/// page's frames of another origin nor an earlier page can guess it.
fn copy_tag() -> String {
    format!("aurora-copy:{}:", uuid::Uuid::new_v4().simple())
}

/// The script behind Cut and Copy on Linux, where the Legacy Viewer sends
/// `edit_cut` and `edit_copy` (LLViewerMediaImpl::cut and copy,
/// indra/newview/llviewermedia.cpp) and the page sets the clipboard itself.
/// It finds the selection as `paste_script` finds the focus: through open
/// shadow roots and frames of the same origin; the selected part of a
/// focused text field, else the document's selection; never a password
/// field's. As a real cut or copy, a `cut` or `copy` event goes first: a
/// page that cancels it gives its own `text/plain` instead. A cut then
/// deletes the selection, only in an editable field (one undo step; a
/// focused checkbox, button or other field without text is not one).
/// The answer is logged as the tag and one JSON string literal, a lone
/// half of a surrogate pair made U+FFFD as a browser's copy does, every
/// character past ASCII escaped (XML, which carries it to the viewer, does
/// not allow some); an empty one when there is nothing to copy, so the
/// viewer stops waiting. A page that replaced `console.log` gets the log of
/// a blank frame of its own, removed at once.
pub fn copy_script(tag: &str, cut: bool) -> String {
    const SCRIPT: &str = r#"(function(tag,cut){var t='';try{var d=document,e=d.activeElement,r=null;for(;;){if(e&&e.shadowRoot&&e.shadowRoot.activeElement){r=e.shadowRoot;e=r.activeElement;continue}if(e&&/^(I?FRAME|OBJECT|EMBED|FENCEDFRAME)$/.test(e.tagName)){d=e.contentDocument;if(!d)throw 0;e=d.activeElement;r=null;continue}break}var f=e&&/^(INPUT|TEXTAREA)$/.test(e.tagName)?e:null;if(f&&String(f.type).toLowerCase()==='password')throw 0;var ed=f?!/^(checkbox|radio|button|submit|reset|file|image|color|range|hidden|date|datetime-local|month|week|time)$/i.test(f.type)&&!f.readOnly&&!f.disabled:!!(e&&e.isContentEditable),sel=r&&r.getSelection?r.getSelection():d.getSelection(),s=null;if(f&&f.selectionStart!=null)s=f.value.substring(f.selectionStart,f.selectionEnd);if(s===null)s=sel?String(sel):'';var a=sel&&sel.anchorNode,tg=(f||ed)?e:a?(a.nodeType===1?a:a.parentElement):null;tg=tg||e||d.body;var dt=new DataTransfer(),go=true;try{go=tg.dispatchEvent(new ClipboardEvent(cut?'cut':'copy',{clipboardData:dt,bubbles:true,cancelable:true,composed:true}))}catch(x){}if(!go)t=dt.getData('text/plain');else if(!cut||ed){t=s;if(cut&&s)d.execCommand('delete')}}catch(x){t=''}var j=JSON.stringify(String(t||'').replace(/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g,'\ufffd')).replace(/[\u007f-\uffff]/g,function(c){return'\\u'+('000'+c.charCodeAt(0).toString(16)).slice(-4)}),l=console.log;if(!/\[native code\]/.test(Function.prototype.toString.call(l))){try{var i=document.createElement('iframe');i.style.display='none';(document.body||document.documentElement).appendChild(i);i.contentWindow.console.log(tag+j);i.remove();return}catch(x){}}l.call(console,tag+j)})(TAG,CUT)"#;
    let tag = serde_json::to_string(tag).unwrap_or_else(|_| "\"\"".into());
    SCRIPT.replace("(TAG,CUT)", &format!("({tag},{cut})"))
}

/// The text a page's answer to `copy_script` gives the clipboard: the JSON
/// string literal at the start of `MediaEvent::ScriptReply`, never run;
/// None for an empty one (nothing selected: the clipboard stays as it is,
/// as in a browser) or a malformed one.
pub fn copied_text(reply: &str) -> Option<String> {
    let text = serde_json::Deserializer::from_str(reply).into_iter::<String>().next()?.ok()?;
    (!text.is_empty()).then_some(text)
}

/// What a paste held behind a Cut or Copy (`MediaEvent::PasteDue`) puts
/// into the page: the last of those answers that gave text, which is then
/// on the system clipboard, else what the clipboard held at the press.
pub fn due_paste(clipboard: &str, answers: &[String]) -> String {
    answers
        .iter()
        .rev()
        .find_map(|a| copied_text(a))
        .unwrap_or_else(|| clipboard.to_owned())
}

/// The script that pastes `text` into the page's focused field, where the
/// Legacy Viewer sends `edit_paste` (LLViewerMediaImpl::paste,
/// indra/newview/llviewermedia.cpp) and the page reads the clipboard itself.
/// As a real paste: a `paste` event carrying the text first, so a page that
/// handles pastes gets it, then, unless the page cancelled it, the text
/// inserted as plain text (`insertText`, one undo step, line breaks and
/// tabs kept). It follows the focus into open shadow roots and into
/// frames and embedded documents of the same origin; one of another origin
/// is out of the script's reach and gets nothing, not even the paste event.
/// An empty text pastes nothing. The text is one JSON string literal, never
/// code, with U+FFFE and U+FFFF escaped: XML, which carries the message to
/// the plugin, does not allow them.
pub fn paste_script(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let literal = serde_json::to_string(&text)
        .unwrap_or_else(|_| "\"\"".into())
        .replace('\u{FFFE}', "\\ufffe")
        .replace('\u{FFFF}', "\\uffff");
    format!(
        "(function(t){{if(!t)return;var d=document,e=d.activeElement;\
         for(;;){{if(e&&e.shadowRoot&&e.shadowRoot.activeElement){{e=e.shadowRoot.activeElement;continue}}\
         if(e&&/^(I?FRAME|OBJECT|EMBED|FENCEDFRAME)$/.test(e.tagName)){{try{{d=e.contentDocument;e=d.activeElement}}catch(x){{return}}continue}}\
         break}}\
         if(!e)return;var go=true;\
         try{{var dt=new DataTransfer();dt.setData('text/plain',t);\
         go=e.dispatchEvent(new ClipboardEvent('paste',{{clipboardData:dt,bubbles:true,cancelable:true,composed:true}}))}}catch(x){{}}\
         if(go)d.execCommand('insertText',false,t)}})({literal})"
    )
}

/// UTF-16 code units of a text, one WM_CHAR each.
pub fn char_units(text: &str) -> Vec<u32> {
    text.encode_utf16().map(|u| u as u32).collect()
}

/// Send a key (and the characters it types) to a plugin; false when the key
/// has no virtual-key code.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn forward(
    p: &mut MediaPlugin,
    code: KeyCode,
    scancode: u32,
    pressed: bool,
    repeat: bool,
    text: Option<&str>,
    mods: Modifiers,
) -> bool {
    let Some(vk) = virtual_key(code) else {
        return false;
    };
    let lparam = key_lparam(scancode, !pressed, repeat);
    let (ev, msg) = match (pressed, repeat) {
        (false, _) => (aurora_media::KeyEvent::Up, WM_KEYUP),
        (true, true) => (aurora_media::KeyEvent::Repeat, WM_KEYDOWN),
        (true, false) => (aurora_media::KeyEvent::Down, WM_KEYDOWN),
    };
    p.key_event(ev, vk as i32, mods, MediaPlugin::native_key_data(msg, vk, lparam));
    if pressed && let Some(t) = text.filter(|t| !t.is_empty()) {
        for unit in char_units(t) {
            p.text_input(t, mods, MediaPlugin::native_key_data(WM_CHAR, unit, lparam));
        }
    }
    true
}

/// Send a key (and the characters it types) to a plugin, as the Legacy
/// Viewer's SDL2 window does (LLWindowSDL::gatherInput, LLViewerMediaImpl::
/// handleKeyHere / handleUnicodeCharHere); false when the key has no code.
#[cfg(target_os = "linux")]
pub fn forward(
    p: &mut MediaPlugin,
    code: KeyCode,
    _scancode: u32,
    pressed: bool,
    _repeat: bool,
    text: Option<&str>,
    mods: Modifiers,
) -> bool {
    let Some((sym, win)) = sdl::key(code) else {
        return false;
    };
    let mask = sdl::modifiers(mods);
    // an SDL key repeat is one more key down: Dullahan's SDL2 path ignores
    // the "repeat" event
    let (ev, input) = if pressed {
        (aurora_media::KeyEvent::Down, "keydown")
    } else {
        (aurora_media::KeyEvent::Up, "keyup")
    };
    p.key_event(ev, win as i32, mods, MediaPlugin::native_key_data_sdl(sym, win, mask, input));
    if pressed {
        for (ch, unit) in sdl::typed(code, text, mods) {
            let native = MediaPlugin::native_key_data_sdl(unit, sdl::to_windows(unit), mask, "textinput");
            p.text_input(&ch, mods, native);
        }
    }
    true
}

/// Send a key (and the characters it types) to a plugin, as the Legacy
/// Viewer's Cocoa window does (LLWindowMacOSX::getNativeKeyData,
/// LLViewerMediaImpl::handleKeyHere / handleUnicodeCharHere); `scancode` is
/// the key's macOS key code (winit's scancode there). The CEF plugin passes
/// only non-printable keys on as keys (MediaPluginCEF::keyEvent); the
/// characters arrive as text, with the native data of their key
/// (MediaPluginCEF::unicodeInput). False when the key has no code.
#[cfg(target_os = "macos")]
pub fn forward(
    p: &mut MediaPlugin,
    code: KeyCode,
    scancode: u32,
    pressed: bool,
    repeat: bool,
    text: Option<&str>,
    mods: Modifiers,
) -> bool {
    let Some(vk) = virtual_key(code) else {
        return false;
    };
    let flags = mac::flags(mods);
    let (ev, kind) = match (pressed, repeat) {
        (false, _) => (aurora_media::KeyEvent::Up, mac::KEY_UP),
        (true, true) => (aurora_media::KeyEvent::Repeat, mac::KEY_DOWN),
        (true, false) => (aurora_media::KeyEvent::Down, mac::KEY_DOWN),
    };
    let (chars, unmodified) = mac::characters(code, text);
    p.key_event(
        ev,
        vk as i32,
        mods,
        MediaPlugin::native_key_data_mac(kind, flags, scancode, chars, unmodified, repeat),
    );
    if pressed {
        for (unit, unmodified) in mac::typed(text, mods) {
            let native = MediaPlugin::native_key_data_mac(mac::KEY_DOWN, flags, scancode, unit, unmodified, repeat);
            p.text_input(text.unwrap_or_default(), mods, native);
        }
    }
    true
}

/// The Cocoa side of the macOS keyboard (pure functions, tested everywhere).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod mac {
    use super::char_units;
    use aurora_media::Modifiers;
    use winit::keyboard::KeyCode;

    /// NSEventType of a key press and of its release.
    pub const KEY_DOWN: u32 = 10;
    pub const KEY_UP: u32 = 11;

    /// NSEventModifierFlags: Shift, Control, Option, Command.
    pub fn flags(mods: Modifiers) -> u32 {
        let mut f = 0;
        if mods.shift {
            f |= 1 << 17;
        }
        if mods.control {
            f |= 1 << 18;
        }
        if mods.alt {
            f |= 1 << 19;
        }
        if mods.command {
            f |= 1 << 20;
        }
        f
    }

    /// The character Cocoa gives a key that types none: the control
    /// characters of Return, Tab, Escape and Backspace, the private-use
    /// codes of the function keys (NSUpArrowFunctionKey…).
    pub fn function_char(code: KeyCode) -> Option<u32> {
        use KeyCode::*;
        Some(match code {
            Enter => 0x0D,
            NumpadEnter => 0x03,
            Tab => 0x09,
            Escape => 0x1B,
            Backspace => 0x7F,
            ArrowUp => 0xF700,
            ArrowDown => 0xF701,
            ArrowLeft => 0xF702,
            ArrowRight => 0xF703,
            F1 => 0xF704,
            F2 => 0xF705,
            F3 => 0xF706,
            F4 => 0xF707,
            F5 => 0xF708,
            F6 => 0xF709,
            F7 => 0xF70A,
            F8 => 0xF70B,
            F9 => 0xF70C,
            F10 => 0xF70D,
            F11 => 0xF70E,
            F12 => 0xF70F,
            Insert => 0xF727,
            Delete => 0xF728,
            Home => 0xF729,
            End => 0xF72B,
            PageUp => 0xF72C,
            PageDown => 0xF72D,
            _ => return None,
        })
    }

    /// The event's first character and first character without
    /// modifiers ([NSEvent characters] / charactersIgnoringModifiers):
    /// what the key types, else its function character, else none (0). A
    /// shifted letter is the lower-case one without modifiers.
    pub fn characters(code: KeyCode, text: Option<&str>) -> (u32, u32) {
        if let Some(c) = function_char(code) {
            return (c, c);
        }
        let Some(first) = text.and_then(|t| t.chars().next()) else {
            return (0, 0);
        };
        let unit = |c: char| char_units(&c.to_string()).first().copied().unwrap_or(0);
        (unit(first), unit(first.to_lowercase().next().unwrap_or(first)))
    }

    /// The UTF-16 units a key press types, each with its unmodified unit.
    /// With Control or Command down the Legacy Viewer's window types
    /// nothing (the key is a shortcut), nor for the keys that type control
    /// characters, which go as keys.
    pub fn typed(text: Option<&str>, mods: Modifiers) -> Vec<(u32, u32)> {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return Vec::new();
        };
        if mods.control || mods.command || text.chars().any(|c| c.is_control()) {
            return Vec::new();
        }
        char_units(text).into_iter().map(|u| (u, u)).collect()
    }
}

/// The SDL2 side of the Linux keyboard (pure functions, tested everywhere).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod sdl {
    use super::virtual_key;
    use aurora_media::Modifiers;
    use winit::keyboard::KeyCode;

    /// Keycodes of the keys that are not characters: their scancode (USB
    /// usage) with this bit (SDLK_SCANCODE_MASK).
    const SCANCODE: u32 = 1 << 30;

    /// SDL keycode of a key and the Windows virtual key the Legacy Viewer
    /// derives from it (LLKeyboardSDL::mapSDL2toWin). Printable keys are
    /// their unshifted character on a US layout: the CEF plugin only passes
    /// non-printable keycodes on (below 0x1c or from 0x7f), the characters
    /// arrive as text.
    pub fn key(code: KeyCode) -> Option<(u32, u32)> {
        use KeyCode::*;
        let printable = match code {
            Backquote => Some('`'),
            Minus => Some('-'),
            Equal => Some('='),
            BracketLeft => Some('['),
            BracketRight => Some(']'),
            Backslash => Some('\\'),
            Semicolon => Some(';'),
            Quote => Some('\''),
            Comma => Some(','),
            Period => Some('.'),
            Slash => Some('/'),
            IntlBackslash => Some('<'),
            Space => Some(' '),
            _ => {
                let vk = virtual_key(code)?;
                // letters and digits: their virtual key is their character
                ((0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk)).then(|| (vk as u8).to_ascii_lowercase() as char)
            }
        };
        if let Some(c) = printable {
            let win = match code {
                Backquote => 0xDF, // VK_OEM_8 in mapSDL2toWin
                _ => virtual_key(code)?,
            };
            return Some((c as u32, win));
        }
        let (sym, win) = match code {
            Backspace => (0x08, 0x08),
            Tab => (0x09, 0x09),
            // KP_ENTER is treated as RETURN (LLWindowSDL::gatherInput)
            Enter | NumpadEnter => (0x0D, 0x0D),
            Escape => (0x1B, 0x1B),
            Delete => (0x7F, 0x2E),
            CapsLock => (SCANCODE | 57, 0x14),
            F1 => (SCANCODE | 58, 0x70),
            F2 => (SCANCODE | 59, 0x71),
            F3 => (SCANCODE | 60, 0x72),
            F4 => (SCANCODE | 61, 0x73),
            F5 => (SCANCODE | 62, 0x74),
            F6 => (SCANCODE | 63, 0x75),
            F7 => (SCANCODE | 64, 0x76),
            F8 => (SCANCODE | 65, 0x77),
            F9 => (SCANCODE | 66, 0x78),
            F10 => (SCANCODE | 67, 0x79),
            F11 => (SCANCODE | 68, 0x7A),
            F12 => (SCANCODE | 69, 0x7B),
            PrintScreen => (SCANCODE | 70, 0x2C),
            ScrollLock => (SCANCODE | 71, 0x91),
            Pause => (SCANCODE | 72, 0x13),
            Insert => (SCANCODE | 73, 0x2D),
            Home => (SCANCODE | 74, 0x24),
            PageUp => (SCANCODE | 75, 0x21),
            End => (SCANCODE | 77, 0x23),
            PageDown => (SCANCODE | 78, 0x22),
            ArrowRight => (SCANCODE | 79, 0x27),
            ArrowLeft => (SCANCODE | 80, 0x25),
            ArrowDown => (SCANCODE | 81, 0x28),
            ArrowUp => (SCANCODE | 82, 0x26),
            NumLock => (SCANCODE | 83, 0x90),
            NumpadDivide => (SCANCODE | 84, 0x6F),
            NumpadMultiply => (SCANCODE | 85, 0x6A),
            NumpadSubtract => (SCANCODE | 86, 0xBD),
            NumpadAdd => (SCANCODE | 87, 0xBB),
            // the keypad maps to the functions of its keys, not to
            // VK_NUMPADx, which breaks things for SDL2 (mapSDL2toWin)
            Numpad1 => (SCANCODE | 89, 0x23),
            Numpad2 => (SCANCODE | 90, 0x28),
            Numpad3 => (SCANCODE | 91, 0x22),
            Numpad4 => (SCANCODE | 92, 0x25),
            Numpad5 => (SCANCODE | 93, 0x65),
            Numpad6 => (SCANCODE | 94, 0x27),
            Numpad7 => (SCANCODE | 95, 0x24),
            Numpad8 => (SCANCODE | 96, 0x26),
            Numpad9 => (SCANCODE | 97, 0x21),
            Numpad0 => (SCANCODE | 98, 0x2D),
            NumpadDecimal => (SCANCODE | 99, 0x2E),
            ContextMenu => (SCANCODE | 101, 0x5D),
            ControlLeft => (SCANCODE | 224, 0x11),
            ShiftLeft => (SCANCODE | 225, 0x10),
            AltLeft => (SCANCODE | 226, 0x12),
            ControlRight => (SCANCODE | 228, 0x11),
            ShiftRight => (SCANCODE | 229, 0x10),
            AltRight => (SCANCODE | 230, 0x12),
            _ => return None,
        };
        Some((sym, win))
    }

    /// mapSDL2toWin for a typed character: lower-case letters and digits
    /// give their virtual key, anything else stays as it is.
    pub fn to_windows(unit: u32) -> u32 {
        match char::from_u32(unit) {
            Some(c @ ('a'..='z' | '0'..='9')) => c.to_ascii_uppercase() as u32,
            _ => unit,
        }
    }

    /// The modifier mask of getNativeKeyData (it "appears" to match GDK):
    /// shift 1, control 4, alt 8.
    pub fn modifiers(mods: Modifiers) -> u32 {
        u32::from(mods.shift) | if mods.control { 4 } else { 0 } | if mods.alt { 8 } else { 0 }
    }

    /// Characters a key press types, each with its UTF-16 units. SDL's text
    /// input has no control characters, and with Control or Alt down the
    /// Legacy Viewer turns the text into a shortcut instead; Return is the
    /// exception, sent as a character as well (handleUnicodeUTF16 on
    /// SDLK_RETURN).
    pub fn typed(code: KeyCode, text: Option<&str>, mods: Modifiers) -> Vec<(String, u32)> {
        if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
            return vec![("\r".into(), 0x0D)];
        }
        if mods.control || mods.alt {
            return Vec::new();
        }
        let mut out = Vec::new();
        for c in text.unwrap_or_default().chars().filter(|c| !c.is_control()) {
            let mut units = [0u16; 2];
            for &unit in c.encode_utf16(&mut units).iter() {
                out.push((c.to_string(), u32::from(unit)));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lparams() {
        assert_eq!(key_lparam(0x1E, false, false), 0x001E_0001);
        assert_eq!(key_lparam(0xE04B, true, false) & (1 << 24), 1 << 24);
        assert_eq!(key_lparam(0x1E, true, false) >> 30, 3);
        assert_eq!(char_units("é😀"), vec![0xE9, 0xD83D, 0xDE00]);
        assert_eq!(virtual_key(KeyCode::KeyA), Some(0x41));
    }

    #[test]
    fn linux_cuts_and_copies_through_the_viewer() {
        assert_eq!(clipboard_through_viewer(), cfg!(target_os = "linux"));
    }

    #[test]
    fn copy_script_carries_only_its_tag_and_kind() {
        let tag = copy_tag();
        assert!(tag.starts_with("aurora-copy:") && tag.ends_with(':') && tag.len() == "aurora-copy::".len() + 32);
        assert_ne!(tag, copy_tag());
        let s = copy_script(&tag, true);
        assert!(s.ends_with(&format!("(\"{tag}\",true)")));
        assert!(copy_script(&tag, false).ends_with(&format!("(\"{tag}\",false)")));
        // the event first, then the delete of a cut, then the answer
        let event = s.find("new ClipboardEvent(cut?'cut':'copy'").expect("event");
        let delete = s.find("execCommand('delete')").expect("delete");
        let answer = s.find("l.call(console,tag+j)").expect("answer");
        assert!(event < delete && delete < answer);
        // never a password field, and the same reach as the paste
        assert!(s.contains("'password')throw 0"));
        // a cut deletes only in a field that holds text
        assert!(s.contains("checkbox|radio|button") && s.contains("&&!f.readOnly&&!f.disabled"));
        // a lone surrogate becomes U+FFFD before the literal
        assert!(s.contains(r"(?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g,'\ufffd')"));
        assert!(s.contains("e.shadowRoot.activeElement") && s.contains("OBJECT|EMBED|FENCEDFRAME"));
        // XML, which carries the script to the plugin, refuses U+FFFE and
        // U+FFFF: with one in it the plugin drops the whole script
        assert!(!s.contains(['\u{FFFE}', '\u{FFFF}']));
    }

    #[test]
    fn copied_text_is_the_literal_only() {
        let suffix = " in file() at line 1";
        let reply = |lit: &str| format!("{lit}{suffix}");
        assert_eq!(copied_text(&reply(r#""hello""#)), Some("hello".into()));
        // escaped past ASCII, as the script sends it: line breaks, tabs,
        // é, an emoji as its surrogate pair, U+FFFE
        assert_eq!(
            copied_text(&reply(r#""a\nb\tc \u00e9\ud83d\ude00\ufffe""#)),
            Some("a\nb\tc é😀\u{FFFE}".into())
        );
        assert_eq!(copied_text(&reply(r#""a\ufffdb""#)), Some("a\u{FFFD}b".into()));
        // a lone half of a pair is refused whole, so the script sends
        // U+FFFD in its place
        assert_eq!(copied_text(&reply(r#""a\ud83db""#)), None);
        // what would be code stays text
        assert_eq!(copied_text(&reply(r#""\"); alert(1); (\"""#)), Some("\"); alert(1); (\"".into()));
        // nothing selected, or not a literal: the clipboard is left alone
        assert_eq!(copied_text(&reply(r#""""#)), None);
        assert_eq!(copied_text("hello in file() at line 1"), None);
        assert_eq!(copied_text(r#""unterminated"#), None);
        assert_eq!(copied_text(""), None);
    }

    #[test]
    fn a_held_paste_takes_the_last_answer_with_text() {
        let lit = |t: &str| format!("\"{t}\" in file() at line 1");
        assert_eq!(due_paste("old", &[]), "old");
        assert_eq!(due_paste("old", &[lit("A")]), "A");
        // a later copy of nothing leaves the cut text on the clipboard
        assert_eq!(due_paste("old", &[lit("A"), lit(""), "junk".into()]), "A");
        assert_eq!(due_paste("old", &[lit("A"), lit("B")]), "B");
        assert_eq!(due_paste("old", &[lit("")]), "old");
    }

    #[test]
    fn paste_script_quotes_the_text() {
        // the text is one JSON string literal at the end of the script, line breaks as \n
        let lit = |t: &str| {
            let s = paste_script(t);
            let start = s.rfind("})(").expect("call") + 3;
            serde_json::from_str::<String>(&s[start..s.len() - 1]).expect("literal")
        };
        assert_eq!(lit("https://example.com/watch?v=1"), "https://example.com/watch?v=1");
        assert_eq!(lit("one\r\ntwo\rthree\tx"), "one\ntwo\nthree\tx");
        assert_eq!(lit(""), "");
        // quotes, backslashes and script ends stay text, never code
        let nasty = "\"); alert(1); (\"\\ ' ` ${x} </script> \u{2028} é😀";
        assert_eq!(lit(nasty), nasty);
        assert!(!paste_script(nasty).contains("alert(1); (\"\\"));
        // a paste event first, then the insert unless the page cancelled it
        let s = paste_script("x");
        assert!(s.find("ClipboardEvent('paste'").expect("event") < s.find("if(go)").expect("insert"));
        // XML refuses U+FFFE and U+FFFF, so they go escaped and still arrive
        let odd = "a\u{FFFE}b\u{FFFF}";
        assert!(!paste_script(odd).contains('\u{FFFE}') && !paste_script(odd).contains('\u{FFFF}'));
        assert_eq!(lit(odd), odd);
        // the focus is followed into shadow roots and embedded documents
        assert!(s.contains("e.shadowRoot.activeElement") && s.contains("OBJECT|EMBED|FENCEDFRAME"));
    }

    #[test]
    fn edit_shortcuts_are_the_legacy_edit_menu() {
        let none = Modifiers::default();
        let ctrl = Modifiers { control: true, ..none };
        let command = Modifiers { command: true, ..none };
        assert_eq!(edit_shortcut(None, KeyCode::KeyV, ctrl), Some(EditCommand::Paste));
        assert_eq!(edit_shortcut(None, KeyCode::KeyV, command), Some(EditCommand::Paste));
        assert_eq!(edit_shortcut(None, KeyCode::KeyC, ctrl), Some(EditCommand::Copy));
        assert_eq!(edit_shortcut(None, KeyCode::KeyX, ctrl), Some(EditCommand::Cut));
        assert_eq!(edit_shortcut(None, KeyCode::KeyA, ctrl), Some(EditCommand::SelectAll));
        assert_eq!(edit_shortcut(None, KeyCode::KeyZ, ctrl), Some(EditCommand::Undo));
        assert_eq!(edit_shortcut(None, KeyCode::KeyY, ctrl), Some(EditCommand::Redo));
        // plain typing and other shortcuts go to the page
        assert_eq!(edit_shortcut(None, KeyCode::KeyV, none), None);
        assert_eq!(edit_shortcut(None, KeyCode::KeyF, ctrl), None);
        assert_eq!(edit_shortcut(None, KeyCode::Delete, ctrl), None);
        // the modifiers must match exactly
        assert_eq!(edit_shortcut(None, KeyCode::KeyV, Modifiers { shift: true, ..ctrl }), None);
        assert_eq!(edit_shortcut(None, KeyCode::KeyV, Modifiers { alt: true, ..ctrl }), None);
        // the layout's letter decides: QWERTZ swaps Z and Y, AZERTY moves A and Z
        assert_eq!(edit_shortcut(Some('z'), KeyCode::KeyY, ctrl), Some(EditCommand::Undo));
        assert_eq!(edit_shortcut(Some('y'), KeyCode::KeyZ, ctrl), Some(EditCommand::Redo));
        assert_eq!(edit_shortcut(Some('a'), KeyCode::KeyQ, ctrl), Some(EditCommand::SelectAll));
        assert_eq!(edit_shortcut(Some('q'), KeyCode::KeyA, ctrl), None);
        assert_eq!(edit_shortcut(Some('W'), KeyCode::KeyZ, ctrl), None);
        // a layout without Latin letters falls back to the physical key
        assert_eq!(edit_shortcut(Some('м'), KeyCode::KeyV, ctrl), Some(EditCommand::Paste));
        // an ASCII key that is not a letter is no shortcut: Dvorak's `;` on Z
        assert_eq!(edit_shortcut(Some(';'), KeyCode::KeyZ, ctrl), None);
        assert_eq!(edit_shortcut(Some('j'), KeyCode::KeyC, ctrl), None);
        assert_eq!(layout_letter(&winit::keyboard::Key::Character("v".into())), Some('v'));
        assert_eq!(layout_letter(&winit::keyboard::Key::Named(winit::keyboard::NamedKey::Delete)), None);
        assert!(edit_repeats(EditCommand::Paste));
        assert!(!edit_repeats(EditCommand::Copy));
        assert!(!edit_repeats(EditCommand::SelectAll));
    }

    #[test]
    fn linux_keys_are_what_the_sdl2_window_sends() {
        // printable keys: their character, which the CEF plugin leaves to the text
        assert_eq!(sdl::key(KeyCode::KeyA), Some((0x61, 0x41)));
        assert_eq!(sdl::key(KeyCode::Digit7), Some((0x37, 0x37)));
        assert_eq!(sdl::key(KeyCode::Space), Some((0x20, 0x20)));
        // others: SDL keycode and the Windows virtual key of mapSDL2toWin
        assert_eq!(sdl::key(KeyCode::Backspace), Some((0x08, 0x08)));
        assert_eq!(sdl::key(KeyCode::Delete), Some((0x7F, 0x2E)));
        assert_eq!(sdl::key(KeyCode::NumpadEnter), Some((0x0D, 0x0D)));
        assert_eq!(sdl::key(KeyCode::ArrowLeft), Some((0x4000_0050, 0x25)));
        assert_eq!(sdl::key(KeyCode::F5), Some((0x4000_003E, 0x74)));
        assert_eq!(sdl::key(KeyCode::ShiftRight), Some((0x4000_00E5, 0x10)));
        assert_eq!(sdl::key(KeyCode::Numpad4), Some((0x4000_005C, 0x25)));
        let none = Modifiers::default();
        let ctrl_shift = Modifiers {
            control: true,
            shift: true,
            ..none
        };
        assert_eq!(sdl::modifiers(ctrl_shift), 5);
        // macOS: Cocoa flags, function characters, what a key types
        let command = Modifiers { command: true, ..none };
        assert_eq!(mac::flags(ctrl_shift), (1 << 17) | (1 << 18));
        assert_eq!(mac::flags(command), 1 << 20);
        assert_eq!(mac::characters(KeyCode::ArrowLeft, None), (0xF702, 0xF702));
        assert_eq!(mac::characters(KeyCode::Backspace, None), (0x7F, 0x7F));
        assert_eq!(mac::characters(KeyCode::KeyA, Some("A")), ('A' as u32, 'a' as u32));
        assert_eq!(mac::characters(KeyCode::ShiftLeft, None), (0, 0));
        assert_eq!(mac::typed(Some("é"), none), vec![(0xE9, 0xE9)]);
        assert!(mac::typed(Some("a"), command).is_empty(), "Cmd+A is a shortcut");
        assert!(mac::typed(Some("\r"), none).is_empty(), "Return goes as a key");
        assert_eq!(sdl::typed(KeyCode::KeyE, Some("é"), none), vec![("é".to_string(), 0xE9)]);
        assert_eq!(
            sdl::typed(KeyCode::KeyE, Some("😀"), none),
            vec![("😀".to_string(), 0xD83D), ("😀".to_string(), 0xDE00)]
        );
        assert_eq!(sdl::typed(KeyCode::Enter, Some("\r"), none), vec![("\r".to_string(), 0x0D)]);
        assert!(sdl::typed(KeyCode::Backspace, Some("\u{8}"), none).is_empty());
        let ctrl = Modifiers { control: true, ..none };
        assert!(sdl::typed(KeyCode::KeyA, Some("a"), ctrl).is_empty());
        assert_eq!(sdl::to_windows(0x61), 0x41);
        assert_eq!(sdl::to_windows(0xE9), 0xE9);
    }
}
