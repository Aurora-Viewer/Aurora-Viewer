//! Preferences › Graphics (panel_preferences_graphics1.xml and its
//! advanced floater): the level of the whole, then quality, light, the sky,
//! reflections and Aurora's water, the image (anti-aliasing, tone mapping,
//! depth of field), smoothness, parcel visibility and ray tracing.

use super::super::cx::{Custom, Cx, Nothing, field};
use super::super::{COMBO_W, blend_order_choice, blend_precisions, preset_choice, stepped, toggle};
use egui::RichText;

fn saved_diagnostic(s: &crate::settings::Settings) -> aurora_render::render_controls::ContactDiagnostic {
    use aurora_render::render_controls::{ContactDiagnostic, ContactDiagnosticSwitches};
    ContactDiagnostic::from_saved(
        s.shadow_visibility_debug,
        ContactDiagnosticSwitches {
            mask: s.experimental_contact_mask_debug,
            candidates: s.experimental_contact_hit_debug,
            failures: s.experimental_contact_failure_debug,
            view_ray: s.contact_view_debug,
        },
        s.near_shadow_debug,
    )
}

fn shadow_diagnostics(cx: &mut Cx) {
    use super::super::cx::fields;
    use aurora_i18n::tr;
    use aurora_render::render_controls::{ContactBackend, ContactDiagnostic};

    cx.row(
        tr!("ui-options-shadow-visibility-diagnostic"),
        tr!("ui-options-shadow-recovery-diagnostic-help"),
        &fields!(
            shadow_visibility_debug,
            experimental_contact_mask_debug,
            experimental_contact_hit_debug,
            experimental_contact_failure_debug,
            contact_view_debug,
            near_shadow_debug
        ),
        |ui, s| {
            let mut selected = saved_diagnostic(s);
            // Preferences can know the selected method. The renderer also
            // gates against live availability and AURORA_CONTACT_VARIANT=skip.
            let backend = ContactBackend::from_frame(s.contact_shadows && s.shadows, s.contact_ray_method, false);
            let contact_reason = (backend == ContactBackend::None).then_some(tr!("ui-options-shadow-visibility-requires-contact"));
            let bend_reason =
                contact_reason.or_else(|| (backend != ContactBackend::Bend).then_some(tr!("ui-options-baseline-requires-bend")));
            let view_reason =
                contact_reason.or_else(|| (backend != ContactBackend::ViewRay).then_some(tr!("ui-options-baseline-requires-view-ray")));
            let changed = super::super::combo_blocked(
                ui,
                &mut selected,
                &[
                    (ContactDiagnostic::Off, tr!("ui-options-baseline-diagnostic-off"), None),
                    (
                        ContactDiagnostic::ShadowMapVisibility,
                        tr!("ui-options-shadow-visibility-map"),
                        None,
                    ),
                    (
                        ContactDiagnostic::ContactVisibility,
                        tr!("ui-options-shadow-visibility-contact"),
                        contact_reason,
                    ),
                    (
                        ContactDiagnostic::CombinedVisibility,
                        tr!("ui-options-shadow-visibility-combined"),
                        None,
                    ),
                    (ContactDiagnostic::Cascades, tr!("ui-options-shadow-visibility-cascades"), None),
                    (
                        ContactDiagnostic::Mask,
                        tr!("ui-options-shadow-visibility-filtered-mask"),
                        contact_reason,
                    ),
                    (
                        ContactDiagnostic::BendCandidates,
                        tr!("ui-options-baseline-bend-candidates"),
                        bend_reason,
                    ),
                    (
                        ContactDiagnostic::BendFailures,
                        tr!("ui-options-baseline-bend-failures"),
                        bend_reason,
                    ),
                    (
                        ContactDiagnostic::ViewRayOutcomes,
                        tr!("ui-options-baseline-view-ray-outcomes"),
                        view_reason,
                    ),
                ],
            );
            if !selected.supported_by(backend) {
                ui.label(tr!("ui-options-shadow-visibility-inactive"));
            }
            if changed {
                let switches = selected.switches();
                s.shadow_visibility_debug = selected.visibility_setting();
                s.experimental_contact_mask_debug = switches.mask;
                s.experimental_contact_hit_debug = switches.candidates;
                s.experimental_contact_failure_debug = switches.failures;
                s.contact_view_debug = switches.view_ray;
                // This former standalone toggle now belongs to the selector.
                // Separate Dev Toolbox shadow/AO preferences remain untouched.
                s.near_shadow_debug = false;
            }
            changed
        },
    );
}

pub fn page(cx: &mut Cx, tab: usize) {
    match tab {
        0 => quality(cx),
        1 => light(cx),
        // the weather's settings are all in the Aurora Weather window
        2 => environment(cx),
        3 => reflections_and_water(cx),
        4 => image(cx),
        5 => {
            frames_per_second(cx);
            graphics_card(cx);
            overlays(cx);
        }
        6 => parcels(cx),
        7 => ray_tracing(cx),
        _ => {}
    }
}

/// Graphics › Sky and environment: the environment's own settings.
fn environment(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("common-environment"), |cx| {
        cx.row(
            tr!("common-time-of-day"),
            tr!("ui-options-shared-environment-region-parcel-eep-day-cycle"),
            &field!(time_of_day),
            |ui, s| {
                let names = crate::ui::bars::time_of_day();
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.scope_id().with("tod"))
                    .width(COMBO_W)
                    .selected_text(names[s.time_of_day.min(crate::world::env::TIME_OF_DAY_PRESETS) as usize])
                    .show_ui(ui, |ui| {
                        for i in std::iter::once(0).chain(crate::world::env::TIME_OF_DAY_ORDER) {
                            changed |= ui.selectable_value(&mut s.time_of_day, i, names[i as usize]).changed();
                        }
                    });
                changed
            },
        );
        cx.row(
            tr!("ui-options-environment-fade"),
            tr!("ui-options-fade-duration-when-you-change-the-environment-yourself-selector"),
            &field!(env_manual_transition),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.env_manual_transition, 0.0..=60.0)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(0.5)
                        .suffix(" s")
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        // the stars' sky (world/night_sky.rs)
        {
            use crate::world::night_sky::NightSky;
            cx.unfolded().choice(
                tr!("ui-options-night-sky"),
                tr!("ui-options-night-sky-help"),
                &field!(night_sky),
                &[
                    (NightSky::Real, tr!("ui-options-night-sky-real"), false),
                    (NightSky::Legacy, tr!("ui-options-night-sky-legacy"), false),
                ],
            );
            let legacy = cx.s.night_sky != NightSky::Real;
            cx.subs(legacy, night_sky_rows);
        }
        // an Aurora difference, also listed in Preferences › Aurora
        cx.experimental().toggle(
            tr!("ui-options-colored-sky-ambient"),
            tr!("ui-options-colored-sky-ambient-help"),
            &field!(colored_sky_ambient),
        );
        // an Aurora difference: the world drawn on a round planet
        cx.experimental().toggle(
            tr!("ui-options-planet-curvature"),
            tr!("ui-options-planet-curvature-help"),
            &field!(planet_curvature),
        );
        let flat = !cx.s.planet_curvature;
        cx.subs(flat, |cx| {
            cx.row(
                tr!("ui-options-planet-radius"),
                tr!("ui-options-planet-radius-help"),
                &field!(planet_radius_km),
                |ui, s| {
                    ui.add(
                        egui::Slider::new(&mut s.planet_radius_km, crate::settings::PLANET_RADIUS_KM_RANGE)
                            .logarithmic(true)
                            .step_by(1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .suffix(" km")
                            .trailing_fill(true),
                    )
                    .changed()
                },
            );
        });
        cx.toggle(
            tr!("ui-options-keep-environment"),
            tr!("ui-options-restore-the-local-environment-selected-sky-water-and-day"),
            &field!(env_persist),
        );
        // the "ignore" box of SettingsMakeNoTrans (Legacy Viewer:
        // Preferences › Notifications › Alerts)
        cx.toggle(
            tr!("ui-options-confirm-no-transfer-settings"),
            tr!("ui-options-confirm-no-transfer-settings-help"),
            &field!(confirm_no_transfer_settings),
        );
    });
}

/// The GPU the viewer draws with (#894): automatic (discrete first) or one
/// by name; the renderer opens it at startup, so the choice applies at the
/// next start. `AURORA_GPU` overrides it.
/// The real sky's rows (world/night_sky.rs): what sets its hour (first: it
/// decides whether the longitude below counts), where it is seen from, its
/// latitude and longitude (set by a preset, typed for Custom; moving one
/// makes the place Custom), the stars' brightness and the Milky Way's.
fn night_sky_rows(cx: &mut Cx) {
    use crate::world::night_sky::{GAIN_RANGE, MILKY_WAY_RANGE, SkyPlace, StarHour};
    let hours: Vec<(StarHour, &str)> = StarHour::ALL.iter().map(|h| (*h, h.label())).collect();
    cx.combo(
        aurora_i18n::tr!("ui-options-night-sky-hour"),
        aurora_i18n::tr!("ui-options-night-sky-hour-help"),
        &field!(night_sky_hour),
        &hours,
    );
    let places: Vec<(SkyPlace, &str)> = SkyPlace::ALL.iter().map(|p| (*p, p.label())).collect();
    if cx.combo(
        aurora_i18n::tr!("ui-options-night-sky-place"),
        aurora_i18n::tr!("ui-options-night-sky-place-help"),
        &field!(night_sky_place),
        &places,
    ) {
        cx.s.sync_night_sky();
    }
    cx.greyed(cx.s.night_sky_place != SkyPlace::Custom);
    if cx.slider(
        aurora_i18n::tr!("ui-options-night-sky-latitude"),
        aurora_i18n::tr!("ui-options-night-sky-latitude-help"),
        &field!(night_sky_latitude),
        -90.0..=90.0,
        "°",
    ) {
        cx.s.night_sky_place = SkyPlace::Custom;
        cx.s.sync_night_sky();
    }
    // the longitude counts with the real time only
    cx.greyed(cx.s.night_sky_place != SkyPlace::Custom || cx.s.night_sky_hour != StarHour::RealTime);
    if cx.slider(
        aurora_i18n::tr!("ui-options-night-sky-longitude"),
        aurora_i18n::tr!("ui-options-night-sky-longitude-help"),
        &field!(night_sky_longitude),
        -180.0..=180.0,
        "°",
    ) {
        cx.s.night_sky_place = SkyPlace::Custom;
        cx.s.sync_night_sky();
    }
    cx.greyed(false);
    if cx.slider(
        aurora_i18n::tr!("ui-options-night-sky-brightness"),
        aurora_i18n::tr!("ui-options-night-sky-brightness-help"),
        &field!(night_sky_gain),
        GAIN_RANGE,
        "×",
    ) {
        cx.s.sync_night_sky();
    }
    if cx.slider(
        aurora_i18n::tr!("ui-options-night-sky-milky-way"),
        aurora_i18n::tr!("ui-options-night-sky-milky-way-help"),
        &field!(night_sky_milky_way),
        MILKY_WAY_RANGE,
        "×",
    ) {
        cx.s.sync_night_sky();
    }
}

/// Ray tracing (aurora-render's rt_shadows.rs): the sun's shadows, and
/// behind their cog what else traces the scene they build. Greyed with the
/// reason on a graphics card that cannot, in a run set up for VR, and
/// without the shadows.
fn ray_tracing(cx: &mut Cx) {
    use aurora_i18n::tr;
    use aurora_render::RayTracingSupport as Support;
    cx.group(tr!("ui-options-ray-tracing"), |cx| {
        let p = cx.p;
        let card = match cx.st.ray_tracing {
            Support::Unsupported => Some(tr!("ui-options-ray-tracing-unsupported")),
            Support::Vr => Some(tr!("ui-options-ray-tracing-vr")),
            Support::Ready | Support::NotOpened => None,
        };
        if let Some(why) = card {
            cx.note(why);
        }
        let blocked = card.or((cx.s.shadow_quality == 0).then(|| tr!("ui-options-ray-tracing-needs-shadows")));
        // (on by default where the card can, in the shadow maps' place: no
        // experimental mark; the indirect light and the reflections keep it)
        cx.keywords("RT raytracing").row(
            tr!("ui-options-ray-traced-shadows"),
            tr!("ui-options-ray-traced-shadows-help"),
            &field!(ray_tracing.enabled),
            |ui, s| {
                let r = ui.add_enabled_ui(blocked.is_none(), |ui| toggle(ui, &p, &mut s.ray_tracing.enabled));
                if let Some(why) = blocked {
                    r.response.on_disabled_hover_text(why);
                }
                r.inner
            },
        );
        let off = blocked.is_some() || !cx.s.ray_tracing.enabled;
        cx.subs(off, |cx| {
            use crate::settings::{RT_QUALITY_HIGH, RT_SHADOW_LAMPS_MAX, RT_SOFTNESS_MAX, Settings};
            // (Aurora's own, as the games that trace rays have it: every
            // traced effect stays on, with fewer samples; choosing one sets
            // its lamps' count, which the slider below still moves)
            let levels = [
                (0, tr!("ui-options-preset-low"), false),
                (1, tr!("ui-options-preset-medium"), false),
                (RT_QUALITY_HIGH, tr!("ui-options-preset-high"), false),
            ];
            if cx.keywords("RT raytracing").choice(
                tr!("ui-options-rt-quality"),
                tr!("ui-options-rt-quality-help"),
                &field!(rt_quality),
                &levels,
            ) {
                let lamps = Settings::rt_lamp_quality(cx.s.rt_quality).lamps;
                cx.s.rt_shadow_lamps = u8::try_from(lamps).unwrap_or(RT_SHADOW_LAMPS_MAX).clamp(1, RT_SHADOW_LAMPS_MAX);
            }
            // (Aurora's own: the Legacy Viewer's maps blur by a kernel,
            // RenderShadowBlurSize, a debug setting)
            cx.slider(
                tr!("ui-options-rt-sun-softness"),
                tr!("ui-options-rt-sun-softness-help"),
                &field!(rt_sun_softness),
                0.0..=RT_SOFTNESS_MAX,
                "×",
            );
            cx.toggle(
                tr!("ui-options-ray-traced-lights"),
                tr!("ui-options-ray-traced-lights-help"),
                &field!(ray_tracing.lights),
            );
            cx.greyed(!cx.s.ray_tracing.lights);
            cx.slider(
                tr!("ui-options-rt-lamp-softness"),
                tr!("ui-options-rt-lamp-softness-help"),
                &field!(rt_lamp_softness),
                0.0..=RT_SOFTNESS_MAX,
                "×",
            );
            cx.slider(
                tr!("ui-options-rt-shadow-lamps"),
                tr!("ui-options-rt-shadow-lamps-help"),
                &field!(rt_shadow_lamps),
                1..=RT_SHADOW_LAMPS_MAX,
                "",
            );
            cx.greyed(false);
            cx.experimental().toggle(
                tr!("ui-options-ray-traced-indirect"),
                tr!("ui-options-ray-traced-indirect-help"),
                &field!(ray_tracing.indirect),
            );
            // (Aurora's own, under the traced indirect light it completes)
            cx.greyed(!cx.s.ray_tracing.indirect);
            cx.experimental()
                .toggle(tr!("ui-options-gi-probes"), tr!("ui-options-gi-probes-help"), &field!(rt_gi_probes));
            cx.greyed(false);
            let ssr = cx.s.ssr;
            cx.experimental().row(
                tr!("ui-options-ray-traced-reflections"),
                tr!("ui-options-ray-traced-reflections-help"),
                &field!(ray_tracing.reflections),
                |ui, s| {
                    let r = ui.add_enabled_ui(ssr, |ui| toggle(ui, &p, &mut s.ray_tracing.reflections));
                    r.response.on_disabled_hover_text(tr!("ui-options-ray-tracing-needs-ssr"));
                    r.inner
                },
            );
        });
    });
}

fn graphics_card(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-graphics-card"), |cx| {
        // why the dedicated card may not be the one drawing, the startup
        // log's notice in words a user can act on
        use aurora_render::adapter_choice::Notice;
        match cx.st.gpu_notice {
            Some(Notice::NoDiscrete) => cx.note(tr!("ui-options-graphics-card-no-discrete")),
            Some(Notice::DiscreteCannotPresent) => cx.note(tr!("ui-options-graphics-card-cannot-present")),
            None => {}
        }
        let in_use = tr!("ui-options-graphics-card-in-use", name = cx.st.gpu_in_use.clone());
        let hint = format!("{} {in_use}", tr!("ui-options-graphics-card-help"));
        cx.row_with(
            tr!("ui-options-graphics-card-choice"),
            &hint,
            &field!(gpu_adapter),
            |ui, s, st, _| {
                super::super::named_combo(
                    ui,
                    "gpu_adapter",
                    &mut s.gpu_adapter,
                    &st.gpus,
                    COMBO_W,
                    tr!("ui-options-graphics-card-automatic"),
                    false,
                )
                .0
            },
        );
    });
}

/// Other programs' overlays (OBS's game capture, Steam, RivaTuner, Epic…): their Vulkan layers
/// are kept out at startup unless allowed (`aurora_render::vulkan_layers`),
/// so the choice applies at the next start. `AURORA_VULKAN_LAYERS=keep`
/// loads them all.
fn overlays(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-overlays"), |cx| {
        cx.toggle(
            tr!("ui-options-allow-overlays"),
            tr!("ui-options-allow-overlays-help"),
            &field!(allow_overlays),
        );
    });
}

fn frames_per_second(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-frames-per-second"), |cx| {
        // FSR's mode sets the world's size in their place
        let fsr = cx.s.antialiasing == 7;
        cx.greyed(fsr);
        cx.toggle(
            tr!("ui-options-render-scale-auto"),
            tr!("ui-options-render-scale-auto-help"),
            &field!(render_scale_auto),
        );
        // While the world matches the display, the slider shows the
        // percentage the display gives, greyed: not a choice of the user,
        // so neither modified nor reset by this row.
        let scale = Custom {
            path: "render_scale_percent",
            modified: |s: &crate::settings::Settings, d: &crate::settings::Settings| {
                !s.render_scale_auto && s.render_scale_percent != d.render_scale_percent
            },
            reset: |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
                if !s.render_scale_auto {
                    s.render_scale_percent = d.render_scale_percent;
                }
            },
        };
        cx.greyed(fsr || cx.s.render_scale_auto);
        cx.row(
            tr!("ui-options-render-scale"),
            tr!("ui-options-render-scale-help"),
            &scale,
            |ui, s| super::super::slider(ui, &mut s.render_scale_percent, 50..=100, "%"),
        );
        cx.greyed(false);
        // across, its switch in line with those of the limits below
        cx.wide().toggle(
            tr!("ui-options-vertical-synchronization"),
            tr!("ui-options-synchronizes-frames-with-the-monitor-refresh-rate-no-tearing"),
            &field!(vsync),
        );
        cx.wide().toggle(
            tr!("ui-options-borderless-fullscreen"),
            tr!("ui-options-borderless-fullscreen-help"),
            &field!(window_fullscreen),
        );
        // the limits show under their switches: the value is what the
        // switch is turned on for
        cx.unfolded().toggle(
            tr!("extra-limit-frame-rate"),
            tr!("ui-options-caps-frames-per-second-less-heat-and-noise-enabled"),
            &field!(fps_cap),
        );
        let off = !cx.s.fps_cap;
        cx.subs(off, |cx| {
            cx.slider(tr!("ui-options-frame-limit"), "", &field!(fps_limit), 10..=360, tr!("status-fps"));
        });
        cx.unfolded().toggle(
            tr!("ui-options-limit-when-unfocused"),
            tr!("ui-options-when-the-window-is-not-in-the-foreground-the"),
            &field!(background_fps_cap),
        );
        let off = !cx.s.background_fps_cap;
        cx.subs(off, |cx| {
            cx.slider(
                tr!("ui-options-frame-limit-unfocused"),
                "",
                &field!(background_fps_limit),
                1..=120,
                tr!("status-fps"),
            );
        });
        cx.unfolded().toggle(
            tr!("ui-options-pause-when-card-busy"),
            tr!("ui-options-pause-when-card-busy-tip"),
            &field!(background_gpu_pause),
        );
        cx.unfolded().toggle(
            tr!("ui-options-gpu-recovery"),
            tr!("ui-options-gpu-recovery-tip"),
            &field!(gpu_recovery),
        );
        cx.unfolded().toggle(
            tr!("ui-options-gaming-mode-leave-on-focus"),
            tr!("ui-options-gaming-mode-leave-on-focus-tip"),
            &field!(gaming_mode_leave_on_focus),
        );
    });
}

fn parcels(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("parcel-visibility-filter"), |cx| {
        cx.combo(
            tr!("parcel-visibility-mode"),
            tr!("parcel-visibility-help"),
            &field!(parcel_visibility.mode),
            &[
                (crate::world::parcel_visibility::Mode::Off, tr!("common-disabled")),
                (crate::world::parcel_visibility::Mode::Always, tr!("parcel-visibility-always")),
                (crate::world::parcel_visibility::Mode::Listed, tr!("parcel-visibility-listed")),
                (crate::world::parcel_visibility::Mode::ExceptListed, tr!("parcel-visibility-except")),
            ],
        );
        cx.toggle(
            tr!("parcel-visibility-same-owner"),
            tr!("parcel-visibility-same-owner-help"),
            &field!(parcel_visibility.same_owner),
        );
        cx.toggle(
            tr!("parcel-visibility-same-group"),
            tr!("parcel-visibility-same-group-help"),
            &field!(parcel_visibility.same_group),
        );
    });
    cx.group(tr!("parcel-visibility-parcels"), |cx| {
        parcel_list(
            cx,
            tr!("parcel-visibility-parcels"),
            tr!("parcel-visibility-parcels-help"),
            &field!(parcel_visibility.parcels),
            false,
        );
    });
    cx.group(tr!("parcel-visibility-always-visible"), |cx| {
        parcel_list(
            cx,
            tr!("parcel-visibility-always-visible"),
            tr!("parcel-visibility-always-visible-help"),
            &field!(parcel_visibility.always_visible),
            true,
        );
    });
    cx.group(tr!("parcel-environment-title"), parcel_environments);
}

fn quality(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-presets"), |cx| {
        let p = cx.p;
        cx.wide().keywords(tr!("ui-options-preset-custom")).row_with(
            tr!("ui-options-preset-level"),
            tr!("ui-options-preset-level-help"),
            // a level writes the page's settings: they are the rows below
            &Nothing,
            |ui, s, st, _| preset_choice(ui, &p, s, st.ray_tracing),
        );
        cx.below()
            .row_with(tr!("graphics-saved-title"), tr!("graphics-saved-help"), &Nothing, |ui, s, st, _| {
                super::super::saved_graphics::controls(ui, &p, s, st)
            });
    });
    cx.group(tr!("ui-options-distance-and-detail"), |cx| {
        cx.slider(
            tr!("common-draw-distance"),
            tr!("ui-options-objects-beyond-this-distance-are-not-drawn-96-m"),
            &field!(draw_distance),
            crate::settings::DRAW_DISTANCE_MIN..=crate::settings::DRAW_DISTANCE_MAX,
            " m",
        );
        // FSRenderFarClipStepping; its interval in Debug Settings (clean menus)
        cx.keywords("FSRenderFarClipStepping progressive").toggle(
            tr!("ui-options-draw-distance-stepping"),
            tr!("ui-options-draw-distance-stepping-hint"),
            &field!(draw_distance_stepping),
        );
        cx.row(
            tr!("ui-options-level-of-detail-lod"),
            tr!("ui-options-lod-factor-for-sl-objects-and-meshes-2-by"),
            &field!(lod_factor),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.lod_factor, 0.5..=4.0)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(0.125)
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        // TreeMeshDetail (panel_preferences_graphics1.xml). From 0, where the
        // Legacy slider starts at 0.125: 0 turns trees and grass off, as the
        // Potato level and Debug Settings do (the human's decision,
        // 2026-11-07)
        cx.keywords("RenderTreeLODFactor LOD").row(
            tr!("ui-options-trees"),
            tr!("ui-options-trees-hint"),
            &field!(tree_lod_factor),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.tree_lod_factor, 0.0..=1.0)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(0.125)
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        cx.row(
            tr!("common-particles"),
            tr!("ui-options-maximum-particles-0-disabled"),
            &field!(max_particles),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.max_particles, 0..=8192)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(256.0)
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        // RenderAttachedParticles (panel_preferences_graphics1.xml)
        cx.keywords("RenderAttachedParticles").toggle(
            tr!("ui-options-attached-particles"),
            tr!("ui-options-attached-particles-hint"),
            &field!(attached_particles),
        );
        cx.toggle(
            tr!("ui-options-occlusion"),
            tr!("ui-options-objects-hidden-behind-others-walls-floors-are-not-drawn"),
            &field!(occlusion_culling),
        );
        cx.toggle(
            tr!("ui-options-auto-texture-memory"),
            tr!("ui-options-based-on-detected-video-memory"),
            &field!(texture_budget_auto),
        );
        cx.subs(false, |cx| {
            if cx.s.texture_budget_auto {
                let (vram, shared, integrated) = cx.st.vram;
                let budget = crate::settings::auto_texture_budget(vram, shared, integrated);
                let gpu = match vram {
                    Some(v) if !integrated => tr!("ui-options-mb-of-video-memory-detected", value0 = format!("{}", v)),
                    _ if integrated => tr!("ui-options-integrated-graphics").to_owned(),
                    _ => tr!("ui-options-video-memory-not-detected").to_owned(),
                };
                let p = cx.p;
                cx.row(tr!("ui-options-texture-memory"), &gpu, &field!(texture_budget_mb), |ui, _| {
                    ui.label(RichText::new(tr!("status-budget-mb", budget = format!("{}", budget))).color(p.ink));
                    false
                });
            } else {
                cx.row(
                    tr!("ui-options-texture-memory"),
                    tr!("ui-options-video-memory-the-textures-may-use"),
                    &field!(texture_budget_mb),
                    |ui, s| {
                        ui.add(
                            egui::Slider::new(&mut s.texture_budget_mb, 256..=32768)
                                .custom_parser(aurora_i18n::parse_number)
                                .logarithmic(true)
                                .suffix(tr!("memory-mb-suffix"))
                                .trailing_fill(true),
                        )
                        .changed()
                    },
                );
            }
        });
        let label = |v: u16| {
            if v <= 1 {
                tr!("common-disabled").to_owned()
            } else {
                format!("{v}×")
            }
        };
        let names: Vec<(u16, String)> = [1u16, 2, 4, 8, 16].into_iter().map(|v| (v, label(v))).collect();
        let choices: Vec<(u16, &str)> = names.iter().map(|(v, l)| (*v, l.as_str())).collect();
        cx.combo(
            tr!("ui-options-anisotropic-filtering"),
            tr!("ui-options-sharp-textures-viewed-at-an-angle"),
            &field!(anisotropy),
            &choices,
        );
        // RenderMaxTextureResolution (panel_preferences_graphics1.xml,
        // FSRestrictMaxTexturePixels): 512, 1024 or 2048 pixels
        let names: Vec<(u32, String)> = crate::settings::MAX_TEXTURE_RESOLUTIONS
            .into_iter()
            .map(|v| (v, tr!("ui-options-texture-resolution-pixels", size = v.to_string())))
            .collect();
        let choices: Vec<(u32, &str)> = names.iter().map(|(v, l)| (*v, l.as_str())).collect();
        cx.combo(
            tr!("ui-options-maximum-texture-resolution"),
            tr!("ui-options-world-textures-load-no-larger-more-fit-before-blurring"),
            &field!(max_texture_resolution),
            &choices,
        );
    });
    cx.group(tr!("ui-options-avatars"), |cx| {
        cx.slider(
            tr!("ui-options-fully-rendered-avatars"),
            tr!("ui-options-nearest-avatars-are-fully-rendered-others-use-impostors-periodically"),
            &field!(max_avatars),
            1..=100,
            "",
        );
        cx.row(
            tr!("ui-options-maximum-avatar-complexity"),
            tr!("ui-options-above-this-limit-the-avatar-appears-as-a-gray"),
            &field!(max_complexity),
            |ui, s| {
                use crate::scene::complexity::slider;
                let mut pos = slider::from_limit(s.max_complexity);
                let text = |v: f64| {
                    let limit = slider::to_limit(v.round() as u32);
                    if limit == 0 {
                        tr!("ui-options-unlimited").to_owned()
                    } else {
                        format!("{} k", limit / 1000)
                    }
                };
                let r = ui.add(
                    egui::Slider::new(&mut pos, 1..=slider::OFF)
                        .custom_parser(aurora_i18n::parse_number)
                        .custom_formatter(move |v, _| text(v))
                        .trailing_fill(true),
                );
                if r.changed() {
                    s.max_complexity = slider::to_limit(pos);
                }
                r.changed()
            },
        );
        cx.row(
            tr!("ui-options-maximum-worn-surface-area"),
            tr!("ui-options-above-this-limit-the-avatar-also-appears-as-a"),
            &field!(max_attachment_area),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.max_attachment_area, 0.0..=10_000.0)
                        .custom_parser(aurora_i18n::parse_number)
                        .logarithmic(true)
                        .step_by(10.0)
                        .custom_formatter(|v, _| {
                            if v < 1.0 {
                                tr!("ui-options-unlimited").to_owned()
                            } else {
                                format!("{v:.0} m²")
                            }
                        })
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        // ShowMyComplexityChanges: a debug setting only in the Legacy
        // Viewer, a row here next to the limits it is about
        cx.slider(
            tr!("ui-options-complexity-notices"),
            tr!("ui-options-complexity-notices-help"),
            &field!(complexity_notice_seconds),
            0..=60,
            " s",
        );
        cx.stepped(
            tr!("ui-options-loading-avatars"),
            tr!("ui-options-particle-cloud-until-the-avatar-and-its-attachments-load"),
            &field!(loading_avatars),
            &[tr!("ui-options-cloud-bar"), tr!("ui-options-bar-only"), tr!("common-nothing")],
        );
        cx.toggle(
            tr!("ui-options-avatar-cache"),
            tr!("ui-options-avatar-cache-help"),
            &field!(avatar_cache),
        );
        cx.stepped(
            tr!("ui-options-limit-applied"),
            tr!("ui-options-always-fully-render-friends-or-fully-render-only-friends"),
            &field!(complexity_mode),
            &[
                tr!("ui-options-to-everyone"),
                tr!("ui-options-except-friends"),
                tr!("ui-options-friends-only"),
            ],
        );
        cx.toggle(
            tr!("ui-options-avatar-physics"),
            tr!("ui-options-avatar-physics-help"),
            &field!(avatar_physics),
        );
        let off = !cx.s.avatar_physics;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-avatar-physics-detail"),
                tr!("ui-options-avatar-physics-detail-help"),
                &field!(avatar_physics_detail),
                |ui, s| {
                    ui.add(
                        egui::Slider::new(&mut s.avatar_physics_detail, 0.0..=1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .step_by(0.05)
                            .fixed_decimals(2)
                            .trailing_fill(true),
                    )
                    .changed()
                },
            );
        });
    });
    // the photo window's own switch, also here: a PC that crashes as soon
    // as the window opens with it on can still turn it off
    cx.group(tr!("photo-title"), |cx| {
        cx.toggle(tr!("photo-quality"), tr!("photo-quality-hint"), &field!(snapshot.quality));
        cx.toggle(
            tr!("photo-quality-off-after-crash"),
            tr!("photo-quality-off-after-crash-help"),
            &field!(snapshot.quality_off_after_crash),
        );
    });
}

fn shadow_tier_labels() -> [&'static str; 5] {
    use aurora_i18n::tr;
    [
        tr!("ui-options-baseline-diagnostic-off"),
        tr!("ui-options-low"),
        tr!("common-medium"),
        tr!("ui-options-high"),
        tr!("ui-options-ultra"),
    ]
}

fn shadow_tier_defaults(tier: crate::settings::shadow_profile::Tier) -> &'static crate::settings::Settings {
    use crate::settings::{Settings, shadow_profile};
    use std::sync::OnceLock;

    static TIERS: [OnceLock<Settings>; 5] = [const { OnceLock::new() }; 5];
    TIERS[tier as usize].get_or_init(|| {
        let mut defaults = Settings::default();
        shadow_profile::select_fixed_tier(&mut defaults, tier);
        defaults
    })
}

fn shadow_preferences(cx: &mut Cx) {
    use crate::settings::shadow_profile::{self as profile, Tier};
    use aurora_i18n::tr;
    // Preferences edit the underlying user choices even while Photo Quality
    // temporarily raises the map size and enables projectors for the camera.
    crate::snapshot::quality::begin_shadow_edit(cx.s);
    let changed_before = std::mem::take(&mut cx.changed);
    let levels = [Tier::Off, Tier::Low, Tier::Medium, Tier::High, Tier::Ultra];
    let labels = shadow_tier_labels();
    cx.group(tr!("ui-options-shadows"), |cx| {
        let palette = cx.p;
        cx.track(&field!(shadows));
        cx.track(&field!(shadow_quality));
        cx.track(&field!(shadow_resume_quality));
        cx.keywords(&labels.join(" "))
            .keywords(tr!("ui-options-preset-custom"))
            .own_reset()
            .wide()
            .row(
                tr!("ui-options-shadow-quality"),
                tr!("ui-shadow-profile-help"),
                &Custom {
                    path: "shadow_profile",
                    modified: |s: &crate::settings::Settings, d: &crate::settings::Settings| s.shadow_profile != d.shadow_profile,
                    reset: |s: &mut crate::settings::Settings, _: &crate::settings::Settings| profile::reset_all(s),
                },
                |ui, s| {
                    ui.vertical(|ui| {
                        let mut index = profile::tier(s) as u8;
                        let picked = ui.horizontal(|ui| stepped(ui, &palette, &mut index, &labels)).inner;
                        if picked {
                            profile::select_tier(s, levels[index as usize]);
                        }
                        // Keep the checkbox with its rollback at compact widths and larger text sizes.
                        let custom_changed = ui
                            .horizontal(|ui| {
                                let mut custom = profile::custom_enabled(s);
                                let toggled = crate::ui::widgets::checkbox(ui, &palette, &mut custom, tr!("ui-options-preset-custom"))
                                    .on_hover_text(tr!("ui-shadow-profile-help"))
                                    .changed();
                                if toggled {
                                    profile::set_custom_enabled(s, custom);
                                }
                                let tip = if index == Tier::Off as u8 {
                                    tr!("ui-shadow-profile-off-help").to_owned()
                                } else {
                                    tr!("ui-shadow-profile-reset", tier = labels[index as usize])
                                };
                                let reset = crate::ui::widgets::icon_button(ui, &palette, "restore", &tip, custom && profile::can_reset(s));
                                if reset {
                                    profile::reset_to_base(s);
                                }
                                toggled || reset
                            })
                            .inner;
                        picked || custom_changed
                    })
                    .inner
                },
            );
        cx.row(
            tr!("ui-options-shadow-intensity"),
            tr!("ui-options-shadow-intensity-help"),
            &field!(shadow_intensity),
            |ui, s| {
                ui.add_enabled(
                    s.shadows && s.shadow_quality > 0,
                    egui::Slider::new(&mut s.shadow_intensity, 0.0..=1.0)
                        .custom_parser(aurora_i18n::parse_number)
                        .fixed_decimals(2)
                        .step_by(0.05)
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        if profile::custom_enabled(cx.s) && profile::tier(cx.s) == Tier::Off {
            cx.note(tr!("ui-shadow-profile-off-help"));
        }
        if cx.s.shadows && profile::tier(cx.s) != Tier::Off && cx.s.projector_shadows {
            if let Some(maps) = cx.st.shadow_maps {
                cx.note(&tr!(
                    "ui-shadow-projector-active",
                    active = maps.active_projectors.to_string(),
                    limit = maps.projector_capacity.to_string(),
                    size = (maps.sun_resolution / 2).to_string()
                ));
            } else {
                cx.note(tr!("ui-shadow-projector-waiting"));
            }
        }
    });
    let tier = profile::tier(cx.s);
    // Legacy flat settings do not yet have a preset that can supply their defaults.
    let defaults = cx.s.shadow_profile.as_ref().map(|_| shadow_tier_defaults(tier));
    cx.gated_with_defaults(profile::custom_enabled(cx.s) && tier != Tier::Off, defaults, |cx| {
        cx.group(tr!("ui-options-preset-custom"), |cx| {
            cx.row(
                tr!("ui-shadow-profile-distance"),
                tr!("ui-shadow-profile-distance-help"),
                &Nothing,
                |ui, s| {
                    let names = [
                        tr!("ui-shadow-profile-near"),
                        tr!("ui-shadow-profile-balanced"),
                        tr!("ui-shadow-profile-far"),
                    ];
                    let choices = [0, 1, 2].map(|level| {
                        tr!(
                            "ui-shadow-profile-range",
                            name = names[usize::from(level)],
                            distance = profile::coverage_distance(s, level).to_string()
                        )
                    });
                    let selected = profile::active_coverage(s);
                    let current = selected.map_or_else(
                        || tr!("ui-shadow-profile-manual-range", distance = s.shadow_params().1.to_string()),
                        |level| choices[usize::from(level)].clone(),
                    );
                    let mut picked = None;
                    egui::ComboBox::from_id_salt(ui.scope_id().with("shadow-distance"))
                        .width(COMBO_W)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (level, label) in choices.iter().enumerate() {
                                let level = level as u8;
                                if ui.selectable_label(selected == Some(level), label.as_str()).clicked() {
                                    picked = Some(level);
                                }
                            }
                        });
                    if let Some(level) = picked {
                        profile::set_coverage(s, level);
                    }
                    picked.is_some()
                },
            );
            cx.toggle(
                tr!("ui-shadow-contact-hardening"),
                tr!("ui-shadow-contact-hardening-help"),
                &field!(sun_shadow_contact_hardening),
            );
            cx.enabled(cx.s.sun_shadow_contact_hardening, |cx| {
                cx.row(
                    tr!("ui-shadow-angular-diameter"),
                    tr!("ui-shadow-angular-diameter-help"),
                    &field!(sun_shadow_angular_diameter_deg),
                    |ui, s| {
                        ui.add(
                            egui::Slider::new(
                                &mut s.sun_shadow_angular_diameter_deg,
                                crate::settings::shadow_profile::SUN_DIAMETER_DEGREES,
                            )
                            .step_by(0.01)
                            .fixed_decimals(2)
                            .suffix("°")
                            .custom_parser(aurora_i18n::parse_number),
                        )
                        .changed()
                    },
                );
            });
            cx.toggle(
                tr!("ui-options-projector-shadows"),
                tr!("ui-options-projector-shadows-help"),
                &field!(projector_shadows),
            );
            cx.greyed(!cx.s.shadows || !cx.s.projector_shadows);
            cx.slider(
                tr!("ui-shadow-projector-limit"),
                tr!("ui-options-projector-shadow-limit-help"),
                &field!(projector_shadow_limit),
                1..=aurora_render::projector_shadows::MAX_PROJECTOR_SHADOWS as u8,
                "",
            );
            cx.greyed(false);
            let palette = cx.p;
            cx.row(tr!("common-advanced"), tr!("ui-shadow-profile-advanced-help"), &Nothing, |ui, s| {
                let mut open = profile::advanced_enabled(s);
                let checkbox = crate::ui::widgets::checkbox(ui, &palette, &mut open, "");
                checkbox.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), open, tr!("common-advanced")));
                if checkbox.changed() {
                    profile::set_advanced_enabled(s, open);
                }
                // Disclosure is transient and doesn't change the active shadow settings.
                false
            });
            if profile::advanced_enabled(cx.s) {
                cx.note(tr!("ui-shadow-profile-warning"));
                if profile::has_paused_advanced(cx.s) {
                    cx.wide().row(
                        tr!("ui-shadow-profile-paused"),
                        tr!("ui-shadow-profile-paused-help"),
                        &Nothing,
                        |ui, s| {
                            let restore = ui.button(tr!("ui-shadow-profile-apply-paused")).clicked();
                            if restore {
                                profile::restore_paused_advanced(s);
                            }
                            restore
                        },
                    );
                }
            }
        });
        cx.gated(profile::advanced_enabled(cx.s), advanced_shadow_preferences);
    });
    if cx.changed && profile::custom_enabled(cx.s) {
        profile::commit_edits(cx.s);
    }
    cx.changed |= changed_before;
    crate::snapshot::quality::end_shadow_edit(cx.s);
}

fn advanced_shadow_preferences(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-shadow-map-resolution"), |cx| {
        cx.keywords(tr!("ui-options-shadow-quality")).row(
            tr!("ui-shadow-map-resolution"),
            tr!("ui-shadow-map-resolution-help"),
            &Custom {
                path: "shadow_map_resolution",
                modified: |s: &crate::settings::Settings, d: &crate::settings::Settings| s.shadow_map_size() != d.shadow_map_size(),
                reset: |s: &mut crate::settings::Settings, d: &crate::settings::Settings| s.set_shadow_map_size(d.shadow_map_size()),
            },
            |ui, s| {
                let mut size = s.shadow_map_size();
                let changed = super::super::combo(
                    ui,
                    &mut size,
                    &[
                        (512, "512 × 512"),
                        (1024, "1024 × 1024"),
                        (2048, "2048 × 2048"),
                        (4096, "4096 × 4096"),
                    ],
                );
                if changed {
                    s.set_shadow_map_size(size);
                }
                changed
            },
        );
    });
    cx.group(tr!("ui-shadow-sun"), |cx| {
        cx.greyed(!cx.s.shadows);
        cx.combo(
            tr!("ui-shadow-layout"),
            tr!("ui-options-graded-sun-shadows-help"),
            &field!(graded_sun_shadows),
            &[(false, tr!("ui-shadow-three")), (true, tr!("ui-shadow-four"))],
        );
        let graded = cx.s.graded_sun_shadows;
        if graded {
            four_level_controls(cx);
        } else {
            three_level_controls(cx);
        }
        if cx.s.shadows {
            shadow_map_readout(cx);
        }
        let saved = if graded {
            tr!("ui-shadow-saved-three")
        } else {
            tr!("ui-shadow-saved-four")
        };
        cx.wide().row(saved, tr!("ui-shadow-saved-help"), &Nothing, |_, _| false);
        cx.subs(false, |cx| {
            if graded {
                three_level_controls(cx);
            } else {
                four_level_controls(cx);
            }
        });
    });
    cx.group(tr!("ui-shadow-contact-section"), |cx| {
        cx.greyed(!cx.s.shadows);
        cx.toggle(
            tr!("ui-options-contact-shadows"),
            tr!("ui-options-contact-shadows-help"),
            &field!(contact_shadows),
        );
        let contact = cx.s.contact_shadows;
        cx.conditional(contact, |cx| {
            cx.combo(
                tr!("ui-shadow-method"),
                tr!("ui-options-shadow-recovery-method-help"),
                &field!(contact_ray_method),
                &[(0, tr!("ui-shadow-bend")), (1, tr!("ui-shadow-view"))],
            );
            let bend = cx.s.contact_ray_method == 0;
            cx.conditional(bend, bend_controls);
            cx.conditional(!bend, view_ray_controls);
            cx.conditional(bend, |cx| {
                cx.toggle(
                    tr!("ui-shadow-adaptive-depth"),
                    tr!("ui-shadow-adaptive-depth-help"),
                    &field!(contact_integrity_depth),
                );
            });
            cx.toggle(
                tr!("ui-shadow-hole-repair"),
                tr!("ui-shadow-hole-repair-help"),
                &field!(contact_integrity_hole_repair),
            );
        });
    });
    cx.group(tr!("ui-shadow-diagnostics-section"), |cx| {
        cx.greyed(!cx.s.shadows);
        shadow_diagnostics(cx);
        if cx.s.shadows {
            shadow_legend(cx);
        }
    });
}

fn four_level_controls(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.slider(
        tr!("ui-options-graded-detail-range"),
        tr!("ui-options-graded-detail-range-help"),
        &field!(graded_shadow_detail_metres),
        2..=12,
        " m",
    );
    cx.row(
        tr!("ui-shadow-reach-four"),
        tr!("ui-options-graded-reach-help"),
        &field!(graded_shadow_distance),
        |ui, s| {
            ui.add(
                egui::Slider::new(&mut s.graded_shadow_distance, 16..=512)
                    .logarithmic(true)
                    .suffix(" m"),
            )
            .changed()
        },
    );
    cx.combo(
        tr!("ui-shadow-smoothing"),
        tr!("ui-shadow-smoothing-help"),
        &field!(graded_shadow_edge_smoothing),
        &[(false, tr!("ui-shadow-sharp")), (true, tr!("ui-shadow-mild"))],
    );
}

fn three_level_controls(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.row(
        tr!("ui-shadow-reach-three"),
        tr!("ui-options-graded-reach-help"),
        &Custom {
            path: "three_level_shadow_distance",
            modified: |s: &crate::settings::Settings, d: &crate::settings::Settings| {
                s.three_level_shadow_reach() != d.three_level_shadow_reach()
            },
            reset: |s: &mut crate::settings::Settings, d: &crate::settings::Settings| {
                s.three_level_shadow_distance = d.three_level_shadow_reach()
            },
        },
        |ui, s| {
            let mut reach = s.three_level_shadow_reach();
            let changed = ui
                .add(egui::Slider::new(&mut reach, 16..=512).logarithmic(true).suffix(" m"))
                .changed();
            if changed {
                s.three_level_shadow_distance = reach;
            }
            changed
        },
    );
    cx.toggle(
        tr!("ui-options-near-shadow-detail"),
        tr!("ui-options-near-shadow-detail-help"),
        &field!(near_shadow_detail),
    );
    cx.row(
        tr!("ui-options-near-shadow-detail-range"),
        tr!("ui-options-near-shadow-detail-range-help"),
        &field!(near_shadow_detail_metres),
        |ui, s| {
            ui.add_enabled(
                s.near_shadow_detail,
                egui::Slider::new(&mut s.near_shadow_detail_metres, 2..=12).suffix(" m"),
            )
            .changed()
        },
    );
    cx.toggle(
        tr!("ui-shadow-first-custom"),
        tr!("ui-shadow-first-custom-help"),
        &field!(near_shadow_precision),
    );
    cx.row(
        tr!("ui-shadow-first-share"),
        tr!("ui-shadow-first-share-help"),
        &field!(near_shadow_cascade_percent),
        |ui, s| {
            ui.add_enabled(
                s.near_shadow_precision,
                egui::Slider::new(&mut s.near_shadow_cascade_percent, 3..=12).suffix("%"),
            )
            .changed()
        },
    );
}

fn shadow_map_readout(cx: &mut Cx) {
    use aurora_i18n::tr;
    let Some(maps) = cx.st.shadow_maps else {
        cx.note(tr!("ui-shadow-no-measurement"));
        return;
    };
    let levels = || maps.detail.iter().chain(&maps.general);
    let splits = levels().map(|v| format!("{:.1}", v[0])).collect::<Vec<_>>().join(" / ");
    let texels = levels().map(|v| format!("{:.1}", 1000.0 * v[1])).collect::<Vec<_>>().join(" / ");
    let active = if maps.detail.is_some() {
        tr!("ui-shadow-four")
    } else {
        tr!("ui-shadow-three")
    };
    cx.note(&tr!(
        "ui-shadow-measured",
        mode = active,
        size = maps.sun_resolution.to_string(),
        splits = splits,
        texels = texels
    ));
    if maps.detail.is_some()
        && let Some(ms) = cx.st.graded_detail_gpu_ms
    {
        cx.note(&tr!("ui-options-graded-timing", ms = format!("{ms:.2}")));
    }
}

fn bend_controls(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.combo(
        tr!("ui-shadow-search-reach"),
        tr!("ui-shadow-search-help"),
        &field!(experimental_contact_reach_mode),
        &[
            (0, "60 px"),
            (1, "120 px"),
            (2, "240 px"),
            (3, "480 px"),
            (4, tr!("ui-shadow-adaptive")),
        ],
    );
    cx.conditional(cx.s.experimental_contact_reach_mode == 4, |cx| {
        cx.row(
            tr!("ui-shadow-adaptive-reach"),
            tr!("ui-shadow-adaptive-reach-help"),
            &field!(experimental_contact_reach_metres),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.experimental_contact_reach_metres, 0.10..=1.20)
                        .suffix(" m")
                        .step_by(0.05),
                )
                .changed()
            },
        );
    });
    cx.row(
        tr!("ui-shadow-occluder-depth"),
        tr!("ui-shadow-occluder-depth-help"),
        &field!(experimental_contact_occluder_depth_metres),
        |ui, s| {
            ui.add(
                egui::Slider::new(&mut s.experimental_contact_occluder_depth_metres, 0.0..=0.30)
                    .suffix(" m")
                    .step_by(0.01),
            )
            .changed()
        },
    );
}

fn view_ray_controls(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.row(
        tr!("ui-shadow-ray-reach"),
        tr!("ui-shadow-ray-reach-help"),
        &field!(contact_view_reach),
        |ui, s| {
            ui.add(egui::Slider::new(&mut s.contact_view_reach, 0.10..=2.0).suffix(" m").step_by(0.05))
                .changed()
        },
    );
    cx.row(
        tr!("ui-shadow-ray-tolerance"),
        tr!("ui-shadow-ray-tolerance-help"),
        &field!(contact_view_thickness),
        |ui, s| {
            ui.add(
                egui::Slider::new(&mut s.contact_view_thickness, 0.01..=0.30)
                    .suffix(" m")
                    .step_by(0.01),
            )
            .changed()
        },
    );
    cx.slider(
        tr!("ui-shadow-ray-budget"),
        tr!("ui-options-shadow-recovery-budget-help"),
        &field!(contact_view_budget),
        48..=128,
        "",
    );
}

fn shadow_legend(cx: &mut Cx) {
    use aurora_i18n::tr;
    use aurora_render::render_controls::ContactDiagnostic;
    match saved_diagnostic(cx.s) {
        ContactDiagnostic::Cascades => {
            let four = cx.st.shadow_maps.map_or(cx.s.graded_sun_shadows, |m| m.detail.is_some());
            cx.note(if four {
                tr!("ui-shadow-legend-four")
            } else {
                tr!("ui-shadow-legend-three")
            });
        }
        ContactDiagnostic::ShadowMapVisibility | ContactDiagnostic::ContactVisibility | ContactDiagnostic::CombinedVisibility => {
            cx.note(tr!("ui-shadow-legend-visibility"))
        }
        ContactDiagnostic::Mask => cx.note(tr!("ui-shadow-legend-mask")),
        ContactDiagnostic::ViewRayOutcomes => cx.note(tr!("ui-shadow-legend-rays")),
        ContactDiagnostic::BendCandidates => cx.note(tr!("ui-shadow-legend-candidates")),
        ContactDiagnostic::BendFailures => cx.note(tr!("ui-shadow-legend-failures")),
        ContactDiagnostic::Off => {}
    }
}

fn light(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-local-lights"), |cx| {
        // the Legacy Viewer's "Max. nearby lights" (RenderLocalLightCount),
        // up to the 64 the renderer holds (Settings::local_light_count)
        cx.slider(
            tr!("ui-options-max-nearby-lights"),
            tr!("ui-options-max-nearby-lights-help"),
            &field!(local_light_count),
            0..=crate::scene::lights::MAX_LIGHTS as u32,
            "",
        );
        cx.toggle(
            tr!("ui-options-others-attached-lights"),
            tr!("ui-options-others-attached-lights-help"),
            &field!(others_attached_lights),
        );
    });
    shadow_preferences(cx);
    cx.group(tr!("ui-options-ambient-occlusion"), |cx| {
        cx.stepped(
            tr!("ui-options-ambient-occlusion"),
            tr!("ui-options-darkens-corners-and-contact-areas-ssao"),
            &field!(ssao),
            &[tr!("ui-options-disabled"), tr!("ui-options-low"), tr!("ui-options-high")],
        );
        let off = cx.s.ssao == 0;
        cx.subs(off, |cx| {
            cx.combo(
                tr!("ui-options-ao-method"),
                tr!("ui-options-ao-method-help"),
                &field!(ao_method),
                &[(0, tr!("ui-options-ao-legacy")), (1, "GTAO")],
            );
            let gtao = cx.s.ao_method == 1;
            let p = cx.p;
            let levels = [
                tr!("ui-options-low"),
                tr!("common-medium"),
                tr!("ui-options-high"),
                tr!("ui-options-ultra"),
            ];
            cx.keywords(&levels.join(" ")).row(
                tr!("ui-options-gtao-quality"),
                tr!("ui-options-gtao-quality-help"),
                &field!(gtao_quality),
                |ui, s| ui.add_enabled_ui(gtao, |ui| stepped(ui, &p, &mut s.gtao_quality, &levels)).inner,
            );
            cx.experimental().row(
                tr!("ui-options-p3-ao-thin"),
                tr!("ui-options-p3-ao-thin-help"),
                &field!(experimental_gtao_thin_compensation),
                |ui, s| {
                    ui.add_enabled(
                        gtao,
                        egui::Slider::new(&mut s.experimental_gtao_thin_compensation, 0.0..=1.5)
                            .step_by(0.05)
                            .trailing_fill(true),
                    )
                    .changed()
                },
            );
            cx.experimental().row(
                tr!("ui-options-screen-indirect-light"),
                tr!("ui-options-screen-indirect-light-help"),
                &field!(screen_indirect_light),
                |ui, s| ui.add_enabled_ui(gtao, |ui| toggle(ui, &p, &mut s.screen_indirect_light)).inner,
            );
            let on = gtao && cx.s.screen_indirect_light;
            cx.row(
                tr!("ui-options-screen-indirect-intensity"),
                tr!("ui-options-screen-indirect-intensity-help"),
                &field!(screen_indirect_intensity),
                |ui, s| {
                    ui.add_enabled_ui(on, |ui| {
                        ui.add(
                            egui::Slider::new(&mut s.screen_indirect_intensity, 0.0..=4.0)
                                .custom_parser(aurora_i18n::parse_number)
                                .step_by(0.1)
                                .trailing_fill(true),
                        )
                        .changed()
                    })
                    .inner
                },
            );
        });
    });
    // All controls are independent of the PBR material's actual asset.
    cx.group(tr!("ui-options-p3-thin-group"), |cx| {
        cx.experimental().toggle(
            tr!("ui-options-p3-thin-enable"),
            tr!("ui-options-p3-thin-enable-help"),
            &field!(pbr_thin_enabled),
        );
        let enabled = cx.s.pbr_thin_enabled;
        cx.subs(!enabled, |cx| {
            cx.combo(
                tr!("ui-options-p3-thin-scope"),
                tr!("ui-options-p3-thin-scope-help"),
                &field!(pbr_thin_scope),
                &[
                    (0, tr!("ui-options-p3-thin-scope-uuid")),
                    (1, tr!("ui-options-p3-thin-scope-preview")),
                ],
            );
            let specific = cx.s.pbr_thin_scope == 0;
            cx.row(
                tr!("ui-options-p3-thin-uuid"),
                tr!("ui-options-p3-thin-uuid-help"),
                &field!(pbr_thin_material_id),
                |ui, s| {
                    ui.add_enabled(
                        specific,
                        egui::TextEdit::singleline(&mut s.pbr_thin_material_id).desired_width(270.0),
                    )
                    .changed()
                },
            );
            cx.row(
                tr!("ui-options-p3-thin-ao"),
                tr!("ui-options-p3-thin-ao-help"),
                &field!(pbr_thin_ao_floor),
                |ui, s| {
                    ui.add(
                        egui::Slider::new(&mut s.pbr_thin_ao_floor, 0.0..=0.8)
                            .step_by(0.05)
                            .trailing_fill(true),
                    )
                    .changed()
                },
            );
            cx.row(
                tr!("ui-options-p3-thin-transmission"),
                tr!("ui-options-p3-thin-transmission-help"),
                &field!(pbr_thin_transmission),
                |ui, s| {
                    ui.add(
                        egui::Slider::new(&mut s.pbr_thin_transmission, 0.0..=1.5)
                            .step_by(0.05)
                            .trailing_fill(true),
                    )
                    .changed()
                },
            );
            cx.toggle(
                tr!("ui-options-p3-thin-highlight"),
                tr!("ui-options-p3-thin-highlight-help"),
                &field!(pbr_thin_debug),
            );
        });
    });
    cx.group(tr!("ui-options-exposure-and-glow"), |cx| {
        // greyed without HDR, as RenderExposure in the Legacy Viewer
        // (LLFloaterPreference::refreshEnabledState)
        let hdr = cx.s.hdr_emissive;
        cx.row(tr!("ui-options-exposure"), "", &field!(exposure), |ui, s| {
            let r = ui.add_enabled_ui(hdr, |ui| {
                ui.add(
                    egui::Slider::new(&mut s.exposure, 0.25..=4.0)
                        .custom_parser(aurora_i18n::parse_number)
                        .logarithmic(true)
                        .trailing_fill(true),
                )
                .changed()
            });
            r.response.on_disabled_hover_text(tr!("ui-options-needs-hdr-emissive"));
            r.inner
        });
        cx.toggle(
            tr!("ui-options-glow"),
            tr!("ui-options-glow-around-glowing-faces-as-in-firestorm"),
            &field!(glow),
        );
        cx.experimental()
            .toggle(tr!("ui-options-bloom"), tr!("ui-options-bloom-help"), &field!(bloom));
        let off = !cx.s.bloom;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-bloom-strength"),
                tr!("ui-options-bloom-strength-help"),
                &field!(bloom_strength),
                |ui, s| {
                    ui.add(
                        egui::Slider::new(&mut s.bloom_strength, 0.05..=1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .step_by(0.05)
                            .trailing_fill(true),
                    )
                    .changed()
                },
            );
        });
    });
}

fn reflections_and_water(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-reflections"), |cx| {
        let levels = [
            tr!("ui-options-disabled-3"),
            tr!("ui-options-low-14"),
            tr!("ui-options-medium-12"),
            tr!("ui-options-high-34"),
            tr!("ui-options-ultra-full"),
        ];
        cx.stepped(
            tr!("ui-options-water-reflections"),
            tr!("ui-options-planar-scene-reflection-high-and-ultra-are-costly"),
            &field!(water_reflections),
            &levels,
        );
        cx.stepped(
            tr!("ui-options-mirrors"),
            tr!("ui-options-sl-mirror-probes-render-resolution-low-by-default"),
            &field!(mirrors),
            &levels,
        );
        cx.toggle(
            tr!("ui-options-screen-space-reflections-ssr"),
            tr!("ui-options-shiny-surfaces-expensive-disabled-by-default"),
            &field!(ssr),
        );
        cx.stepped(
            tr!("common-reflection-probes"),
            tr!("ui-options-what-shiny-surfaces-reflect-as-in-firestorm"),
            &field!(reflection_probes),
            &[
                tr!("ui-options-sky-only"),
                tr!("ui-options-manual"),
                tr!("ui-options-manual-terrain"),
                tr!("ui-options-entire-scene"),
            ],
        );
        cx.row(
            tr!("ui-options-probe-count"),
            tr!("ui-options-active-probes-around-you-video-memory"),
            &field!(probe_count),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.probe_count, 8..=256)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(8.0)
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        cx.row(
            tr!("ui-options-probe-range"),
            tr!("ui-options-probe-range-help"),
            &field!(probe_range),
            |ui, s| {
                ui.add(
                    egui::Slider::new(
                        &mut s.probe_range,
                        crate::settings::PROBE_RANGE_MIN..=crate::settings::DRAW_DISTANCE_MAX,
                    )
                    .custom_parser(aurora_i18n::parse_number)
                    .step_by(16.0)
                    .suffix(" m")
                    .trailing_fill(true),
                )
                .changed()
            },
        );
    });
    // Aurora's own water effects, experimental and off by default
    cx.group(tr!("ui-options-aurora-water"), |cx| {
        cx.experimental().toggle(
            tr!("water-effects-enabled"),
            tr!("water-effects-help"),
            &field!(water_effects.enabled),
        );
        let off = !cx.s.water_effects.enabled;
        cx.subs(off, |cx| {
            cx.stepped(
                tr!("water-effects-quality"),
                tr!("water-effects-quality-help"),
                &field!(water_effects.quality),
                &[
                    tr!("ui-options-low"),
                    tr!("water-effects-medium"),
                    tr!("ui-options-high"),
                    tr!("ui-options-ultra"),
                ],
            );
            cx.toggle(tr!("water-effects-waves"), "", &field!(water_effects.waves));
            cx.toggle(tr!("water-effects-absorption"), "", &field!(water_effects.absorption));
            cx.toggle(tr!("water-effects-fresnel"), "", &field!(water_effects.fresnel));
            cx.toggle(tr!("water-effects-shore"), "", &field!(water_effects.shore_foam));
            cx.toggle(tr!("water-effects-wet-ground"), "", &field!(water_effects.wet_ground));
            cx.toggle(tr!("water-effects-contact"), "", &field!(water_effects.contact_foam));
            cx.toggle(tr!("water-effects-crests"), "", &field!(water_effects.crest_foam));
            cx.toggle(tr!("water-effects-interactions"), "", &field!(water_effects.interactions));
            cx.toggle(tr!("water-effects-caustics"), "", &field!(water_effects.caustics));
            cx.toggle(tr!("water-effects-splashes"), "", &field!(water_effects.splashes));
            cx.toggle(tr!("water-effects-bubbles"), "", &field!(water_effects.bubbles));
        });
    });
}

fn image(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-anti-aliasing"), |cx| {
        cx.combo(
            tr!("ui-options-anti-aliasing"),
            tr!("ui-options-msaa-sharp-but-costly-fxaa-lightweight-smaa-lightweight-and"),
            &field!(antialiasing),
            &[
                (0, tr!("common-none")),
                (1, "FXAA"),
                (2, "MSAA 2×"),
                (3, "MSAA 4×"),
                (4, "MSAA 8×"),
                (5, "TAA"),
                (6, "SMAA"),
                (7, "FSR 3.1"),
            ],
        );
        cx.toggle(
            tr!("ui-options-specular-aa"),
            tr!("ui-options-specular-aa-help"),
            &field!(specular_aa),
        );
    });
    cx.group(tr!("ui-options-p3-legacy-group"), |cx| {
        cx.experimental().toggle(
            tr!("ui-options-p3-legacy-enable"),
            tr!("ui-options-p3-legacy-enable-help"),
            &field!(legacy_light_harmonization),
        );
        cx.subs(!cx.s.legacy_light_harmonization, |cx| {
            cx.toggle(
                tr!("ui-options-p3-legacy-sun"),
                tr!("ui-options-p3-legacy-sun-help"),
                &field!(harmonize_sunlight),
            );
            cx.toggle(
                tr!("ui-options-p3-legacy-ambient"),
                tr!("ui-options-p3-legacy-ambient-help"),
                &field!(harmonize_ambient),
            );
            cx.toggle(
                tr!("ui-options-p3-legacy-local"),
                tr!("ui-options-p3-legacy-local-help"),
                &field!(harmonize_local_lights),
            );
        });
    });
    // the order of the blended faces (scene/blend_order.rs), beyond the
    // Legacy Viewer's sort
    cx.group(tr!("ui-options-transparency"), |cx| {
        let p = cx.p;
        // the search finds the row by its methods and the exact order's
        // precisions
        let mut words = vec![
            tr!("ui-options-blend-order-standard"),
            tr!("ui-options-blend-order-exact"),
            tr!("ui-options-blend-order-stochastic"),
        ];
        words.extend(blend_precisions().map(|(_, label)| label));
        cx.wide().keywords(&words.join(" ")).row(
            tr!("ui-options-blend-order"),
            tr!("ui-options-blend-order-help"),
            &field!(blend_order),
            |ui, s| blend_order_choice(ui, &p, &mut s.blend_order),
        );
    });
    // FSR's own settings (aurora_render fsr.rs), greyed under another
    // anti-aliasing
    cx.group(tr!("ui-options-fsr"), |cx| {
        cx.note(tr!("ui-options-fsr-note"));
        cx.greyed(cx.s.antialiasing != 7);
        cx.combo(
            tr!("ui-options-fsr-quality"),
            tr!("ui-options-fsr-quality-help"),
            &field!(fsr_quality),
            &[
                (0, tr!("ui-options-fsr-native-aa")),
                (1, tr!("ui-options-fsr-quality-mode")),
                (2, tr!("ui-options-fsr-balanced")),
                (3, tr!("ui-options-fsr-performance")),
                (4, tr!("ui-options-fsr-ultra-performance")),
            ],
        );
        cx.slider(
            tr!("ui-options-fsr-sharpness"),
            tr!("ui-options-fsr-sharpness-help"),
            &field!(fsr_sharpness),
            0..=100,
            "%",
        );
    });
    cx.group(tr!("ui-options-color-and-sharpness"), |cx| {
        // "HDR and Emissive" (RenderDisableVintageMode): off, the tone
        // mapping, its mix and the sharpening are greyed, as in
        // LLFloaterPreference::refreshEnabledState
        cx.keywords("HDR Emissive Vintage").toggle(
            tr!("ui-options-hdr-emissive"),
            tr!("ui-options-hdr-emissive-help"),
            &field!(hdr_emissive),
        );
        let hdr = cx.s.hdr_emissive;
        // a classic sky is not tone mapped (no_post): the choice would
        // change nothing
        let mapped = cx.st.sky_tone_mapped && hdr;
        let why = if hdr {
            tr!("ui-options-tone-mapping-classic-sky")
        } else {
            tr!("ui-options-needs-hdr-emissive")
        };
        cx.keywords("ACES Khronos PBR Neutral").row(
            tr!("ui-options-tone-mapping"),
            tr!("ui-options-tone-mapping-help"),
            &field!(tonemapper),
            |ui, s| {
                let mut changed = false;
                ui.add_enabled_ui(mapped, |ui| {
                    changed = super::super::combo(ui, &mut s.tonemapper, &[(0, tr!("ui-options-khronos-pbr-neutral")), (1, "ACES")]);
                })
                .response
                .on_disabled_hover_text(why);
                changed
            },
        );
        cx.row(
            tr!("ui-options-tone-mapping-mix"),
            tr!("ui-options-tone-mapping-mix-help"),
            &field!(tonemap_mix),
            |ui, s| {
                let r = ui.add_enabled_ui(mapped, |ui| {
                    ui.add(
                        egui::Slider::new(&mut s.tonemap_mix, 0.0..=1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .step_by(0.1)
                            .fixed_decimals(1)
                            .trailing_fill(true),
                    )
                    .changed()
                });
                r.response.on_disabled_hover_text(why);
                r.inner
            },
        );
        cx.row(
            tr!("ui-options-sharpness"),
            tr!("ui-options-enhances-fine-details-amd-cas-0-4-as-in"),
            &field!(sharpen),
            |ui, s| {
                // FSR's RCAS takes its place: both on top of each other
                // turn FSR's faint sub-pixel pattern into a visible one
                let fsr = s.antialiasing == 7;
                let r = ui.add_enabled_ui(hdr && !fsr, |ui| {
                    ui.add(
                        egui::Slider::new(&mut s.sharpen, 0.0..=1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .step_by(0.05)
                            .trailing_fill(true),
                    )
                    .changed()
                });
                r.response.on_disabled_hover_text(if fsr {
                    tr!("ui-options-sharpness-off-with-fsr")
                } else {
                    tr!("ui-options-needs-hdr-emissive")
                });
                r.inner
            },
        );
    });
    world_look(cx);
    depth_of_field(cx);
}

/// The world's own filters (`snapshot::looks::WorldLook`): switched on here,
/// chosen in their window (`ui/world_look.rs`). Aurora's own.
fn world_look(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-world-look"), |cx| {
        cx.keywords("LUT look filter colour grading").toggle(
            tr!("ui-options-world-look-on"),
            tr!("ui-options-world-look-on-help"),
            &field!(world_look.on),
        );
        let p = cx.p;
        cx.row_with(
            tr!("ui-options-world-look-choose"),
            tr!("ui-options-world-look-choose-help"),
            &Nothing,
            |ui, _, _, r| {
                use crate::ui::widgets::{ActionKind, action_button};
                if action_button(
                    ui,
                    &p,
                    "color-filter",
                    tr!("ui-options-world-look-open"),
                    ActionKind::Flat,
                    egui::Vec2::new(0.0, 24.0),
                )
                .clicked()
                {
                    r.open_world_look = true;
                }
                false
            },
        );
    });
}

/// Depth of field (panel_preferences_graphics1.xml, "DOF panel":
/// RenderDepthOfField, RenderDepthOfFieldInEditMode,
/// FSFocusPointFollowsPointer, CameraFNumber, CameraFocalLength,
/// CameraFieldOfView, CameraFocusTransitionTime, CameraDoFResScale, with
/// Phototools' CameraMaxCoF and focus lock).
fn depth_of_field(cx: &mut Cx) {
    use crate::dof::FocusMode;
    use aurora_i18n::tr;
    /// A slider whose steps grow with its value (f-numbers, focal lengths).
    fn log_slider(
        ui: &mut egui::Ui,
        v: &mut f32,
        range: std::ops::RangeInclusive<f32>,
        prefix: &str,
        suffix: &str,
        decimals: usize,
    ) -> bool {
        ui.add(
            egui::Slider::new(v, range)
                .logarithmic(true)
                .prefix(prefix)
                .suffix(suffix)
                .fixed_decimals(decimals)
                .custom_parser(aurora_i18n::parse_number)
                .trailing_fill(true),
        )
        .changed()
    }
    fn plain_slider(ui: &mut egui::Ui, v: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, decimals: usize) -> bool {
        ui.add(
            egui::Slider::new(v, range)
                .suffix(suffix)
                .fixed_decimals(decimals)
                .custom_parser(aurora_i18n::parse_number)
                .trailing_fill(true),
        )
        .changed()
    }
    cx.group(tr!("ui-options-dof"), |cx| {
        cx.toggle(
            tr!("ui-options-dof-enable"),
            tr!("ui-options-dof-enable-help"),
            &field!(dof.enabled),
        );
        // The Photo lens can use this preference while the world lens is off.
        cx.toggle(
            tr!("ui-options-dof-blended-depth"),
            tr!("ui-options-dof-blended-depth-help"),
            &field!(dof.experimental_alpha_depth_proxy),
        );
        let off = !cx.s.dof.enabled;
        cx.subs(off, |cx| {
            cx.toggle(tr!("ui-options-dof-edit"), tr!("ui-options-dof-edit-help"), &field!(dof.in_edit));
            cx.choice(
                tr!("ui-options-dof-focus"),
                tr!("ui-options-dof-focus-help"),
                &field!(dof.focus),
                &[
                    (FocusMode::Camera, tr!("ui-options-dof-focus-camera"), true),
                    (FocusMode::Pointer, tr!("ui-options-dof-focus-pointer"), true),
                    (FocusMode::Click, tr!("ui-options-dof-focus-click"), true),
                    (FocusMode::Locked, tr!("ui-options-dof-focus-locked"), true),
                ],
            );
            cx.toggle(
                tr!("ui-options-dof-show-focus-point"),
                tr!("ui-options-dof-show-focus-point-help"),
                &field!(dof.show_focus_point),
            );
            cx.row(
                tr!("ui-options-dof-aperture"),
                tr!("ui-options-dof-aperture-help"),
                &field!(dof.f_number),
                |ui, s| log_slider(ui, &mut s.dof.f_number, 1.0..=64.0, "f/", "", 1),
            );
            cx.row(
                tr!("ui-options-dof-focal-length"),
                tr!("ui-options-dof-focal-length-help"),
                &field!(dof.focal_length),
                |ui, s| log_slider(ui, &mut s.dof.focal_length, 1.0..=1000.0, "", " mm", 0),
            );
            cx.row(
                tr!("ui-options-dof-reference-fov"),
                tr!("ui-options-dof-reference-fov-help"),
                &field!(dof.reference_fov),
                |ui, s| plain_slider(ui, &mut s.dof.reference_fov, 10.0..=200.0, "°", 0),
            );
            cx.row(
                tr!("ui-options-dof-max-blur"),
                tr!("ui-options-dof-max-blur-help"),
                &field!(dof.max_coc),
                |ui, s| plain_slider(ui, &mut s.dof.max_coc, 1.0..=64.0, " px", 0),
            );
            cx.row(
                tr!("ui-options-dof-transition"),
                tr!("ui-options-dof-transition-help"),
                &field!(dof.transition),
                |ui, s| plain_slider(ui, &mut s.dof.transition, 0.1..=10.0, " s", 2),
            );
            cx.row(
                tr!("ui-options-dof-quality"),
                tr!("ui-options-dof-quality-help"),
                &field!(dof.quality),
                |ui, s| plain_slider(ui, &mut s.dof.quality, 0.25..=1.0, "", 2),
            );
            cx.toggle(
                tr!("ui-options-p3-dof-smooth"),
                tr!("ui-options-p3-dof-smooth-help"),
                &field!(dof.experimental_near_transition),
            );
            cx.row(
                tr!("ui-options-p3-dof-strength"),
                tr!("ui-options-p3-dof-strength-help"),
                &field!(dof.experimental_near_strength),
                |ui, s| plain_slider(ui, &mut s.dof.experimental_near_strength, 0.0..=1.0, "", 2),
            );
            cx.toggle(
                tr!("ui-options-p3-dof-zones"),
                tr!("ui-options-p3-dof-zones-help"),
                &field!(dof.experimental_dof_zones),
            );
        });
    });
}

/// The environments saved for parcels (`world/parcel_env.rs`), set from the
/// Environment window and the inventory: this list only removes them. They
/// are the account's on this grid, in its own file, not settings: the app
/// lists them in `OptionsUi::parcel_environments` and removes them.
fn parcel_environments(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.section_body().row_with(
        tr!("parcel-environment-title"),
        tr!("parcel-environment-help"),
        &Nothing,
        |ui, _, st, r| {
            ui.vertical(|ui| {
                let entries = &st.parcel_environments;
                if entries.is_empty() {
                    ui.label(RichText::new(tr!("parcel-environment-empty")).color(p.muted));
                    return;
                }
                let remove = parcel_rows(ui, &p, "parcel_environments", entries, |e| {
                    ((e.region, e.local_id), &e.name, &e.region_name)
                });
                if let Some(e) = remove.and_then(|i| entries.get(i)) {
                    r.forget_parcel_environment = Some((e.region, e.local_id));
                }
                if ui.button(tr!("parcel-environment-remove-all")).clicked() {
                    st.confirm_remove_parcel_environments = true;
                }
            });
            false
        },
    );
}

/// Scope and rendering exceptions use the same saved identities and list controls.
fn parcel_list(
    cx: &mut Cx,
    label: &str,
    hint: &str,
    field: &super::super::cx::F<Vec<crate::world::parcel_visibility::Entry>>,
    selected: bool,
) {
    use aurora_i18n::tr;
    let p = cx.p;
    let get = field.get_mut;
    cx.section_body().row_with(label, hint, field, |ui, s, st, _| {
        let entries = get(s);
        let mut changed = false;
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                for (entry, label) in std::iter::once((st.current_parcel.as_ref(), tr!("parcel-visibility-add")))
                    .chain(selected.then_some((st.selected_parcel.as_ref(), tr!("parcel-visibility-add-selected"))))
                {
                    let listed = entry.is_some_and(|p| entries.iter().any(|saved| saved.same_parcel(p)));
                    let response = ui.add_enabled(entry.is_some() && !listed, egui::Button::new(label));
                    if let Some(parcel) = entry {
                        let response = response.on_hover_text(format!("{} — {}", parcel.name, parcel.region_name));
                        if response.clicked() {
                            entries.push(parcel.clone());
                            changed = true;
                        }
                    }
                }
            });
            if entries.is_empty() {
                ui.label(RichText::new(tr!("parcel-visibility-empty")).color(p.muted));
                return;
            }

            let remove = parcel_rows(ui, &p, field.path, entries, |e| ((e.region, e.local_id), &e.name, &e.region_name));
            if let Some(i) = remove {
                entries.remove(i);
                changed = true;
            }
        });
        changed
    });
}

/// The rows of a saved parcel list: the parcel's name over its region, and a
/// remove button. Returns the index of the row whose button was clicked.
/// Shared by the scope and rendering exceptions (`parcel_list`) and the
/// parcels' saved environments (`parcel_environments`).
fn parcel_rows<'a, T, K: egui::AsIdSalt>(
    ui: &mut egui::Ui,
    p: &crate::theme::Palette,
    salt: impl egui::AsIdSalt,
    entries: &'a [T],
    row: impl Fn(&'a T) -> (K, &'a str, &'a str),
) -> Option<usize> {
    use crate::ui::widgets;
    use aurora_i18n::tr;
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .max_height(180.0)
        // Cx measures controls from a short first-frame placeholder.
        // Let the list establish its height, then shrink to its content.
        .min_scrolled_height(180.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, entry) in entries.iter().enumerate() {
                let (key, name, region) = row(entry);
                ui.push_id(key, |ui| {
                    ui.horizontal_top(|ui| {
                        let width = (ui.available_width() - 20.0 - ui.spacing().item_spacing.x).max(1.0);
                        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_min_width(width);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(egui::Label::new(RichText::new(name).strong()).wrap());
                            ui.add(egui::Label::new(RichText::new(region).size(12.0).color(p.muted)).wrap());
                        });
                        if widgets::icon_button(ui, p, "trash-filled", tr!("common-remove"), true) {
                            remove = Some(i);
                        }
                    });
                });
            }
        });
    remove
}
