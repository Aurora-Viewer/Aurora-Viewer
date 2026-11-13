# #3349 Chat bar: Enter after sending stays in chat

## What changed and why

With "Keep the chat bar active" on (the default), Enter on the empty chat bar left the keyboard and the next letters walked the avatar. The Legacy Viewer does the opposite: `CloseChatOnReturn` is off by default, so Enter after a send does not drop focus.

`ui/chat.rs` `bar_input` now:

- keeps the keyboard on an empty Enter when `chat_keep_focus` is on. Escape (the text field) and a click in the world (`app.rs`, `text_edit_focused` then `surrender_focus`, around line 22573) still leave;
- on a non-empty send with keep-active on, asks for the bar again instead of surrendering;
- with keep-active off, a single Enter still lets the keyboard go. A second Enter in the same pass asks for the bar again (start-chat);
- sends only the first Enter of the pass. Characters inserted after that Enter stay in the field as the next draft (`enter_pass` / `split_on_enter`).

The preference hint (`ui-options-chat-keep-focus-hint`, English, French, Japanese) now describes the empty-Enter rule. The settings comment and the `bar_input` comments match it. No new message id.

`app.rs` was not changed. The movement gate samples the previous pass's focus (`egui_wants_keyboard_input` around line 22406). Enter is not a `chat_bar_moves` key, and `chat_bar_passes_key` (around line 8087) only hands movement keys over when the bar already has the keyboard. Letters in the send frame already reach the field. Keeping the keyboard, or asking for it again on a second Enter, is what stops the next frame from walking.

Conversations still send on the first Enter only (`send_enter`). Ctrl+Shift+Enter is still a newline: `input::prepare` turns that chord into a paste before this path sees it.

Limit of the split: the chat bar's caret is at the end, so text after Enter is a suffix. If that suffix is not found, the whole field is sent, as before. Two complete lines in one frame send only the first; the rest stays for a later Enter.

## Legacy Viewer sources

- `indra/newview/fsnearbychatcontrol.cpp` `FSNearbyChatControl::handleKeyHere` (about lines 265–331): KEY_RETURN sends, then `autohide(true)`. KEY_ESCAPE calls `autohide(false)`.
- `FSNearbyChatControl::autohide` (about lines 241–261): `setFocus(false)` only when `CloseChatOnReturn`.
- `indra/newview/app_settings/settings.xml` `CloseChatOnReturn` (about line 3309): Boolean, default 0.
- `indra/newview/llviewerwindow.cpp` `LLViewerWindow::handleUnicodeChar` (about lines 3583–3597): Return is handled per key, after the characters already typed.

## Tests

Compiled in this worktree before the behavior change. They failed (`/tmp/3349-fail-first.log`, exit 101):

- `letters_after_enter_in_the_same_frame_stay_for_the_next_line`: left `"onetwo"`, right `"one"`.
- `a_second_enter_in_the_same_frame_takes_the_bar_back`: "the second Enter took the bar back".
- `an_empty_enter_keeps_the_bar_when_it_stays_active`: "the Enter after the message kept the bar".
- `the_chat_bar_can_keep_the_keyboard_after_sending`: "Enter on the empty bar kept the keyboard".

After the change, those four plus `the_enter_that_starts_the_chat_keeps_the_keyboard` (keep-active off, empty Enter still returns to the world) passed (`/tmp/3349-retest.log`, exit 0).

## Checks (toolchain 1.99.1)

- `cargo +1.99.1 fmt --all --check`: exit 0.
- `cargo +1.99.1 clippy --workspace --all-targets --locked -- -D warnings`: exit 101, exactly the 17 accepted Linux `dead_code` items (`write_plain`, `preserve`, `parse_driver_date`, `luid_text`, `adapter_used`, `path_ids`, `fold_serial`, `usable_uuid`, `pick_windows`, `arrived`, `ABSOLUTE`, `VIRTUAL_DESKTOP`, `PIXELS`, `RANGE`, `Packet`, `Absolute`, `packet`). None are in files this branch changes.
- `cargo +1.99.1 test --workspace --locked --no-fail-fast`: exit 101. Failures: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu`, `what_inter_lacks_comes_from_the_same_faces_as_with_noto`. `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` passed. `closing_viewer_refuses_links` passed. aurora-viewer: 7465 passed, 1 failed (the font test).
- `cargo +1.99.1 build -p aurora-viewer --locked`: exit 0. `target/debug/aurora-viewer` is 662M.

cfg(windows) was not touched. No Windows cross-build Clippy is needed for this branch.

## What to test by hand

No new demo variable. `AURORA_DEMO=1`, the ordinary chat bar. Do it once uncapped and once with `AURORA_FPS_LIMIT=10`.

1. Preferences › Chat › Nearby chat › "Keep the chat bar active after sending" on (the default). Type a line, Enter, Enter, type, twenty times. Every line is sent. The avatar does not walk. Enter on the empty bar leaves the cursor in the bar.
2. Escape leaves the bar. A click in the world leaves the bar. The next Enter takes it again.
3. Turn the preference off. Enter after a message lets the bar go. The next Enter takes it back. Escape and a world click still leave.

A grid login is not required. The same three steps on a grid are the same bar.

## Base and overlaps

Base `2cc8b2c6bcbcb7d5bcea1e56cd12e52cd0d6ad11`. Not rebased. At hand-off, `origin/main` was `ed5100a0bc241e884cb8afb4f99dcc610d2a0aa4` (`#2307`, third-person camera follow). That commit adds camera strings and docs in other sections of `en.ftl` / `fr.ftl` / `ja.ftl`, `TASKS.md`, and `docs/TESTING.md`. This branch edits the chat-bar hint near line 1493, the chat section of `TASKS.md`, and a chat-bar bullet in `docs/TESTING.md`.

Open PR #2436 (another contributor, an Enter-toggles option) edits the same `bar_input` lines. Expect a textual conflict. This branch does not include that option.

Suggested `Closes #3349`. Labels `fix`, `chat`.
