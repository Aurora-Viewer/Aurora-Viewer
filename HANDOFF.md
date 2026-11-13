# #3147 Blank group titles

Legacy Viewer origin: no, extends it.

## What changed and why

`GroupTitlesReply` dropped a role whose title string was empty, so Group titles had no row for it: nothing to activate, and no region rule. The human asked to list that role. The Legacy Viewer does not.

- `titles_reply` keeps the empty title, with the role id and the selected flag.
- The Group titles window shows `<Blank title>` (`group-titles-blank`) in the list, the filter, and the region-rule line. English `<Blank title>`, French `<Titre vide>`, Japanese `＜空のタイトル＞`. The stored title stays empty, so Activate and "This region" still use the role id, and arrival still matches that role (by role once the list is known, by the empty string before that).
- The group profile "Title shown" combo filters the blank title out again, which is what the Legacy profile did by never receiving it.
- Profile data and the region-title list already kept an empty title. A blank worn title still does not set a name-tag title.
- `AURORA_DEMO_GROUP_REGION`: Aurora Builders offers one blank role. It is not the title worn at the start.

## Legacy Viewer sources

- `indra/newview/llgroupmgr.cpp`, `LLGroupMgr::processGroupTitlesReply`: a title is pushed only when `!title.mTitle.empty()` (the check is at line 1382).
- `indra/newview/fsfloatergrouptitles.cpp`, `FSFloaterGroupTitles::processGroupTitleResults` (about lines 233–249): the floater fills its list from `gmgr_data->mTitles` and does not filter empties again, because the manager already dropped them.

## Tests

Compiled in this worktree before the fix, and failed:

- `groups::tests::role_data_titles_and_pairs`: left 2, right 3, "an empty title is kept".
- `ui::group_titles::tests::a_blank_role_title_is_listed_and_can_be_activated`: label was `""`, expected `<Blank title>`. The row's role was already set; the assertion that failed was the placeholder.
- `ui::group_titles::tests::a_blank_title_rule_says_the_placeholder`: left ` · Builders`, right `<Blank title> · Builders`.
- `ui::group_profile::home::tests::a_blank_title_is_not_offered_in_the_profile_combo`: left `["Member", "", "Founder"]`, right `["Member", "Founder"]`. The helper listed every title, which is what the combo did.

These passed in that same run, before `titles_reply` changed. The empty entry was already kept once it reached them:

- `world::region_titles::tests::an_empty_title_is_kept_and_a_region_rule_matches_it_by_role`
- `world::group_data::tests::an_empty_title_stays_in_the_profile_data`

After the fix, the same filtered run passed: aurora-net 1 passed, aurora-viewer 5 passed.

## Checks

Toolchain 1.99.1. Base `2cc8b2c6b`. `origin/main` was still `2cc8b2c6b` at hand-off. Not rebased.

- `cargo +1.99.1 fmt --all --check`: exit 0.
- `cargo +1.99.1 clippy --workspace --all-targets --locked -- -D warnings`: exit 101, exactly the 17 accepted Linux `dead_code` items (`write_plain`, `preserve`, `parse_driver_date`, `luid_text`, `adapter_used`, `path_ids`, `fold_serial`, `usable_uuid`, `pick_windows`, `arrived`, `ABSOLUTE`, `VIRTUAL_DESKTOP`, `PIXELS`, `RANGE`, `Packet`, `Absolute`, `packet`). Nothing else.
- `cargo +1.99.1 test --workspace --locked --no-fail-fast`: exit 101. Failures: `shader_tests::water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu`, `ui::fonts::tests::what_inter_lacks_comes_from_the_same_faces_as_with_noto`. `terrain_tests::pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` passed. `slurl_handoff::tests::closing_viewer_refuses_links` passed. aurora-viewer: 7467 passed, 1 failed, 16 ignored.
- `cargo +1.99.1 build -p aurora-viewer --locked`: exit 0. Binary `target/debug/aurora-viewer`.

`cfg(windows)` was not touched. No Windows cross-build Clippy for this branch.

Open PR #2653 (Communicate menu entry) does not overlap.

## What to test by hand

Demo, without logging in:

`AURORA_DEMO_GROUP_REGION=still`

Open Group titles (toolbar Group titles, or Contacts › Groups › Titles). Aurora Builders has a row `<Blank title>` (French `<Titre vide>`, Japanese `＜空のタイトル＞`). Builder is still the bold active row at the start. A filter of `blank` finds the placeholder. Double-click or Activate wears that role: the name tag has no group-title text. "This region" on that row: the rule line shows the placeholder and the group name.

On the grid (you log in): in a group whose role title is blank, Group titles shows `<Blank title>`, Activate works, and a "This region" rule for that role applies on the next arrival. The group profile "Title shown" combo does not list the blank title.

Suggested `Closes #3147`. Labels `fix`, `ui`.
