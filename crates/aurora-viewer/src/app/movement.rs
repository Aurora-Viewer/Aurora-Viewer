//! Enhancer orchestration. No position prediction is changed on a real grid.
use super::*;
use crate::movement::{Frame, Style};
use uuid::Uuid;

impl App {
    fn movement_height(&self) -> f32 {
        self.world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .map_or(1.7, |o| o.scale.z.clamp(0.6, 3.0))
    }
    pub(super) fn update_movement(&mut self) {
        if !self.in_world() {
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
        self.toolbar_ui.movement_active = prefs.any_enabled();
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
        let mut input = self.input;
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
        let step = self.movement_runtime.update(&prefs, &frame, &input, now);
        self.scene.movement_preview = None;
        self.bridge_runtime.motion = step.motion;
        if let Some(flying) = step.flying {
            self.world.agent.flying = flying;
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
        let actions = crate::ui::ao::show(
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
        );
        self.panels.ao = open;
        self.toolbar_ui.movement_active = prefs.any_enabled();
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
