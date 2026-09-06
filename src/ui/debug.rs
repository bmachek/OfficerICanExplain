//! Developer overlay.
//!
//! The tuning panel exists because feel-critical constants (camera, and from M3
//! the entire vehicle handling model) are impossible to get right by recompiling
//! between guesses. Anything in `GameConfig` should be editable here.

use bevy::dev_tools::fps_overlay::FpsOverlayPlugin;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use leafwing_input_manager::prelude::ActionState;

use crate::ai::pedestrian::Pedestrian;
use crate::core::config::GameConfig;
use crate::core::states::AppState;
use crate::mood::feeling::{CityMood, Tempers};
use crate::player::camera::CameraRig;
use crate::player::input::Action;
use crate::player::interact::Driving;
use crate::player::on_foot::Player;
use crate::render::quality::{AoQuality, Capabilities, QualityPreset};
use crate::vehicle::controller::VehicleState;
use crate::vehicle::spec::VehicleSpec;
use crate::world::weather::Weather;

pub struct DebugUiPlugin;

impl Plugin for DebugUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FpsOverlayPlugin::default())
            .add_systems(EguiPrimaryContextPass, (tuning_panel, vehicle_panel));
    }
}

fn tuning_panel(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut config: ResMut<GameConfig>,
    mut weather: ResMut<Weather>,
    mut tempers: ResMut<Tempers>,
    city: Res<CityMood>,
    caps: Res<Capabilities>,
    state: Res<State<AppState>>,
    cameras: Query<(&Transform, &CameraRig)>,
    crowd: Query<Entity, With<Pedestrian>>,
    actions: Query<&ActionState<Action>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    egui::Window::new("dev")
        .default_pos([12.0, 12.0])
        .default_width(280.0)
        .show(ctx, |ui| {
            ui.label(format!("state:  {:?}", state.get()));
            ui.label(format!("seed:   {:#x}", config.world_seed));
            if let Ok((transform, rig)) = cameras.single() {
                let p = transform.translation;
                ui.label(format!(
                    "camera: {:.0}, {:.0}, {:.0}  [{:?}]",
                    p.x, p.y, p.z, rig.mode
                ));
            }

            ui.separator();
            ui.label(egui::RichText::new("free camera (F1)").strong());
            ui.add(
                egui::Slider::new(&mut config.camera.speed, 1.0..=200.0)
                    .text("speed")
                    .logarithmic(true),
            );
            ui.add(
                egui::Slider::new(&mut config.camera.boost_multiplier, 1.0..=20.0).text("boost"),
            );
            ui.add(
                egui::Slider::new(&mut config.camera.mouse_sensitivity, 0.0005..=0.01)
                    .text("sensitivity"),
            );

            ui.separator();
            ui.label(egui::RichText::new("follow camera").strong());
            ui.add(
                egui::Slider::new(&mut config.camera.auto_follow, 0.0..=10.0).text("auto swing"),
            );
            ui.add(
                egui::Slider::new(&mut config.camera.auto_follow_delay, 0.0..=3.0)
                    .text("swing delay s"),
            );

            ui.separator();
            ui.label(egui::RichText::new("mixer").strong());
            ui.add(egui::Slider::new(&mut config.audio.master, 0.0..=1.0).text("master"));
            ui.add(egui::Slider::new(&mut config.audio.effects, 0.0..=1.5).text("effects"));
            ui.add(egui::Slider::new(&mut config.audio.ambience, 0.0..=1.5).text("ambience"));

            ui.separator();
            bounce_section(ui, &mut config);

            ui.separator();
            mood_section(ui, &mut commands, &mut config, &mut tempers, &city, &crowd);

            ui.separator();
            weather_section(ui, &mut weather, &mut config);

            ui.separator();
            graphics_section(ui, &mut config, &caps);

            ui.separator();
            ui.label(egui::RichText::new("input").strong());
            if let Ok(action_state) = actions.single() {
                let movement = action_state.clamped_axis_pair(&Action::Move);
                ui.label(format!("move:   {:+.2}, {:+.2}", movement.x, movement.y));
                ui.label(format!(
                    "jump {}   sprint {}   taunt {}   cheer {}",
                    action_state.pressed(&Action::Jump),
                    action_state.pressed(&Action::Sprint),
                    action_state.pressed(&Action::Taunt),
                    action_state.pressed(&Action::Cheer),
                ));
            } else {
                ui.label("no input carrier");
            }

            ui.separator();
            ui.label(
                egui::RichText::new(
                    "WASD move · Shift sprint · Space jump · LMB taunt · RMB cheer · MMB sorry · F1 free cam",
                )
                .small()
                .weak(),
            );
        });

    Ok(())
}

/// How the city feels, and the five dispositions it is made of.
///
/// The whole game is the interaction between these numbers, and no amount of
/// reasoning about them substitutes for pushing one and watching a street turn.
/// The temperaments are the awkward half: a citizen draws its disposition once,
/// at spawn, so editing the table changes nobody who is already walking about.
/// Rather than reach into every flummi and rewrite it — which would also have
/// to decide what to do about the one currently chasing you — the panel simply
/// offers to clear the crowd. `maintain_population` refills it from the new
/// table within a second or two, which is both simpler and easier to believe.
/// The hop dials. Deliberately not the whole of `BounceConfig`: `restitution`
/// and `threshold` are read once at plugin build into `DefaultRestitution`
/// and `SolverConfig` (`world::mod`), so a slider for them would move nothing
/// and teach the wrong lesson about what is live.
fn bounce_section(ui: &mut egui::Ui, config: &mut GameConfig) {
    ui.label(egui::RichText::new("bounce").strong());
    let b = &mut config.bounce;
    ui.add(egui::Slider::new(&mut b.hop_speed, 0.5..=6.0).text("hop m/s"));
    ui.add(egui::Slider::new(&mut b.player_hop_scale, 0.0..=1.5).text("player hop"));
    ui.add(egui::Slider::new(&mut b.npc_spring_max, 1.0..=2.5).text("npc spring max"));
    ui.add(egui::Slider::new(&mut b.squash, 0.0..=0.8).text("squash"));
}

fn mood_section(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    config: &mut GameConfig,
    tempers: &mut Tempers,
    city: &CityMood,
    crowd: &Query<Entity, With<Pedestrian>>,
) {
    ui.label(egui::RichText::new("mood").strong());
    ui.label(format!(
        "you {:+.2}   city {:+.2}   over {} flummis",
        city.player, city.average, city.crowd
    ));

    let m = &mut config.mood;
    ui.add(egui::Slider::new(&mut m.contagion_radius, 0.0..=30.0).text("contagion m"));
    ui.add(egui::Slider::new(&mut m.contagion_rate, 0.0..=4.0).text("contagion rate"));
    ui.add(egui::Slider::new(&mut m.bop_limit, 0.5..=20.0).text("bop limit m/s"));
    ui.add(egui::Slider::new(&mut m.outrage_limit, 2.0..=40.0).text("outrage m/s"));
    ui.add(egui::Slider::new(&mut m.taunt_radius, 1.0..=40.0).text("taunt m"));
    ui.add(egui::Slider::new(&mut m.cheer_radius, 1.0..=40.0).text("cheer m"));
    ui.add(egui::Slider::new(&mut m.taunt_bite, 0.0..=2.0).text("taunt bite"));
    ui.add(egui::Slider::new(&mut m.cheer_warmth, 0.0..=2.0).text("cheer warmth"));
    ui.add(egui::Slider::new(&mut m.provoke_rest, 0.05..=4.0).text("provoke rest s"));
    ui.add(egui::Slider::new(&mut m.apology_range, 2.0..=30.0).text("apology m"));
    ui.add(egui::Slider::new(&mut m.apology_balm, 0.0..=1.0).text("apology balm"));
    ui.add(egui::Slider::new(&mut m.grudge_seconds, 0.0..=30.0).text("grudge s"));
    ui.add(egui::Slider::new(&mut m.grudge_speed, 0.0..=12.0).text("grudge m/s"));

    ui.collapsing("tempers", |ui| {
        for kind in &mut tempers.0 {
            ui.collapsing(kind.name, |ui| {
                let t = &mut kind.temper;
                ui.add(egui::Slider::new(&mut kind.share, 0.0..=1.0).text("share"));
                ui.add(egui::Slider::new(&mut t.baseline, -1.0..=1.0).text("baseline"));
                ui.add(egui::Slider::new(&mut t.fuse, 0.0..=2.0).text("fuse"));
                ui.add(egui::Slider::new(&mut t.recovery, 0.0..=1.5).text("recovery"));
                ui.add(egui::Slider::new(&mut t.contagion, 0.0..=1.5).text("contagion"));
                ui.add(egui::Slider::new(&mut t.grudge, 0.0..=1.0).text("grudge"));
            });
        }
        if ui.button("re-roll the crowd").clicked() {
            for pedestrian in crowd {
                commands.entity(pedestrian).despawn();
            }
        }
        ui.label(
            egui::RichText::new("edits apply to flummis spawned from now on")
                .small()
                .weak(),
        );
    });
}

/// The live sky, and a way to take it over.
///
/// Weather runs itself now, which is exactly what makes a panel necessary: the
/// interesting states — a front rolling in, a road drying out — take game hours
/// to arrive on their own, and nobody tuning the look of rain is going to wait
/// for it. Dragging a slider is an override, so the sliders read back the
/// simulation until they are touched and the simulation carries on from
/// wherever they were left.
fn weather_section(ui: &mut egui::Ui, weather: &mut Weather, config: &mut GameConfig) {
    ui.label(egui::RichText::new("weather").strong());
    ui.add(egui::Slider::new(&mut weather.cover, 0.0..=1.0).text("cloud"));
    ui.add(egui::Slider::new(&mut weather.wetness, 0.0..=1.0).text("wetness"));
    ui.label(format!(
        "rain {:.2}   wind {:.1} m/s   hour {:+.1}",
        weather.rain,
        weather.wind_speed(),
        weather.elapsed,
    ));
    // The one control that is not an override: with the clock stopped, nothing
    // above moves on its own, and that is how a screenshot holds still.
    ui.add(
        egui::Slider::new(&mut config.world.day_length_seconds, 0.0..=1800.0)
            .text("day length (s)"),
    );
}

/// Renderer tier, and what it resolved to.
///
/// Picking a preset re-derives the whole block, which is the point: these
/// settings are meant to be compared as coherent tiers rather than mixed by
/// hand. The individual toggles below it are still editable, because judging
/// whether one effect is worth its cost means being able to turn exactly that
/// one off — but they are shown as what they are, a deviation from the preset.
fn graphics_section(ui: &mut egui::Ui, config: &mut GameConfig, caps: &Capabilities) {
    ui.label(egui::RichText::new("graphics").strong());

    let requested = config.graphics.requested;
    let mut chosen = requested;
    egui::ComboBox::from_label("preset")
        .selected_text(requested.name())
        .show_ui(ui, |ui| {
            for preset in QualityPreset::ALL {
                ui.selectable_value(&mut chosen, preset, preset.name());
            }
        });
    if chosen != requested {
        // Straight back through the same downgrade the startup probe ran, so a
        // preset picked here can never ask for more than the GPU has.
        config.graphics = chosen.settings().downgrade(*caps);
    }

    if !caps.raytracing {
        ui.label(
            egui::RichText::new("no ray query on this GPU — raytracing tiers fall back")
                .small()
                .weak(),
        );
    }

    let g = &mut config.graphics;
    ui.checkbox(&mut g.contact_shadows, "contact shadows");
    ui.checkbox(&mut g.soft_shadows, "soft shadows (PCSS)");
    ui.checkbox(&mut g.ssr, "screen-space reflections");
    ui.checkbox(&mut g.motion_blur, "motion blur");
    ui.checkbox(&mut g.depth_of_field, "depth of field");

    let mut ao_on = g.ssao.is_some();
    if ui.checkbox(&mut ao_on, "ambient occlusion").changed() {
        g.ssao = ao_on.then_some(AoQuality::High);
    }

    ui.add(
        egui::Slider::new(&mut g.shadow_distance, 100.0..=2000.0)
            .text("shadow distance")
            .suffix(" m"),
    );
    // Read once, when a chunk spawns, and baked into the visibility ranges of
    // everything in it. Moving this changes what the *next* chunk to stream in
    // does; walk out of a chunk and back into it to see it applied here.
    ui.add(egui::Slider::new(&mut g.lod_scale, 0.25..=3.0).text("lod scale"));

    ui.label(
        egui::RichText::new(format!(
            "volumetrics {:?} · upscaling {:?} · {}x shadow map",
            g.volumetrics, g.upscaling, g.shadow_map_size,
        ))
        .small()
        .weak(),
    );
}

/// Live handling tuning for whatever the player is driving.
///
/// This exists because vehicle feel cannot be reasoned into place — it has to
/// be driven, adjusted, and driven again. Putting these behind a recompile
/// would mean a thirty-second loop per guess, and the car would never get good.
fn vehicle_panel(
    mut contexts: EguiContexts,
    players: Query<&Driving, With<Player>>,
    mut vehicles: Query<(&mut VehicleSpec, &VehicleState)>,
) -> Result {
    let Ok(driving) = players.single() else {
        return Ok(());
    };
    let Ok((mut spec, state)) = vehicles.get_mut(driving.0) else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;

    egui::Window::new(format!("{} — handling", spec.display_name))
        .default_pos([12.0, 320.0])
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.label(format!("speed:  {:6.1} km/h", state.speed_kph()));
            ui.label(format!(
                "wheels: {}/4 down    steer {:+.2} rad",
                state.grounded_wheels(),
                state.steer_angle
            ));
            ui.separator();
            ui.label(egui::RichText::new("grip").strong());
            ui.add(egui::Slider::new(&mut spec.front_grip, 0.4..=3.0).text("front"));
            ui.add(egui::Slider::new(&mut spec.rear_grip, 0.4..=3.0).text("rear"));
            ui.add(egui::Slider::new(&mut spec.handbrake_grip, 0.02..=1.0).text("handbrake"));
            ui.add(egui::Slider::new(&mut spec.roll_couple, 0.0..=1.0).text("body roll"));

            ui.separator();
            ui.label(egui::RichText::new("drivetrain").strong());
            ui.add(
                egui::Slider::new(&mut spec.engine_force, 2_000.0..=45_000.0)
                    .text("engine")
                    .logarithmic(true),
            );
            ui.add(egui::Slider::new(&mut spec.brake_force, 4_000.0..=60_000.0).text("brakes"));
            ui.add(egui::Slider::new(&mut spec.max_speed, 10.0..=90.0).text("top speed m/s"));
            ui.add(egui::Slider::new(&mut spec.drag, 0.5..=12.0).text("drag"));
            ui.add(egui::Slider::new(&mut spec.downforce, 0.0..=40.0).text("downforce"));

            ui.separator();
            ui.label(egui::RichText::new("steering").strong());
            ui.add(egui::Slider::new(&mut spec.max_steer, 0.15..=1.0).text("lock"));
            ui.add(egui::Slider::new(&mut spec.steer_rate, 1.0..=20.0).text("rate"));
            ui.add(
                egui::Slider::new(&mut spec.high_speed_steer, 0.05..=1.0).text("lock at top speed"),
            );

            ui.separator();
            ui.label(egui::RichText::new("suspension").strong());
            ui.add(egui::Slider::new(&mut spec.spring_strength, 8_000.0..=90_000.0).text("spring"));
            ui.add(egui::Slider::new(&mut spec.damping, 500.0..=12_000.0).text("damping"));
            ui.add(egui::Slider::new(&mut spec.anti_roll, 0.0..=30_000.0).text("anti-roll"));
        });

    Ok(())
}
