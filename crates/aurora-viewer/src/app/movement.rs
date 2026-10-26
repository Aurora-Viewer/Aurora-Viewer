//! Enhancer orchestration. No position prediction is changed on a real grid.
use super::*;
use crate::movement::{Frame, Style};
use uuid::Uuid;

impl App {
    pub(super) fn toggle_water_walk(&mut self) {
        self.toggle_water_mode(true);
    }
    pub(super) fn toggle_swimming(&mut self) {
        self.toggle_water_mode(false);
    }
    fn toggle_water_mode(&mut self, walking: bool) {
        if !self.in_world() {
            return;
        }
        let selected = self
            .settings
            .ao
            .accounts
            .get(&self.world.agent_id)
            .map_or(Uuid::nil(), |a| a.selected);
        let swim_ready = self.ao_catalog.swimming_ready(selected);
        let prefs = self.settings.movements.entry(self.world.agent_id).or_default();
        let enabled = if walking { !prefs.water_walk } else { !prefs.swimming };
        prefs.set_water_mode(walking, enabled, swim_ready);
        self.toolbar_ui.water_walk_active = prefs.water_walk;
        self.toolbar_ui.movement_active = prefs.swimming && swim_ready;
        if !walking && enabled && !swim_ready {
            self.open_movement();
        }
        if !self.demo {
            self.settings.save();
        }
        self.update_movement();
        self.update_bridge();
    }
    pub(super) fn movement_height(&self) -> f32 {
        self.world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .map_or(1.7, |o| o.scale.z.clamp(0.6, 3.0))
    }
    pub(super) fn update_movement(&mut self) {
        if !self.in_world() {
            self.world.water_float = Default::default();
            return;
        }
        let now = Instant::now();
        if let Some(message) = self
            .movement_runtime
            .passage_feedback
            .update(self.bridge_runtime.passage_report(), now)
        {
            self.movement_ui.message = message;
        }
        if let Some(pack) = crate::movement::strokes::ready()
            && self.scene.anims.get(&crate::movement::strokes::local_id(0)).is_none()
        {
            let rig = self.scene.avatar_lib.rig.clone();
            for (index, animation) in pack.iter().enumerate() {
                self.scene.anims.insert(
                    crate::movement::strokes::local_id(index),
                    Arc::new(crate::scene::anim::BoundAnim::bind(animation.clone(), &rig)),
                );
            }
        }
        if self.demo && self.frame_count == 360 && std::env::var_os("AURORA_DEMO_MOVEMENT").is_some() {
            let prefs = self.settings.movements.entry(self.world.agent_id).or_default();
            prefs.running = true;
            prefs.fly_assist = true;
            prefs.swimming = true;
            self.open_movement();
            self.movelock_ui.open = false;
            self.panels.perf = false;
            let mode = std::env::var("AURORA_DEMO_MOVEMENT").unwrap_or_default();
            let prefs = self.settings.movements.entry(self.world.agent_id).or_default();
            match mode.as_str() {
                "independent" => {
                    prefs.run_multiplier = 3.5;
                    prefs.fly_assist = false;
                    prefs.swimming = false;
                }
                "float-low" => prefs.float_height = -0.35,
                "float-high" => prefs.float_height = 0.35,
                "off" => {
                    prefs.running = false;
                    prefs.fly_assist = false;
                    prefs.swimming = false;
                }
                "passage-blocked" | "passage-reached" | "passage-pending" => {
                    prefs.running = false;
                    prefs.fly_assist = false;
                    prefs.swimming = false;
                    prefs.pass_wall = true;
                }
                "run-fall" | "run-ao" | "run-drop" => {
                    prefs.run_multiplier = 3.0;
                    prefs.fly_assist = false;
                    prefs.swimming = false;
                    self.always_run = true;
                }
                _ => {}
            }
            if mode.starts_with("passage-") {
                // Synthetic outcome only: the offline demo has no Havok.
                // Exercise the same feedback formatter used for LSL status.
                self.movement_runtime.passage_feedback.start(123, now);
                let report = aurora_net::bridge::PassageReport {
                    action: 123,
                    state: if mode == "passage-reached" {
                        3
                    } else if mode == "passage-blocked" {
                        4
                    } else {
                        2
                    },
                    progress_cm: if mode == "passage-reached" { 155 } else { 10 },
                    remaining_cm: if mode == "passage-reached" { 5 } else { 150 },
                    speed_cm: 400,
                    pushes: 0,
                };
                if let Some(message) = self.movement_runtime.passage_feedback.update(Some(report), now) {
                    self.movement_ui.message = message;
                    self.movement_ui.demo_reveal_message = true;
                }
            }
            if mode == "run-ao" {
                crate::ao::demo::assets(&mut self.scene);
                let selected = crate::ao::demo::seed(&mut self.world.inventory, self.world.agent_id);
                self.settings
                    .ao
                    .accounts
                    .insert(self.world.agent_id, crate::ao::Account { enabled: true, selected });
                self.panels.ao = true;
            }
            if let Some(style) = match mode.as_str() {
                "breaststroke" => Some(Style::Breaststroke),
                "backstroke" => Some(Style::Backstroke),
                "crawl" => Some(Style::Crawl),
                _ => None,
            } {
                let asset = Uuid::from_u128(0xA0_5100 + style as u128);
                self.panels.ao = false;
                self.panels.people = false;
                self.panels.inventory = false;
                self.panels.chat = false;
                let item = Uuid::from_u128(0xA0_5200 + style as u128);
                let rig = self.scene.avatar_lib.rig.clone();
                self.scene.anims.insert(
                    asset,
                    Arc::new(crate::scene::anim::BoundAnim::bind(
                        crate::movement::strokes::animation(style),
                        &rig,
                    )),
                );
                self.world.inventory.add_items(vec![aurora_net::inventory::InvItem {
                    id: item,
                    parent: self.bridge_runtime.folder(),
                    owner: self.world.agent_id,
                    asset_id: asset,
                    asset_type: 20,
                    inv_type: 19,
                    name: style.name().into(),
                    ..Default::default()
                }]);
                let prefs = self.settings.movements.entry(self.world.agent_id).or_default();
                prefs.style = style;
                prefs.strokes[style as usize] = item;
            }
            if let Some(region) = self.world.main_region.and_then(|h| self.world.regions.get_mut(&h))
                && let Some(info) = &mut region.info
            {
                let mode = std::env::var("AURORA_DEMO_MOVEMENT").unwrap_or_default();
                if !mode.starts_with("run-") && !mode.starts_with("passage-") {
                    Arc::make_mut(info).water_height = self.world.agent.position.z + if mode == "underwater" { 2.0 } else { 0.25 };
                }
            }
        }
        let water_demo = if self.demo {
            let float_height = self.settings.movements.get(&self.world.agent_id).map_or(0.0, |p| p.float_height);
            crate::ao::water_demo::trajectory(&mut self.world, self.frame_count, now, float_height)
        } else {
            None
        };
        if self.demo && crate::ao::water_demo::mode().is_some() {
            let offset = if crate::ao::water_demo::mode() == Some("dry") {
                Vec3::new(44.0, 0.0, 4.0)
            } else {
                Vec3::ZERO
            };
            let (eye, target) = if crate::ao::water_demo::mode()
                .is_some_and(|mode| mode.starts_with("wade") || mode.starts_with("swim") || mode.starts_with("water-"))
            {
                // Keep the inspection camera fixed: restarting the native
                // focus animation on every trajectory sample never settles.
                let x = if crate::ao::water_demo::mode().is_some_and(|mode| mode.starts_with("wade")) {
                    62.0
                } else {
                    46.0
                };
                let center = Vec3::new(x, 124.0, self.world.main_water_height());
                if crate::ao::water_demo::mode().is_some_and(|mode| mode.starts_with("water-jump")) {
                    let height = aurora_net::bridge::WATER_WALK_JUMP_HEIGHT;
                    (center + Vec3::new(-6.0, -8.0, height + 0.6), center + Vec3::Z * (height * 0.6))
                } else {
                    (center + Vec3::new(-3.2, -4.5, 2.1), center + Vec3::Z * 0.15)
                }
            } else {
                (Vec3::new(44.5, 116.0, 23.4) + offset, Vec3::new(45.2, 124.0, 21.8) + offset)
            };
            self.camera.set_view(eye, target, &mut self.world.agent, &self.settings.camera);
            self.camera.position = eye;
            self.camera.target = target;
        }
        let selected = self
            .settings
            .ao
            .accounts
            .get(&self.world.agent_id)
            .map_or(Uuid::nil(), |a| a.selected);
        let swim_ready = self.ao_catalog.swimming_ready(selected);
        let prefs = self.settings.movements.entry(self.world.agent_id).or_default();
        prefs.sanitize();
        let disable_swimming = prefs.swimming && !swim_ready && !self.ao_catalog.loading && !self.ao_catalog.failed;
        if disable_swimming {
            prefs.swimming = false;
        }
        let mut prefs = prefs.clone();
        prefs.swimming &= swim_ready;
        if disable_swimming && !self.demo {
            self.settings.save();
        }
        self.toolbar_ui.movement_active = prefs.swimming;
        self.toolbar_ui.water_walk_active = prefs.water_walk;
        let height = self.movement_height();
        let position = self.world.agent.position;
        let direction = crate::movement::horizontal_heading(self.camera.forward(), self.world.agent.forward());
        let water = self.world.main().and_then(|r| r.info.as_ref()).map(|i| i.water_height);
        let near_water = water.is_some_and(|w| (w - position.z).abs() < height * 0.5 + 0.9);
        if prefs.swimming
            && near_water
            && self
                .movement_probe
                .0
                .is_none_or(|t| now.duration_since(t) > std::time::Duration::from_millis(200))
        {
            let ahead = position + direction;
            let terrain = self
                .world
                .ground_height(position)
                .zip(self.world.ground_height(ahead))
                .filter(|(a, b)| b - a > 0.8 && water.is_some_and(|w| *b >= w - 0.25))
                .map(|(_, ground)| ahead.with_z(ground + height * 0.5 + 0.08))
                .filter(|target| {
                    let delta = *target - position;
                    (0.1..=3.5).contains(&delta.z) && delta.length_squared() <= 16.0
                });
            let obstacle = terrain.is_some() || self.scene.movement_obstacle(&self.world, position, direction);
            let landing = terrain.or_else(|| self.scene.swim_landing(&self.world, position, direction, height));
            self.movement_probe = (Some(now), obstacle, landing);
        } else if !near_water || !prefs.swimming {
            self.movement_probe = (None, false, None);
        }
        let can_fly = self
            .world
            .main()
            .and_then(|r| r.info.as_ref())
            .is_some_and(|r| r.region_flags & aurora_net::region_flags::BLOCK_FLY == 0)
            && self
                .world
                .parcel
                .as_ref()
                .is_none_or(|p| p.flags & aurora_net::parcel_flags::ALLOW_FLY != 0);
        let mut frame = Frame {
            position,
            velocity: self.world.agent.velocity,
            height,
            water,
            water_contact: self.scene.avatar_water_contact(&self.world),
            ground: self.world.ground_height(position),
            flying: self.world.agent.flying,
            can_fly,
            seated: self.world.agent.is_sitting(),
            suspended: self.world.teleporting
                || !self.world.movement_complete
                || !self.focused
                || self.camera.free().is_some()
                || self.autopilot.is_some(),
            locked: self.settings.bridge.get(&self.world.agent_id).is_some_and(|prefs| prefs.enabled)
                && self.bridge_runtime.locked()
                && !(self.input.forward
                    || self.input.back
                    || self.input.strafe_left
                    || self.input.strafe_right
                    || self.input.up
                    || self.input.down
                    || (self.camera.mouselook() && (self.input.turn_left || self.input.turn_right))),
            bridge_ready: self.bridge_runtime.available(),
            running: self.always_run || self.input.run,
            obstacle: self.movement_probe.1,
            landing: self.movement_probe.2,
            on_ground: crate::movement::run::on_ground(
                position,
                height,
                self.world
                    .objects
                    .index_of_uuid(&self.world.agent_id)
                    .and_then(|i| self.world.objects.get(i))
                    .and_then(|o| o.foot_plane),
                self.world.ground_height(position),
                self.world
                    .animations
                    .get(&self.world.agent_id)
                    .is_some_and(|raw| raw.iter().any(|a| a.id == crate::ao::State::Falling.motion())),
            ),
            jumping: self.world.animations.get(&self.world.agent_id).is_some_and(|raw| {
                raw.iter().any(|a| {
                    matches!(
                        crate::ao::State::from_motion(a.id),
                        Some(crate::ao::State::Jumping | crate::ao::State::PreJumping)
                    )
                })
            }),
        };
        // Raw Falling and the native foot plane can lag behind prim contact.
        // Only water exits consult the actual loaded walkable support here.
        if self.movement_runtime.checking_water_landing()
            && frame.velocity.z.abs() < 1.5
            && water.is_some_and(|water| position.z - height * 0.5 >= water - 0.1)
        {
            frame.on_ground |= self.scene.movement_support(&self.world, position, height);
        }
        if self.demo
            && self.frame_count >= 360
            && let Ok(mode) = std::env::var("AURORA_DEMO_MOVEMENT")
            && matches!(mode.as_str(), "run-fall" | "run-ao" | "run-drop")
        {
            let phase = (self.frame_count - 360) % 120;
            let falling = if mode == "run-drop" {
                phase >= 60
            } else {
                (90..100).contains(&phase)
            };
            frame.on_ground = !falling;
            frame.velocity.z = if falling {
                if mode == "run-drop" { -3.0 } else { -0.5 }
            } else {
                0.0
            };
            let id = if falling {
                crate::ao::State::Falling
            } else {
                crate::ao::State::Running
            }
            .motion();
            let previous = self
                .world
                .animations
                .get(&self.world.agent_id)
                .and_then(|raw| raw.iter().find(|a| a.id == id));
            let signal = crate::world::PlayingAnimation::from_signal(id, 1, now, previous);
            self.world.animations.insert(self.world.agent_id, vec![signal]);
        }
        if let Some(sample) = &water_demo {
            frame.on_ground = sample.grounded;
            frame.jumping = sample.jumping;
            frame.suspended = false;
        }
        let mut input = water_demo.as_ref().map_or(self.input, |s| s.input);
        if self.demo
            && self.frame_count >= 360
            && std::env::var("AURORA_DEMO_MOVEMENT").is_ok_and(|m| {
                matches!(
                    m.as_str(),
                    "breaststroke" | "backstroke" | "crawl" | "run-fall" | "run-ao" | "run-drop"
                )
            })
        {
            input.forward = true;
        }
        if self.camera.mouselook() {
            input.strafe_left |= input.turn_left;
            input.strafe_right |= input.turn_right;
        }
        if self.egui_ctx.egui_wants_keyboard_input() {
            self.movement_runtime.cancel_water_gesture();
        }
        let was_water_walking = self.movement_runtime.water_walk.active();
        let step = self.movement_runtime.update(&prefs, &frame, &input, now);
        let set = self.ao_catalog.selected(selected);
        let mut loaded = |state| {
            let asset = self.ao_runtime.selected_animations(set?, state).next()?.asset;
            self.scene.anims.get(&asset)
        };
        let clips = crate::movement::water_jump::Clips {
            dive: prefs.swimming
                && !prefs.water_walk
                && !was_water_walking
                && !matches!(step.motion.mode, 5 | 9 | 10)
                && loaded(crate::ao::State::WaterDiveJump).is_some()
                && loaded(crate::ao::State::WaterDiveEntry).is_some(),
            cannonball: loaded(crate::ao::State::WaterCannonball).is_some(),
            dive_outro: loaded(crate::ao::State::WaterDiveEntry).map_or(2.4, |a| a.anim.duration - a.anim.loop_in),
            cannonball_recovery: loaded(crate::ao::State::WaterCannonball).map_or_else(
                || crate::movement::water_jump::Clips::default().cannonball_recovery,
                |a| crate::movement::water_jump::Recovery::from_animation(&a.anim),
            ),
        };
        let mut clips = clips;
        // This also vetoes a dive-to-cannonball switch above a deck. Existing
        // falls keep their pose; contact/landing still cancel them normally.
        if clips.cannonball
            && prefs.water_jumps
            && self.movement_runtime.water_jump.descent_edge(&input)
            && !self.movement_runtime.water_jump.cannonball()
        {
            clips.cannonball &= self.scene.cannonball_clear(&self.world, &frame);
        }
        let was_cannonball = self.movement_runtime.water_jump.cannonball();
        let water_jumps = prefs.water_jumps
            && (prefs.swimming || input.down || self.movement_runtime.water_jump.busy())
            && !self.egui_ctx.egui_wants_keyboard_input();
        let horizontal = if frame.velocity.truncate().length() > 0.3 {
            frame.velocity.with_z(0.0)
        } else {
            let forward = self.world.agent.forward().with_z(0.0).normalize_or(Vec3::X);
            let left = Vec3::Z.cross(forward);
            (forward * (input.forward as u8 as f32 - input.back as u8 as f32)
                + left * (input.strafe_left as u8 as f32 - input.strafe_right as u8 as f32))
                * if frame.running { 4.5 } else { 2.5 }
        };
        let landing = (water_jumps && (input.up || input.down || frame.jumping || self.movement_runtime.water_jump.pose().is_some()))
            .then(|| crate::movement::water_jump::predict(&frame, horizontal, |p| self.world.ground_height(p)))
            .flatten()
            .filter(|landing| self.scene.water_jump_clear(&self.world, &frame, horizontal, *landing));
        let grounded = frame.on_ground;
        let jumping = frame.jumping;
        frame.on_ground |= was_water_walking || step.motion.mode == 13;
        frame.jumping |= step.motion.mode == 4 && !prefs.water_walk;
        self.movement_runtime
            .water_jump
            .update(water_jumps, clips, &frame, &input, landing, now);
        frame.on_ground = grounded;
        frame.jumping = jumping;
        let cannonball = self.movement_runtime.water_jump.cannonball();
        if cannonball && !was_cannonball {
            self.movement_runtime.cancel_for_cannonball();
        }
        if let Some(impact) = self.movement_runtime.water_jump.take_impact() {
            if self.demo {
                log::info!(
                    "offline water impact: {:?}, speed {:.2}, contact {:?}",
                    impact.kind,
                    impact.speed,
                    impact.position
                );
            }
            self.scene.water_impact(impact);
        }
        self.scene.movement_preview = None;
        self.bridge_runtime.motion = self.movement_runtime.water_jump.observer(height).unwrap_or(step.motion);
        if cannonball {
            self.world.agent.flying = false;
        } else if let Some(flying) = step.flying {
            self.world.agent.flying = flying;
        }
        // Resolve the same explicit swim intent as the AO before the camera
        // reads the new visual offset. The last AO state also covers poses
        // selected without an active bridge motor; stock UUIDs alone cannot
        // distinguish swimming from flight.
        let pose = self
            .movement_runtime
            .water_walk
            .pose()
            .or_else(|| self.movement_runtime.swim_intent(&input))
            .or(self.ao_runtime.current);
        let transitioning = self.movement_runtime.water_jump.pose().is_some()
            || self.movement_runtime.water_jump.outro()
            || input.up
            || (input.down && self.movement_runtime.state != crate::movement::State::WaterWalking)
            || self.movement_runtime.state == crate::movement::State::Leaving;
        let floating = self
            .world
            .water_float
            .at_surface(&frame, self.movement_runtime.state, pose, transitioning);
        let time = self.start.elapsed().as_secs_f32();
        let target = water.filter(|_| floating).map_or(0.0, |level| {
            let ground = self
                .scene
                .lists
                .water_ground
                .sample(position.truncate())
                .map(|g| g[0])
                .or(frame.ground)
                .unwrap_or(level - 20.0);
            aurora_render::water_effects::surface_level(position.truncate(), time, self.settings.water_effects.render(), level, ground)
                - level
        });
        self.world.water_float.update(
            self.world.main_region.unwrap_or(0),
            position,
            time,
            target,
            self.world.teleporting || !self.world.movement_complete || frame.seated,
        );
        if self.demo && self.frame_count.is_multiple_of(30) {
            log::trace!(target: "aurora_viewer::water_contacts", "surface float: active {floating}, pose {pose:?}, physical_z {:.3}, wave {target:.3}, visual {:.3}", position.z, self.world.water_float.offset());
        }
    }
    pub(super) fn pass_wall(&mut self) {
        if !self.in_world()
            || !self
                .settings
                .movements
                .get(&self.world.agent_id)
                .is_some_and(|prefs| prefs.pass_wall)
        {
            return;
        }
        let now = Instant::now();
        if self
            .pass_wall_at
            .is_some_and(|t| now.duration_since(t) < std::time::Duration::from_secs(2))
        {
            return;
        }
        self.pass_wall_at = Some(now);
        let reason = if self.world.agent.is_sitting() || self.world.teleporting || !self.world.movement_complete {
            Some(aurora_i18n::tr!("movement-pass-unavailable"))
        } else if !self.bridge_runtime.available() {
            self.bridge_runtime
                .diagnostic()
                .or(Some(aurora_i18n::tr!("movement-physics-unavailable")))
        } else if self.settings.bridge.get(&self.world.agent_id).is_some_and(|p| p.enabled) {
            Some(aurora_i18n::tr!("movement-pass-locked"))
        } else {
            None
        };
        if let Some(reason) = reason {
            self.movement_ui.message = aurora_i18n::tr!("movement-pass-not-sent", reason = reason.to_owned());
            self.open_movement();
            return;
        }
        let direction = crate::movement::horizontal_heading(self.camera.forward(), self.world.agent.forward());
        let destination = self
            .scene
            .movement_exit(&self.world, self.world.agent.position, direction, self.movement_height());
        if let Some(position) = destination
            && self.bridge_runtime.available()
            && let Some(motion) = self.movement_runtime.start_passage(position - self.world.agent.position, now)
        {
            self.bridge_runtime.motion = motion;
            self.update_bridge();
            self.movement_ui.message = aurora_i18n::tr!("movement-pass-sent").into();
        } else {
            self.movement_ui.message = aurora_i18n::tr!("movement-pass-no-space").into();
            self.open_movement();
        }
    }
    pub(super) fn open_movement(&mut self) {
        self.ao_ui.tab = 1;
        self.panels.ao = true;
        if let Some(prototype) = &mut self.ao_ui.prototype {
            prototype.open_movement();
        }
    }

    pub(super) fn ao_window(&mut self, ctx: &egui::Context, p: &Palette) {
        self.movement_ui.measured_speed = self.world.agent.velocity.length();
        let mut prefs = self.settings.movements.get(&self.world.agent_id).cloned().unwrap_or_default();
        self.movement_ui.motor_status =
            if prefs.running && prefs.run_multiplier > 1.0 && self.movement_runtime.state == crate::movement::State::Ground {
                crate::movement::run::diagnostic(self.bridge_runtime.motor_report(Instant::now()))
            } else {
                String::new()
            };
        let status = if !prefs.any_enabled() {
            aurora_i18n::tr!("movement-disabled")
        } else if let Some(reason) = self.bridge_runtime.diagnostic() {
            reason
        } else {
            self.movement_runtime.state.label()
        };
        let mut open = self.panels.ao;
        let mut account = self.settings.ao.accounts.get(&self.world.agent_id).cloned().unwrap_or_default();
        let mut actions = self.toolbar_ui.ao_quick.show(
            ctx,
            p,
            &self.ao_catalog,
            &self.world.inventory,
            &mut self.ao_runtime,
            &mut account,
            &mut prefs,
            &mut open,
            self.ao_ui.busy(),
        );
        actions.extend(crate::ui::ao::show(
            ctx,
            p,
            &mut self.world.inventory,
            &self.ao_catalog,
            &mut self.ao_runtime,
            &mut account,
            &mut self.ao_ui,
            &mut open,
            &mut self.movement_ui,
            &mut prefs,
            &self.settings.keybinds,
            status,
        ));
        self.panels.ao = open;
        self.toolbar_ui.movement_active = prefs.swimming;
        self.toolbar_ui.water_walk_active = prefs.water_walk;
        if actions.iter().any(|a| matches!(a, crate::ui::ao::Action::MovementPreferences)) {
            self.settings.movements.insert(self.world.agent_id, prefs);
        }
        if actions.iter().any(|a| matches!(a, crate::ui::ao::Action::Preferences)) {
            self.settings.ao.accounts.insert(self.world.agent_id, account);
        }
        for action in actions {
            self.apply_ao_action(action);
        }
    }
}
