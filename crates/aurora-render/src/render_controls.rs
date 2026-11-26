//! Rendering controls shared by the CPU and the generated WGSL block.
//!
//! This module describes control routing only. It does not change a shadow,
//! occlusion, reflection, or lighting algorithm. The frame's dedicated uniform
//! is `[display_mode, feature_flags, contact_backend, abi_version]`.
//!
//! Four existing persisted contact diagnostic booleans remain readable. Their
//! historical priority selects one requested identity first; an incompatible
//! identity is inactive, rather than falling through to another saved boolean.
//! The shadow selector writes either a new visibility mode or one old switch.
//! A zero visibility mode preserves old settings, including cascade coverage.

/// Version of the dedicated four-word CPU/WGSL contract.
pub const ABI_VERSION: u32 = 1;

/// Values in `Frame.render_controls.x`. Exactly one display mode is active.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    #[default]
    Normal = 0,
    Cascades = 1,
    AmbientOcclusion = 2,
    ContactMask = 3,
    BendCandidates = 4,
    BendFailures = 5,
    ViewRayOutcomes = 6,
    ShadowMapVisibility = 7,
    ContactVisibility = 8,
    CombinedVisibility = 9,
}

/// Values in `Frame.render_controls.z`. This is the backend actually dispatched.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContactBackend {
    #[default]
    None = 0,
    Bend = 1,
    ViewRay = 2,
}

impl ContactBackend {
    /// Resolve from this frame's availability, requested method, and GPU variant.
    ///
    /// `enabled` must be `Renderer::contact_now`, which includes shadow state,
    /// target availability, and the environment override. `skipped` means
    /// `ContactShadows::variant == Variant::Skip`. Every method except 1 uses
    /// Bend, matching `configure_view_ray` and its compute dispatch branch.
    pub const fn from_frame(enabled: bool, method: u8, skipped: bool) -> Self {
        if !enabled || skipped {
            Self::None
        } else if method == 1 {
            Self::ViewRay
        } else {
            Self::Bend
        }
    }
}

/// A requested shadow visualization, suitable as the native combo's value.
/// The type's existing name is retained for callers of the baseline controls.
/// These values are a CPU/UI identity, not the uniform display-mode encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContactDiagnostic {
    #[default]
    Off,
    Mask,
    BendCandidates,
    BendFailures,
    ViewRayOutcomes,
    ShadowMapVisibility,
    ContactVisibility,
    CombinedVisibility,
    Cascades,
}

impl ContactDiagnostic {
    /// Additive persisted selector. Zero reads the original diagnostic switches;
    /// recognized new modes take precedence without changing those old fields.
    /// Unknown values also fall back, for forward/backward settings compatibility.
    pub const fn from_saved(visibility: u8, switches: ContactDiagnosticSwitches, legacy_cascades: bool) -> Self {
        match visibility {
            1 => Self::ShadowMapVisibility,
            2 => Self::ContactVisibility,
            3 => Self::CombinedVisibility,
            4 => Self::Cascades,
            _ => {
                let old = switches.requested();
                if matches!(old, Self::Off) && legacy_cascades {
                    Self::Cascades
                } else {
                    old
                }
            }
        }
    }

    /// Value for `shadow_visibility_debug`. The four older modes keep their
    /// existing booleans so an older viewer can still read that selection.
    pub const fn visibility_setting(self) -> u8 {
        match self {
            Self::ShadowMapVisibility => 1,
            Self::ContactVisibility => 2,
            Self::CombinedVisibility => 3,
            Self::Cascades => 4,
            _ => 0,
        }
    }

    /// A selected combo value updates all four existing saved switches together.
    pub const fn switches(self) -> ContactDiagnosticSwitches {
        ContactDiagnosticSwitches {
            mask: matches!(self, Self::Mask),
            candidates: matches!(self, Self::BendCandidates),
            failures: matches!(self, Self::BendFailures),
            view_ray: matches!(self, Self::ViewRayOutcomes),
        }
    }

    /// Whether the chosen visualization can interpret this backend's metadata.
    /// Map, combined and cascade views also work without contact shadows.
    pub const fn supported_by(self, backend: ContactBackend) -> bool {
        match self {
            Self::Off | Self::ShadowMapVisibility | Self::CombinedVisibility | Self::Cascades => true,
            Self::Mask | Self::ContactVisibility => !matches!(backend, ContactBackend::None),
            Self::BendCandidates | Self::BendFailures => matches!(backend, ContactBackend::Bend),
            Self::ViewRayOutcomes => matches!(backend, ContactBackend::ViewRay),
        }
    }
}

/// Existing saved contact diagnostic settings, without a viewer dependency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContactDiagnosticSwitches {
    pub mask: bool,
    pub candidates: bool,
    pub failures: bool,
    pub view_ray: bool,
}

impl ContactDiagnosticSwitches {
    /// Preserve the previous display priority when loading a conflicting preset.
    /// A UI must show this identity even when it is inactive for the chosen
    /// backend; presenting it as Off would conceal a saved switch.
    pub const fn requested(self) -> ContactDiagnostic {
        if self.failures {
            ContactDiagnostic::BendFailures
        } else if self.candidates {
            ContactDiagnostic::BendCandidates
        } else if self.mask {
            ContactDiagnostic::Mask
        } else if self.view_ray {
            ContactDiagnostic::ViewRayOutcomes
        } else {
            ContactDiagnostic::Off
        }
    }
}

/// Resolve one display mode. A supported selector choice takes precedence;
/// otherwise the separate cascade/AO debug controls retain their old priority.
///
/// `cascades` combines `AURORA_DEBUG_SHADOWS`, `f.debug.shadows`, and the existing
/// `near_shadow_debug` setting. `ambient_occlusion` is `f.debug.ambient_occlusion`.
pub const fn resolve_display(
    requested: ContactDiagnostic,
    backend: ContactBackend,
    cascades: bool,
    ambient_occlusion: bool,
) -> DisplayMode {
    if requested.supported_by(backend) {
        match requested {
            ContactDiagnostic::Off => {}
            ContactDiagnostic::Mask => return DisplayMode::ContactMask,
            ContactDiagnostic::BendCandidates => return DisplayMode::BendCandidates,
            ContactDiagnostic::BendFailures => return DisplayMode::BendFailures,
            ContactDiagnostic::ViewRayOutcomes => return DisplayMode::ViewRayOutcomes,
            ContactDiagnostic::ShadowMapVisibility => return DisplayMode::ShadowMapVisibility,
            ContactDiagnostic::ContactVisibility => return DisplayMode::ContactVisibility,
            ContactDiagnostic::CombinedVisibility => return DisplayMode::CombinedVisibility,
            ContactDiagnostic::Cascades => return DisplayMode::Cascades,
        }
    }
    if cascades {
        DisplayMode::Cascades
    } else if ambient_occlusion {
        DisplayMode::AmbientOcclusion
    } else {
        DisplayMode::Normal
    }
}

// `Frame.render_controls.y` only. These are never OR-ed into light_mode.z.
// Bit 0 (mask value 1) remains reserved for the retired compositor; no setting emits it.
pub const FLAG_INTEGRITY_COMPOSITE: u32 = 1;
pub const FLAG_HARMONIZE: u32 = 2;
pub const FLAG_HARMONIZE_SUN: u32 = 4;
pub const FLAG_HARMONIZE_AMBIENT: u32 = 8;
pub const FLAG_HARMONIZE_LOCAL: u32 = 16;
pub const FLAG_NEAR_SHADOW_DETAIL: u32 = 32;

/// Independent features; disabled harmonization ignores only its sub-switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeatureSwitches {
    /// Readable for old callers/settings; deliberately ignored by `bits`.
    pub integrity_composite: bool,
    pub harmonize: bool,
    pub harmonize_sun: bool,
    pub harmonize_ambient: bool,
    pub harmonize_local: bool,
    pub near_shadow_detail: bool,
}

impl FeatureSwitches {
    pub const fn bits(self) -> u32 {
        let mut flags = 0;
        if self.near_shadow_detail {
            flags |= FLAG_NEAR_SHADOW_DETAIL;
        }
        if self.harmonize {
            flags |= FLAG_HARMONIZE;
            if self.harmonize_sun {
                flags |= FLAG_HARMONIZE_SUN;
            }
            if self.harmonize_ambient {
                flags |= FLAG_HARMONIZE_AMBIENT;
            }
            if self.harmonize_local {
                flags |= FLAG_HARMONIZE_LOCAL;
            }
        }
        flags
    }
}

/// Construct the main view's dedicated uniform without touching packed legacy
/// flags, contact parameters, or `Frame.features` (whose y component is in use).
/// Lit mirrors use `mirror_shadow_controls`; other passes keep zeroed controls.
pub const fn pack_frame(
    requested: ContactDiagnostic,
    backend: ContactBackend,
    cascades: bool,
    ambient_occlusion: bool,
    features: FeatureSwitches,
) -> [u32; 4] {
    [
        resolve_display(requested, backend, cascades, ambient_occlusion) as u32,
        features.bits(),
        backend as u32,
        ABI_VERSION,
    ]
}

/// Lit mirrors share the main shadow maps. Copy only the map-lookup feature;
/// contact diagnostics and lighting controls remain specific to the main view.
/// With the feature off, preserve the original all-zero auxiliary controls.
pub const fn mirror_shadow_controls(main: [u32; 4]) -> [u32; 4] {
    let flags = main[1] & FLAG_NEAR_SHADOW_DETAIL;
    if flags == 0 {
        [0; 4]
    } else {
        [DisplayMode::Normal as u32, flags, ContactBackend::None as u32, ABI_VERSION]
    }
}

/// Single source for the WGSL constants. The shader contains this generated
/// block, and an integration test compares it with this function's output.
pub fn wgsl_definitions() -> String {
    use std::fmt::Write;

    let values = [
        ("RC_ABI_VERSION", ABI_VERSION),
        ("RC_MODE_NORMAL", DisplayMode::Normal as u32),
        ("RC_MODE_CASCADES", DisplayMode::Cascades as u32),
        ("RC_MODE_AMBIENT_OCCLUSION", DisplayMode::AmbientOcclusion as u32),
        ("RC_MODE_CONTACT_MASK", DisplayMode::ContactMask as u32),
        ("RC_MODE_BEND_CANDIDATES", DisplayMode::BendCandidates as u32),
        ("RC_MODE_BEND_FAILURES", DisplayMode::BendFailures as u32),
        ("RC_MODE_VIEW_RAY_OUTCOMES", DisplayMode::ViewRayOutcomes as u32),
        ("RC_MODE_SHADOW_MAP_VISIBILITY", DisplayMode::ShadowMapVisibility as u32),
        ("RC_MODE_CONTACT_VISIBILITY", DisplayMode::ContactVisibility as u32),
        ("RC_MODE_COMBINED_VISIBILITY", DisplayMode::CombinedVisibility as u32),
        ("RC_FLAG_INTEGRITY_COMPOSITE", FLAG_INTEGRITY_COMPOSITE),
        ("RC_FLAG_HARMONIZE", FLAG_HARMONIZE),
        ("RC_FLAG_HARMONIZE_SUN", FLAG_HARMONIZE_SUN),
        ("RC_FLAG_HARMONIZE_AMBIENT", FLAG_HARMONIZE_AMBIENT),
        ("RC_FLAG_HARMONIZE_LOCAL", FLAG_HARMONIZE_LOCAL),
        ("RC_FLAG_NEAR_SHADOW_DETAIL", FLAG_NEAR_SHADOW_DETAIL),
        ("RC_BACKEND_NONE", ContactBackend::None as u32),
        ("RC_BACKEND_BEND", ContactBackend::Bend as u32),
        ("RC_BACKEND_VIEW_RAY", ContactBackend::ViewRay as u32),
    ];
    let mut block = String::from("// BEGIN AURORA_RENDER_CONTROLS_V2\n");
    for (name, value) in values {
        writeln!(&mut block, "const {name}: u32 = {value}u;").expect("writing WGSL constants to a String");
    }
    block.push_str("// END AURORA_RENDER_CONTROLS_V2\n");
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUESTS: [ContactDiagnostic; 9] = [
        ContactDiagnostic::Off,
        ContactDiagnostic::Mask,
        ContactDiagnostic::BendCandidates,
        ContactDiagnostic::BendFailures,
        ContactDiagnostic::ViewRayOutcomes,
        ContactDiagnostic::ShadowMapVisibility,
        ContactDiagnostic::ContactVisibility,
        ContactDiagnostic::CombinedVisibility,
        ContactDiagnostic::Cascades,
    ];
    const BACKENDS: [ContactBackend; 3] = [ContactBackend::None, ContactBackend::Bend, ContactBackend::ViewRay];

    #[test]
    fn baseline_control_abi_values_are_fixed() {
        assert_eq!(ABI_VERSION, 1);
        assert_eq!(
            [
                DisplayMode::Normal as u32,
                DisplayMode::Cascades as u32,
                DisplayMode::AmbientOcclusion as u32,
                DisplayMode::ContactMask as u32,
                DisplayMode::BendCandidates as u32,
                DisplayMode::BendFailures as u32,
                DisplayMode::ViewRayOutcomes as u32,
                DisplayMode::ShadowMapVisibility as u32,
                DisplayMode::ContactVisibility as u32,
                DisplayMode::CombinedVisibility as u32,
            ],
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        );
        assert_eq!(BACKENDS.map(|b| b as u32), [0, 1, 2]);
        let flags = [
            FLAG_INTEGRITY_COMPOSITE,
            FLAG_HARMONIZE,
            FLAG_HARMONIZE_SUN,
            FLAG_HARMONIZE_AMBIENT,
            FLAG_HARMONIZE_LOCAL,
            FLAG_NEAR_SHADOW_DETAIL,
        ];
        assert_eq!(flags, [1, 2, 4, 8, 16, 32]);
        for (i, flag) in flags.iter().enumerate() {
            assert_eq!(flag.count_ones(), 1);
            for other in &flags[i + 1..] {
                assert_eq!(flag & other, 0);
            }
        }
    }

    #[test]
    fn baseline_control_old_switches_have_one_stable_requested_identity() {
        use ContactDiagnostic::{BendCandidates as C, BendFailures as F, Mask as M, Off as O, ViewRayOutcomes as V};
        // Input bits: mask=1, candidates=2, failures=4, view_ray=8.
        let expected = [O, M, C, C, F, F, F, F, V, M, C, C, F, F, F, F];
        for (bits, expected) in expected.into_iter().enumerate() {
            let switches = ContactDiagnosticSwitches {
                mask: bits & 1 != 0,
                candidates: bits & 2 != 0,
                failures: bits & 4 != 0,
                view_ray: bits & 8 != 0,
            };
            assert_eq!(switches.requested(), expected, "saved switch combination {bits}");
        }
    }

    #[test]
    fn baseline_control_combo_selection_clears_all_other_saved_modes() {
        let expected = [
            [false, false, false, false],
            [true, false, false, false],
            [false, true, false, false],
            [false, false, true, false],
            [false, false, false, true],
            [false, false, false, false],
            [false, false, false, false],
            [false, false, false, false],
            [false, false, false, false],
        ];
        for (requested, expected) in REQUESTS.into_iter().zip(expected) {
            let s = requested.switches();
            assert_eq!([s.mask, s.candidates, s.failures, s.view_ray], expected);
            assert_eq!(ContactDiagnostic::from_saved(requested.visibility_setting(), s, false), requested,);
        }
    }

    #[test]
    fn shadow_visibility_saved_modes_override_old_switches_without_hiding_incompatible_modes() {
        use ContactDiagnostic::{Cascades, CombinedVisibility, ContactVisibility, ShadowMapVisibility};
        for bits in 0..16 {
            let switches = ContactDiagnosticSwitches {
                mask: bits & 1 != 0,
                candidates: bits & 2 != 0,
                failures: bits & 4 != 0,
                view_ray: bits & 8 != 0,
            };
            for cascades in [false, true] {
                let legacy = if bits == 0 && cascades { Cascades } else { switches.requested() };
                for saved in 0..=u8::MAX {
                    let expected = match saved {
                        1 => ShadowMapVisibility,
                        2 => ContactVisibility,
                        3 => CombinedVisibility,
                        4 => Cascades,
                        _ => legacy,
                    };
                    assert_eq!(ContactDiagnostic::from_saved(saved, switches, cascades), expected);
                }
            }
        }
        let requested = ContactDiagnostic::from_saved(
            2,
            ContactDiagnosticSwitches {
                failures: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(requested, ContactVisibility);
        assert_eq!(resolve_display(requested, ContactBackend::None, false, false), DisplayMode::Normal);
    }

    #[test]
    fn baseline_control_backend_tracks_availability_skip_and_all_method_values() {
        for method in 0..=u8::MAX {
            assert_eq!(ContactBackend::from_frame(false, method, false), ContactBackend::None);
            assert_eq!(ContactBackend::from_frame(false, method, true), ContactBackend::None);
            assert_eq!(ContactBackend::from_frame(true, method, true), ContactBackend::None);
            let expected = if method == 1 {
                ContactBackend::ViewRay
            } else {
                ContactBackend::Bend
            };
            assert_eq!(ContactBackend::from_frame(true, method, false), expected);
        }
    }

    #[test]
    fn baseline_control_every_mode_backend_and_fallback_combination_is_explicit() {
        // Rows are requested modes in REQUESTS order; columns are BACKENDS.
        // Zero means no supported contact view and thus the explicit fallback.
        let contact_modes = [
            [0, 0, 0],
            [0, 3, 3],
            [0, 4, 0],
            [0, 5, 0],
            [0, 0, 6],
            [7, 7, 7],
            [0, 8, 8],
            [9, 9, 9],
            [1, 1, 1],
        ];
        // Input bits: AO=1, cascades=2. Cascades take precedence over AO.
        let fallback_modes = [0, 2, 1, 1];
        for (request_index, request) in REQUESTS.into_iter().enumerate() {
            for (backend_index, backend) in BACKENDS.into_iter().enumerate() {
                for (fallback_bits, fallback) in fallback_modes.into_iter().enumerate() {
                    let expected_contact = contact_modes[request_index][backend_index];
                    let expected = if expected_contact != 0 { expected_contact } else { fallback };
                    assert_eq!(
                        resolve_display(request, backend, fallback_bits & 2 != 0, fallback_bits & 1 != 0) as u32,
                        expected,
                        "request {request:?}, backend {backend:?}, fallback {fallback_bits}",
                    );
                }
            }
        }
    }

    #[test]
    fn baseline_control_incompatible_saved_priority_does_not_expose_a_hidden_mode() {
        let requested = ContactDiagnosticSwitches {
            mask: true,
            candidates: true,
            failures: true,
            view_ray: true,
        }
        .requested();
        assert_eq!(requested, ContactDiagnostic::BendFailures);
        assert_eq!(
            resolve_display(requested, ContactBackend::ViewRay, false, false),
            DisplayMode::Normal
        );
        assert_eq!(
            resolve_display(requested, ContactBackend::ViewRay, true, true),
            DisplayMode::Cascades
        );
        assert_eq!(
            resolve_display(requested, ContactBackend::ViewRay, false, true),
            DisplayMode::AmbientOcclusion
        );
    }

    #[test]
    fn shadow_visibility_all_feature_masks_ignore_retired_compositor_and_obey_master() {
        // Input bits use the same documented order, but these expected masks
        // are a literal truth table; disabling the master drops its sub-switches.
        let expected = [
            0, 0, 2, 2, 0, 0, 6, 6, 0, 0, 10, 10, 0, 0, 14, 14, 0, 0, 18, 18, 0, 0, 22, 22, 0, 0, 26, 26, 0, 0, 30, 30,
        ];
        for (bits, expected) in expected.into_iter().enumerate() {
            let features = FeatureSwitches {
                integrity_composite: bits & 1 != 0,
                harmonize: bits & 2 != 0,
                harmonize_sun: bits & 4 != 0,
                harmonize_ambient: bits & 8 != 0,
                harmonize_local: bits & 16 != 0,
                near_shadow_detail: false,
            };
            assert_eq!(features.bits(), expected, "feature switch combination {bits}");
        }
    }

    #[test]
    fn shadow_visibility_retired_compositor_never_sets_any_uniform_flag() {
        for saved in 0..8 {
            let off = FeatureSwitches {
                integrity_composite: false,
                harmonize: false,
                harmonize_sun: saved & 1 != 0,
                harmonize_ambient: saved & 2 != 0,
                harmonize_local: saved & 4 != 0,
                near_shadow_detail: false,
            };
            let on = FeatureSwitches {
                integrity_composite: true,
                ..off
            };
            assert_eq!(off.bits(), 0);
            assert_eq!(on.bits(), 0);
        }
    }

    #[test]
    fn baseline_control_sun_normalization_and_view_ray_diagnostics_are_independent() {
        for requested in [ContactDiagnostic::Off, ContactDiagnostic::ViewRayOutcomes] {
            for sun in [false, true] {
                let features = FeatureSwitches {
                    harmonize: true,
                    harmonize_sun: sun,
                    ..FeatureSwitches::default()
                };
                let packed = pack_frame(requested, ContactBackend::ViewRay, false, false, features);
                let mode = if requested == ContactDiagnostic::Off { 0 } else { 6 };
                let flags = if sun { 6 } else { 2 };
                assert_eq!(packed, [mode, flags, 2, 1]);
            }
        }
    }

    #[test]
    fn baseline_control_frame_words_keep_diagnostics_and_lighting_separate() {
        for request in REQUESTS {
            for backend in BACKENDS {
                let dark = pack_frame(request, backend, false, false, FeatureSwitches::default());
                let lit = pack_frame(
                    request,
                    backend,
                    false,
                    false,
                    FeatureSwitches {
                        integrity_composite: true,
                        harmonize: true,
                        harmonize_sun: true,
                        harmonize_ambient: true,
                        harmonize_local: true,
                        near_shadow_detail: false,
                    },
                );
                assert_eq!(lit[0], dark[0]);
                assert_eq!(lit[1], 30);
                assert_eq!(dark[1], 0);
                assert_eq!([lit[2], lit[3]], [backend as u32, 1]);
                assert_eq!([dark[2], dark[3]], [backend as u32, 1]);
            }
        }
    }

    #[test]
    fn near_shadow_detail_is_independent_of_lighting_and_every_diagnostic() {
        for saved in 0..32 {
            let off = FeatureSwitches {
                integrity_composite: saved & 1 != 0,
                harmonize: saved & 2 != 0,
                harmonize_sun: saved & 4 != 0,
                harmonize_ambient: saved & 8 != 0,
                harmonize_local: saved & 16 != 0,
                near_shadow_detail: false,
            };
            let on = FeatureSwitches {
                near_shadow_detail: true,
                ..off
            };
            for request in REQUESTS {
                for backend in BACKENDS {
                    let old = pack_frame(request, backend, false, false, off);
                    let new = pack_frame(request, backend, false, false, on);
                    assert_eq!(new, [old[0], old[1] | 32, old[2], old[3]]);
                    assert_eq!(new[1] & FLAG_INTEGRITY_COMPOSITE, 0);
                }
            }
        }
    }

    #[test]
    fn near_shadow_detail_mirror_copies_only_the_shared_map_flag() {
        for flags in 0..128 {
            for mode in 0..=9 {
                for backend in 0..=2 {
                    let expected = if flags & 32 == 0 { [0; 4] } else { [0, 32, 0, 1] };
                    assert_eq!(mirror_shadow_controls([mode, flags, backend, 1]), expected);
                }
            }
        }
    }
}
