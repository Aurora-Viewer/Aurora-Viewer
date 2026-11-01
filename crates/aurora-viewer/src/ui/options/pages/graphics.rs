//! Preferences › Graphics (panel_preferences_graphics1.xml and its
//! advanced floater): the level of the whole, then quality, light, the sky,
//! reflections and Aurora's water, the image (anti-aliasing, tone mapping,
//! depth of field), smoothness and parcel visibility.

use super::super::cx::{Cx, Nothing, field};
use super::super::{COMBO_W, preset_choice, stepped, toggle};
use egui::RichText;

pub fn page(cx: &mut Cx, tab: usize) {
    use aurora_i18n::tr;
    match tab {
        0 => quality(cx),
        1 => light(cx),
        2 => cx.group(tr!("common-environment"), |cx| {
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
        }),
        3 => reflections_and_water(cx),
        4 => image(cx),
        5 => {
            frames_per_second(cx);
            graphics_card(cx);
            overlays(cx);
        }
        6 => parcels(cx),
        _ => {}
    }
}

/// The GPU the viewer draws with (#894): automatic (discrete first) or one
/// by name; the renderer opens it at startup, so the choice applies at the
/// next start. `AURORA_GPU` overrides it.
/// The real sky's rows (world/night_sky.rs): where it is seen from, its
/// latitude (set by a preset, typed for Custom; moving it makes the place
/// Custom) and the stars' brightness.
fn night_sky_rows(cx: &mut Cx) {
    use crate::world::night_sky::{GAIN_RANGE, SkyPlace};
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
                )
                .0
            },
        );
    });
}

/// Other programs' overlays (Steam, RivaTuner, Epic…): their Vulkan layers
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
        cx.slider(
            tr!("ui-options-render-scale"),
            tr!("ui-options-render-scale-help"),
            &field!(render_scale_percent),
            50..=100,
            "%",
        );
        // across, its switch in line with those of the limits below
        cx.wide().toggle(
            tr!("ui-options-vertical-synchronization"),
            tr!("ui-options-synchronizes-frames-with-the-monitor-refresh-rate-no-tearing"),
            &field!(vsync),
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
}

fn quality(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-presets"), |cx| {
        let p = cx.p;
        cx.wide().keywords(tr!("ui-options-preset-custom")).row(
            tr!("ui-options-preset-level"),
            tr!("ui-options-preset-level-help"),
            // a level writes the page's settings: they are the rows below
            &Nothing,
            |ui, s| preset_choice(ui, &p, s),
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
            32.0..=512.0,
            " m",
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
                let (vram, integrated) = cx.st.vram;
                let budget = crate::settings::auto_texture_budget(vram, integrated);
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
        cx.stepped(
            tr!("ui-options-loading-avatars"),
            tr!("ui-options-particle-cloud-until-the-avatar-and-its-attachments-load"),
            &field!(loading_avatars),
            &[tr!("ui-options-cloud-bar"), tr!("ui-options-bar-only"), tr!("common-nothing")],
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
}

fn light(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-shadows"), |cx| {
        let p = cx.p;
        let levels = [
            tr!("ui-options-disabled-2"),
            tr!("ui-options-low"),
            tr!("common-medium"),
            tr!("ui-options-high"),
            tr!("ui-options-ultra"),
        ];
        cx.keywords(&levels.join(" ")).row(
            tr!("ui-options-shadow-quality"),
            tr!("ui-options-cascaded-shadow-resolution-and-distance"),
            &field!(shadow_quality),
            |ui, s| {
                let changed = stepped(ui, &p, &mut s.shadow_quality, &levels);
                s.shadows = s.shadow_quality > 0;
                changed
            },
        );
        // the switch follows the quality (`OptionsUi::after_reset` too)
        cx.track(&field!(shadows));
        let off = cx.s.shadow_quality == 0;
        cx.subs(off, |cx| {
            cx.toggle(
                tr!("ui-options-projector-shadows"),
                tr!("ui-options-projector-shadows-help"),
                &field!(projector_shadows),
            );
            cx.toggle(
                tr!("ui-options-contact-shadows"),
                tr!("ui-options-contact-shadows-help"),
                &field!(contact_shadows),
            );
        });
    });
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
                    egui::Slider::new(&mut s.probe_count, 8..=64)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(8.0)
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
            ],
        );
        cx.toggle(
            tr!("ui-options-specular-aa"),
            tr!("ui-options-specular-aa-help"),
            &field!(specular_aa),
        );
    });
    // Order-independent transparency (aurora-render oit.rs), an Aurora addition
    cx.group(tr!("ui-options-transparency"), |cx| {
        cx.greyed(!cx.st.oit_supported);
        cx.toggle(tr!("ui-options-oit"), tr!("ui-options-oit-help"), &field!(oit));
        let off = !cx.s.oit;
        cx.subs(off, |cx| {
            cx.slider(
                tr!("ui-options-oit-nodes"),
                tr!("ui-options-oit-nodes-help"),
                &field!(oit_nodes_per_pixel),
                aurora_render::oit::MIN_NODES_PER_PIXEL..=aurora_render::oit::MAX_NODES_PER_PIXEL,
                "",
            );
            cx.slider(
                tr!("ui-options-oit-layers"),
                tr!("ui-options-oit-layers-help"),
                &field!(oit_max_layers),
                1..=aurora_render::oit::MAX_SORTED_LAYERS,
                "",
            );
        });
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
                let r = ui.add_enabled_ui(hdr, |ui| {
                    ui.add(
                        egui::Slider::new(&mut s.sharpen, 0.0..=1.0)
                            .custom_parser(aurora_i18n::parse_number)
                            .step_by(0.05)
                            .trailing_fill(true),
                    )
                    .changed()
                });
                r.response.on_disabled_hover_text(tr!("ui-options-needs-hdr-emissive"));
                r.inner
            },
        );
    });
    depth_of_field(cx);
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
        });
    });
}

/// Scope and rendering exceptions use the same saved identities and list controls.
fn parcel_list(
    cx: &mut Cx,
    label: &str,
    hint: &str,
    field: &super::super::cx::F<Vec<crate::world::parcel_visibility::Entry>>,
    selected: bool,
) {
    use crate::ui::widgets;
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

            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt(field.path)
                .max_height(180.0)
                // Cx measures controls from a short first-frame placeholder.
                // Let the list establish its height, then shrink to its content.
                .min_scrolled_height(180.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, entry) in entries.iter().enumerate() {
                        ui.push_id((entry.region, entry.local_id), |ui| {
                            ui.horizontal_top(|ui| {
                                let width = (ui.available_width() - 20.0 - ui.spacing().item_spacing.x).max(1.0);
                                ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_min_width(width);
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.add(egui::Label::new(RichText::new(&entry.name).strong()).wrap());
                                    ui.add(egui::Label::new(RichText::new(&entry.region_name).size(12.0).color(p.muted)).wrap());
                                });
                                if widgets::icon_button(ui, &p, "trash-filled", tr!("common-remove"), true) {
                                    remove = Some(i);
                                }
                            });
                        });
                    }
                });
            if let Some(i) = remove {
                entries.remove(i);
                changed = true;
            }
        });
        changed
    });
}
