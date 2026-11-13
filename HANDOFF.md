# #3360 Ctrl+1–5 build shortcuts follow the key position

## What changed and why

On a French AZERTY layout the key in the 4 position types `'`. egui maps that character to `Key::Quote`, and the build shortcuts only match `Key::Num4`, so Ctrl+4 never opened Create. The same gap was on the native key path and while the Contents list held focus.

`native_shortcut_key` now returns Num1–Num5 for Digit1–5 and Numpad1–5 before it looks at the typed character. The build window's egui dispatcher and the Contents surrender use `physical_key` when it is Num1–Num5, and still consume the logical event so the typed quote is taken. Letters stay on the typed character: a quote key that is not in the 4 position does not open Create, Ctrl+B is unchanged, and Close Window still looks only for W. Digit4 without Ctrl still types. Shift+Digit4 is not a tool shortcut. QWERTY is unchanged because that 4 key is already Num4.

No new interface strings. `cfg(windows)` was not touched. No demo pretends to change the operating-system layout.

## Legacy Viewer sources

- `indra/llwindow/llkeyboardwin32.cpp`, `LLKeyboardWin32` constructor: maps `'0'..'9'` and numpad `0x60..0x69` by virtual key, not by the typed OEM character.
- `indra/newview/skins/default/xui/en/menu_viewer.xml`: Focus `control|1`, Move `control|2`, Edit `control|3`, Create `control|4`, Land `control|5` (`Tools.SelectTool`). Build toggle stays `control|B`.

## Tests

Added in this worktree against the old mapping, compiled, and failed (`/tmp/3360-failfirst.log`, EXIT 101):

- `azerty_number_row_follows_the_key_position`: Digit1 plus the character `'` was `Some(Quote)`, expected `Some(Num1)`.
- `azerty_ctrl_digit4_opens_create_and_drops_the_hold`: `take_native_key` returned `None`, expected `Open(Create)`.
- `azerty_quote_on_digit4_opens_create_in_the_build_frame`: the build frame stayed on Edit, expected Create.
- `azerty_ctrl_4_leaves_contents_focus_and_opens_create`: a focused Contents list stayed on Edit, expected Create.

The same filter passed after the change (`/tmp/3360-retest.log`, EXIT 0): those four, plus the existing `real_xkb_keymaps_give_azerty_and_qwertz` which the `azerty` filter also selected.

The tests also keep QWERTY Digit4 as Num4, letter B as B, a real Quote key as Quote, Digit4 without Ctrl as not a shortcut, and Shift+Digit4 as not a shortcut. Ctrl+4 removes the physical hold so movement does not see it; the gesture path already skips a build shortcut that keeps its key.

## Checks

- `cargo +1.99.1 fmt --all --check`: EXIT 0.
- `cargo +1.99.1 clippy --workspace --all-targets --locked -- -D warnings`: EXIT 101, only the 17 accepted Linux `dead_code` items (`write_plain`, `preserve`, `parse_driver_date`, `luid_text`, `adapter_used`, `path_ids`, `fold_serial`, `usable_uuid`, `pick_windows`, `arrived`, `ABSOLUTE`, `VIRTUAL_DESKTOP`, `PIXELS`, `RANGE`, `Packet`, `Absolute`, `packet`). Nothing in the files this branch changes.
- `cargo +1.99.1 test --workspace --locked --no-fail-fast` (`/tmp/3360-fulltest.log`, EXIT 101). The only failures are the accepted ones: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu`, and `what_inter_lacks_comes_from_the_same_faces_as_with_noto` (Linux font task #3275). `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` passed. `closing_viewer_refuses_links` timed out under the suite load and passed alone (`/tmp/3360-closing.log`, EXIT 0).
- `cargo +1.99.1 build -p aurora-viewer --locked` (`/tmp/3360-build.log`, EXIT 0). Debug binary 661M. The same 17 warnings, and no others.

## What to test by hand

French AZERTY keyboard, Windows or Linux. No grid login is required for the shortcuts themselves; the build tools open offline. There is no `AURORA_DEMO_*` that changes the OS layout. `docs/TESTING.md` records that next to `AURORA_DEMO_BUILD`.

- Ctrl+1 opens Focus, Ctrl+2 Move, Ctrl+3 Edit, Ctrl+4 Create, Ctrl+5 Land.
- Ctrl+4 does not also move the avatar or play a gesture bound to that key.
- With the Contents list focused, Ctrl+4 still opens Create.
- A QWERTY keyboard is unchanged: Ctrl+1–5 and Ctrl+B do the same as before, and the quote key does not open Create.
- Pressing 4 without Ctrl still types the layout's character.

Based on `9b150261e`. `origin/main` later moved to `4c681ce27` (`#3494`, `#3498`, `#3499`); only `TASKS.md` overlaps, and this branch was not rebased. Suggested PR body: `Closes #3360`. Suggested labels: `fix`, `build-tools`.
