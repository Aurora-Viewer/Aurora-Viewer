# #3367 English interface text spells color

## What changed and why

English catalog values used British spelling (colour, colours, coloured, Colour) while the message ids already said color. Preferences search folds the label and matches the start of each word, so a search for "color" missed "Colour blindness" and "Accent colour". Fifty-nine English values now say color, colors, or colored. French and Japanese catalogs are unchanged. Grey stays grey. Code comments and the notification test string "Pick a colour" are unchanged.

## Legacy Viewer sources

`indra/newview/skins/default/xui/en`: the English skin spells this word "color" (for example the "Color" label in `panel_preferences_firestorm.xml` and the "Colors" column in `panel_preferences_backup.xml`). One label still says "Colour scheme:" (`panel_preferences_skins.xml`).

## Tests

`english_catalog_values_spell_color` was compiled in this worktree against the old catalog and failed (`/tmp/3367-failfirst.log`, EXIT 101), listing the 59 lines that still contained "colour". After the value change the same filter passed (`/tmp/3367-retest.log`, EXIT 0).

## Checks

- `cargo +1.99.1 fmt --all --check`: EXIT 0.
- `cargo +1.99.1 clippy --workspace --all-targets --locked -- -D warnings` (`/tmp/3367-clippy.log`, EXIT 101): only the 17 accepted Linux `dead_code` items (`write_plain`, `preserve`, `parse_driver_date`, `luid_text`, `adapter_used`, `path_ids`, `fold_serial`, `usable_uuid`, `pick_windows`, `arrived`, `ABSOLUTE`, `VIRTUAL_DESKTOP`, `PIXELS`, `RANGE`, `Packet`, `Absolute`, `packet`).
- `cargo +1.99.1 test --workspace --locked --no-fail-fast` (`/tmp/3367-fulltest.log`, EXIT 101). The only failures are the accepted ones: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu`, and `what_inter_lacks_comes_from_the_same_faces_as_with_noto` (Linux font task #3275). `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` passed. `closing_viewer_refuses_links` passed in the suite.
- `cargo +1.99.1 build -p aurora-viewer --locked` (`/tmp/3367-build.log`, EXIT 0). Debug binary 661M. The same 17 warnings, and no others.

## What to test by hand

Open Preferences and search "color". These must appear: Accent color, Color blindness, Interface colors, Color vision. French and Japanese labels stay as they were. There is no new demo. `AURORA_DEMO_COLORBLIND` and `AURORA_DEMO_COLORS` still open the same pages; `docs/TESTING.md` names the English labels with the new spelling.

Based on `4c681ce27`. `origin/main` later moved to `63fdc4048` (`#3503`, `#3504`). Those commits add English strings that already say "color", and their `TASKS.md` and `docs/TESTING.md` edits are in other sections. This branch was not rebased. `cfg(windows)` was not touched. Suggested PR body: `Closes #3367`. Suggested labels: `fix`, `ui`, `i18n`.
