# #3450 Script-taken left/right no longer turns the avatar

## What changed and why

A script `llTakeControls(CONTROL_ROT_LEFT|CONTROL_ROT_RIGHT, TRUE, FALSE)` still left the avatar (and the camera, which follows that heading) turning locally. The Legacy Viewer skips that turn.

`ScriptControls::rotate_grabbed` is true when yaw was taken outright (`YAW_POS` or `YAW_NEG`). A pass-on hold does not count. `Agent::update` then skips keyboard yaw and joystick yaw. Pitch, mouselook mouse turning, and the control flags are unchanged, so the script still receives the keys.

The live call is in `app.rs`, from `world.script_controls`. Other `update` call sites pass `false`.

## Legacy Viewer sources

- `indra/newview/llagent.cpp` `LLAgent::yaw` (about lines 1700–1705): the agent frame rotates only `if (!rotateGrabbed())`.
- `LLAgent::rotateGrabbed` (about lines 4121–4125): counts `mControlsTakenCount` for `YAW_POS` and `YAW_NEG` only. Passed-on holds do not count.
- `LLAgent::moveYaw` (about lines 924–953): still sets the yaw control flags, so the script keeps the keys.
- Mouselook mouse turning uses `gAgent.rotate` (`indra/newview/lltoolgun.cpp`, about line 125), not `yaw`. That path was left as it is (`camera/mod.rs` still adds the mouse delta to `agent.yaw`).

## Tests

Compiled in this worktree with the new parameter ignored and `rotate_grabbed` returning false. They failed (`/tmp/3450-fail-first.log`, exit 101):

- `rotate_grabbed_counts_yaw_taken_outright`: "taken outright" (a taken `YAW_NEG` still reported free).
- `a_grabbed_turn_leaves_the_heading_and_still_sends_the_flag`: heading moved from `0.0` to `0.19707966`.

After the change both passed (`/tmp/3450-retest.log`, exit 0).

## Checks (toolchain 1.99.1)

- `cargo +1.99.1 fmt --all --check`: exit 0.
- `cargo +1.99.1 clippy --workspace --all-targets --locked -- -D warnings`: exit 101, exactly the 17 accepted Linux `dead_code` items (`write_plain`, `preserve`, `parse_driver_date`, `luid_text`, `adapter_used`, `path_ids`, `fold_serial`, `usable_uuid`, `pick_windows`, `arrived`, `ABSOLUTE`, `VIRTUAL_DESKTOP`, `PIXELS`, `RANGE`, `Packet`, `Absolute`, `packet`). None are in files this branch changes.
- `cargo +1.99.1 test --workspace --locked --no-fail-fast`: exit 101. Failures: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu`, `what_inter_lacks_comes_from_the_same_faces_as_with_noto`. `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` passed. `closing_viewer_refuses_links` passed. aurora-viewer: 7473 passed, 1 failed (the font test).
- `cargo +1.99.1 build -p aurora-viewer --locked`: exit 0. `target/debug/aurora-viewer` is 662M.

cfg(windows) was not touched.

## What to test by hand

Offline: `AURORA_DEMO=1` and `AURORA_DEMO_RELEASE_KEYS=1`. After frame 400 a vehicle script has taken the arrow keys outright. Left and Right do not turn the avatar or the camera. Walking keys still move the avatar in this demo. Advanced › Release keys, once chosen, restores turning.

On Second Life and on OpenSim: a script `llTakeControls(CONTROL_ROT_LEFT|CONTROL_ROT_RIGHT, TRUE, FALSE)`. Left and Right no longer turn the avatar or the camera. The script still sees the keys. With the third argument `TRUE` (pass on) they turn as before. Mouselook mouse turning still turns the view.

## Base and overlaps

Base `ed5100a0bc241e884cb8afb4f99dcc610d2a0aa4` (`origin/main` at hand-off). Not rebased.

Open PR #3086 (another contributor) adds a `rotate_grabbed` built on `held()`, which also counts pass-on, for the seated camera orbit. Open PR #2248 adds `up_grabbed` in the same `script_controls.rs`. Expect a textual conflict. This function counts taken yaw only, and the name stays `rotate_grabbed`.

Suggested `Closes #3450`. Labels `fix`, `avatar`.
