//! What a frame is allowed to cost.
//!
//! Every renderer feature in this game is a trade, and the trades are not the
//! same on every machine: ray queries exist on some GPUs and not others, a
//! shadow map that is free at 1080p is not free at 4K, and the geometry budget
//! that holds sixty frames a second on a desktop will not hold on a laptop.
//!
//! So nothing here is switched on directly. A [`QualityPreset`] is a single
//! choice a player makes, and it resolves to a [`GraphicsSettings`] — a flat
//! block of concrete numbers that the rest of `render` reads. Two consequences
//! fall out of that shape and both are deliberate:
//!
//! * **The preset is a request, not a promise.** [`GraphicsSettings::downgrade`]
//!   takes what the GPU actually reports and walks the settings back to
//!   something it can run. Asking for raytracing on hardware without ray
//!   queries gets you screen-space reflections, not a crash and not a black
//!   screen.
//! * **It is testable without a GPU.** The preset table and the downgrade rules
//!   are pure functions over plain data, so the interesting failure — a preset
//!   that quietly asks for something the tier below it does not — is a unit
//!   test rather than something you find in a screenshot three weeks later.
//!
//! The types are our own rather than Bevy's on purpose: `GraphicsSettings`
//! lives in `GameConfig`, which is serialised into save files, and a renderer
//! enum from a dependency is not something to write into a file on disk.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// The one knob a player turns.
///
/// Ordered, and the ordering is load-bearing: every rule in this module that
/// says "at least High" is a comparison, and the unit tests walk the tiers in
/// order to check that nothing gets cheaper as you go up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum QualityPreset {
    /// Integrated graphics. No screen-space work beyond ambient occlusion.
    Low,
    /// The floor for the game looking like itself: contact shadows and a real
    /// shadow distance.
    Medium,
    /// Where the city is meant to be seen. Deferred, reflections, volumetrics.
    #[default]
    High,
    /// Raytraced lighting where the hardware has it, and upscaling to pay for it.
    Ultra,
    /// Not a playable tier. Everything on, frame rate irrelevant — this is what
    /// `--screenshot` uses when the point is the picture rather than the game.
    Photo,
}

impl QualityPreset {
    pub const ALL: [Self; 5] = [
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Ultra,
        Self::Photo,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Ultra => "ultra",
            Self::Photo => "photo",
        }
    }

    /// Case-insensitive, for `--quality` and for the dev panel.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| p.name() == raw)
    }
}

/// How much of the horizon-based ambient occlusion to buy.
///
/// Mirrors Bevy's own quality levels rather than wrapping them, so this can be
/// serialised and so `render` owns the mapping in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AoQuality {
    Low,
    Medium,
    High,
    Ultra,
}

/// How far the volumetric pass goes.
///
/// Fog alone is the cheap half — it hazes the air and costs one full-screen
/// march. Lights is what actually sells a night street, because it is what puts
/// a visible cone under a lamp and a shaft between two buildings, and it costs
/// per light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Volumetrics {
    Off,
    /// A fog volume over the city, lit by the sun only.
    Fog,
    /// Street lamps and headlights cast visible shafts too.
    FogAndLights,
}

/// How the final image is resolved.
///
/// Not a quality dial so much as a choice of which temporal accumulator runs:
/// exactly one of these owns the jitter and the history buffer, so they are an
/// enum rather than a set of flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Upscaling {
    /// No temporal pass at all. Aliases, but has no history to smear.
    Off,
    /// Temporal anti-aliasing at native resolution.
    Taa,
    /// DLSS super resolution, rendering below native and reconstructing up.
    Dlss,
}

/// The resolved settings. Everything in `render` reads this and nothing else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphicsSettings {
    /// What was asked for. Kept after downgrading so the UI can say "Ultra
    /// (raytracing unavailable)" rather than silently claiming to be High.
    pub requested: QualityPreset,
    /// Side length of the directional light's shadow map, in texels.
    pub shadow_map_size: usize,
    /// How far from the camera the sun still casts. Clamped against the
    /// streaming radius when it is applied — there is no point shadowing
    /// geometry that has not been spawned.
    pub shadow_distance: f32,
    pub cascades: usize,
    /// Percentage-closer soft shadows: the penumbra widens with distance from
    /// whatever is casting, instead of every edge being equally hard.
    pub soft_shadows: bool,
    /// Short screen-space rays that put a shadow back where the shadow map's
    /// texel is too coarse to have one — under a bollard, under a wheel.
    pub contact_shadows: bool,
    pub ssao: Option<AoQuality>,
    /// Screen-space reflections. Requires the deferred path.
    pub ssr: bool,
    pub volumetrics: Volumetrics,
    pub motion_blur: bool,
    pub depth_of_field: bool,
    /// The artefacts of a real lens: a vignette, and a trace of chromatic
    /// aberration at the edge of the frame. Cheap, and pure luxury — the two
    /// lowest tiers spend the same milliseconds on something load-bearing.
    pub lens: bool,
    /// Multiplies every LOD switching distance. Below one, detail is dropped
    /// closer to the camera; above one it is held further out.
    pub lod_scale: f32,
    /// Raytraced direct and indirect lighting, replacing SSAO and SSR.
    pub raytracing: bool,
    pub upscaling: Upscaling,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        QualityPreset::default().settings()
    }
}

impl QualityPreset {
    /// The table. One row per tier, and the only place these numbers live.
    pub fn settings(self) -> GraphicsSettings {
        match self {
            Self::Low => GraphicsSettings {
                requested: self,
                shadow_map_size: 2048,
                shadow_distance: 300.0,
                cascades: 2,
                soft_shadows: false,
                contact_shadows: false,
                ssao: Some(AoQuality::Low),
                ssr: false,
                volumetrics: Volumetrics::Off,
                motion_blur: false,
                depth_of_field: false,
                lens: false,
                lod_scale: 0.6,
                raytracing: false,
                // No history buffer at all: on the hardware this tier is for,
                // TAA's resolve costs more than the aliasing it removes.
                upscaling: Upscaling::Off,
            },
            Self::Medium => GraphicsSettings {
                requested: self,
                shadow_map_size: 2048,
                shadow_distance: 500.0,
                cascades: 3,
                soft_shadows: false,
                contact_shadows: true,
                ssao: Some(AoQuality::Medium),
                ssr: false,
                volumetrics: Volumetrics::Fog,
                motion_blur: false,
                depth_of_field: false,
                lens: false,
                lod_scale: 0.85,
                raytracing: false,
                upscaling: Upscaling::Taa,
            },
            Self::High => GraphicsSettings {
                requested: self,
                shadow_map_size: 4096,
                shadow_distance: 900.0,
                cascades: 4,
                soft_shadows: true,
                contact_shadows: true,
                ssao: Some(AoQuality::High),
                ssr: true,
                volumetrics: Volumetrics::FogAndLights,
                motion_blur: true,
                depth_of_field: false,
                lens: true,
                lod_scale: 1.0,
                raytracing: false,
                upscaling: Upscaling::Taa,
            },
            Self::Ultra => GraphicsSettings {
                requested: self,
                shadow_map_size: 8192,
                shadow_distance: 1200.0,
                cascades: 4,
                soft_shadows: true,
                contact_shadows: true,
                // Raytraced lighting computes its own occlusion; a second
                // screen-space estimate on top of it double-darkens corners.
                ssao: None,
                ssr: false,
                volumetrics: Volumetrics::FogAndLights,
                motion_blur: true,
                depth_of_field: true,
                lens: true,
                lod_scale: 1.3,
                raytracing: true,
                upscaling: Upscaling::Dlss,
            },
            Self::Photo => GraphicsSettings {
                requested: self,
                shadow_map_size: 8192,
                shadow_distance: 2000.0,
                cascades: 4,
                soft_shadows: true,
                contact_shadows: true,
                ssao: None,
                ssr: false,
                volumetrics: Volumetrics::FogAndLights,
                // Both are shutter effects, and a still has no shutter. They
                // would only smear the thing the shot exists to show.
                motion_blur: false,
                depth_of_field: true,
                lens: true,
                // Nothing is ever allowed to drop detail: a still is judged at
                // full size, and a switched LOD is the one artefact that cannot
                // be argued away afterwards.
                lod_scale: f32::INFINITY,
                raytracing: true,
                // A still can afford to accumulate honestly rather than
                // reconstruct from a lower resolution.
                upscaling: Upscaling::Taa,
            },
        }
    }
}

/// What the GPU turned out to support.
///
/// Filled in once, after the render device exists, by querying `wgpu` features
/// — see `render::quality::probe`. Kept as plain bools so the downgrade rules
/// stay unit-testable with no device in the room.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// `EXPERIMENTAL_RAY_QUERY` and the binding-array features Solari needs.
    pub raytracing: bool,
    /// NVIDIA DLSS, which also requires the `dlss` cargo feature to be built in.
    pub dlss: bool,
}

impl Capabilities {
    /// Everything available. The baseline the preset table is written against.
    pub fn all() -> Self {
        Self {
            raytracing: true,
            dlss: true,
        }
    }
}

impl GraphicsSettings {
    /// Walks the settings back to what this machine can actually run.
    ///
    /// The rule is that a missing capability falls back to the nearest thing
    /// that produces a comparable picture, not to nothing: without ray queries
    /// the lighting returns to screen-space reflections *and* the ambient
    /// occlusion that Ultra had switched off, because otherwise dropping
    /// raytracing would leave corners with no occlusion term at all and the
    /// scene would come out flatter than High.
    pub fn downgrade(mut self, caps: Capabilities) -> Self {
        if self.raytracing && !caps.raytracing {
            self.raytracing = false;
            self.ssr = true;
            self.ssao = self.ssao.or(Some(AoQuality::High));
        }
        if self.upscaling == Upscaling::Dlss && !caps.dlss {
            self.upscaling = Upscaling::Taa;
        }
        self
    }

    /// True when something on the camera consumes motion vectors.
    ///
    /// TAA, DLSS and motion blur all need the same prepass, and asking for it
    /// three times independently is how it ends up requested in one code path
    /// and forgotten in another.
    pub fn needs_motion_vectors(&self) -> bool {
        self.motion_blur || matches!(self.upscaling, Upscaling::Taa | Upscaling::Dlss)
    }

    /// The distance at which a mesh should drop to its next level of detail,
    /// given the distance the art was authored for.
    pub fn lod_distance(&self, base: f32) -> f32 {
        base * self.lod_scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_survives_a_round_trip_through_its_name() {
        for preset in QualityPreset::ALL {
            assert_eq!(QualityPreset::parse(preset.name()), Some(preset));
        }
        assert_eq!(QualityPreset::parse("  ULTRA "), Some(QualityPreset::Ultra));
        assert_eq!(QualityPreset::parse("cinematic"), None);
    }

    /// The point of an ordered tier list is that going up never takes something
    /// away. Anything that genuinely gets *smaller* higher up is a considered
    /// exception and is listed here by name rather than left to be rediscovered.
    #[test]
    fn nothing_gets_cheaper_as_the_tier_goes_up() {
        for pair in QualityPreset::ALL.windows(2) {
            let (lower, upper) = (pair[0].settings(), pair[1].settings());
            let tier = pair[1].name();

            assert!(
                upper.shadow_map_size >= lower.shadow_map_size,
                "{tier} shrank the shadow map"
            );
            assert!(
                upper.shadow_distance >= lower.shadow_distance,
                "{tier} pulled the shadow distance in"
            );
            assert!(upper.cascades >= lower.cascades, "{tier} dropped a cascade");
            assert!(
                upper.volumetrics >= lower.volumetrics,
                "{tier} dropped volumetrics"
            );
            assert!(
                upper.soft_shadows || !lower.soft_shadows,
                "{tier} dropped soft shadows"
            );
            assert!(
                upper.contact_shadows || !lower.contact_shadows,
                "{tier} dropped contact shadows"
            );
            assert!(upper.lens || !lower.lens, "{tier} dropped the lens stack");
        }
    }

    /// LOD scale is the one number Photo is allowed to break the ladder with,
    /// and only because it is not a playable tier.
    #[test]
    fn detail_is_held_further_out_at_every_playable_tier() {
        let playable = [
            QualityPreset::Low,
            QualityPreset::Medium,
            QualityPreset::High,
            QualityPreset::Ultra,
        ];
        for pair in playable.windows(2) {
            let (lower, upper) = (pair[0].settings(), pair[1].settings());
            assert!(
                upper.lod_scale > lower.lod_scale,
                "{} did not hold detail further out",
                pair[1].name()
            );
            assert!(upper.lod_scale.is_finite());
        }
        assert!(QualityPreset::Photo.settings().lod_scale.is_infinite());
    }

    #[test]
    fn a_scene_always_has_an_occlusion_term_of_some_kind() {
        for preset in QualityPreset::ALL {
            let settings = preset.settings();
            assert!(
                settings.ssao.is_some() || settings.raytracing,
                "{} has neither ambient occlusion nor raytracing",
                preset.name()
            );
        }
    }

    #[test]
    fn losing_raytracing_falls_back_to_screen_space_rather_than_to_nothing() {
        let bare = Capabilities::default();
        for preset in [QualityPreset::Ultra, QualityPreset::Photo] {
            let settings = preset.settings().downgrade(bare);
            assert!(!settings.raytracing);
            assert!(settings.ssr, "{} lost reflections entirely", preset.name());
            assert!(
                settings.ssao.is_some(),
                "{} lost its occlusion term entirely",
                preset.name()
            );
        }
    }

    #[test]
    fn losing_dlss_leaves_a_temporal_pass_behind() {
        let settings = QualityPreset::Ultra.settings().downgrade(Capabilities {
            raytracing: true,
            dlss: false,
        });
        assert_eq!(settings.upscaling, Upscaling::Taa);
        assert!(settings.raytracing, "dlss and raytracing are independent");
    }

    #[test]
    fn downgrading_on_capable_hardware_changes_nothing() {
        for preset in QualityPreset::ALL {
            let settings = preset.settings();
            assert_eq!(settings.clone().downgrade(Capabilities::all()), settings);
        }
    }

    #[test]
    fn downgrading_twice_is_a_no_op() {
        let bare = Capabilities::default();
        for preset in QualityPreset::ALL {
            let once = preset.settings().downgrade(bare);
            assert_eq!(once.clone().downgrade(bare), once);
        }
    }

    #[test]
    fn motion_vectors_are_requested_by_every_pass_that_reads_them() {
        // Low has no temporal pass and no shutter effect, so it is the one tier
        // that can skip the prepass.
        assert!(!QualityPreset::Low.settings().needs_motion_vectors());
        for preset in [
            QualityPreset::Medium,
            QualityPreset::High,
            QualityPreset::Ultra,
            QualityPreset::Photo,
        ] {
            assert!(
                preset.settings().needs_motion_vectors(),
                "{} runs a temporal pass with no motion vectors",
                preset.name()
            );
        }
    }

    #[test]
    fn a_still_is_never_smeared_by_a_shutter() {
        let photo = QualityPreset::Photo.settings();
        assert!(!photo.motion_blur);
    }

    #[test]
    fn lod_distances_scale_with_the_tier() {
        assert_eq!(QualityPreset::High.settings().lod_distance(80.0), 80.0);
        assert!(QualityPreset::Low.settings().lod_distance(80.0) < 80.0);
        assert!(QualityPreset::Ultra.settings().lod_distance(80.0) > 80.0);
        assert!(
            QualityPreset::Photo
                .settings()
                .lod_distance(80.0)
                .is_infinite()
        );
    }
}
