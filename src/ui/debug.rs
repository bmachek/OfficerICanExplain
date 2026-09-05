//! Developer overlay.
//!
//! The tuning panel exists because feel-critical constants (camera, and from M3
//! the entire vehicle handling model) are impossible to get right by recompiling
//! between guesses. Anything in `GameConfig` should be editable here.

use bevy::dev_tools::fps_overlay::FpsOverlayPlugin;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use leafwing_input_manager::prelude::ActionState;

use crate::core::config::GameConfig;
use crate::core::states::AppState;
use crate::player::camera::CameraRig;
use crate::player::input::Action;
use crate::player::interact::Driving;
use crate::player::on_foot::Player;
use crate::vehicle::controller::VehicleState;
use crate::vehicle::damage::VehicleHealth;
use crate::vehicle::spec::VehicleSpec;

pub struct DebugUiPlugin;

impl Plugin for DebugUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((EguiPlugin::default(), FpsOverlayPlugin::default()))
            .add_systems(EguiPrimaryContextPass, (tuning_panel, vehicle_panel));
    }
}

fn tuning_panel(
    mut contexts: EguiContexts,
    mut config: ResMut<GameConfig>,
    state: Res<State<AppState>>,
    cameras: Query<(&Transform, &CameraRig)>,
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
            ui.label(egui::RichText::new("weather").strong());
            ui.add(egui::Slider::new(&mut config.world.wetness, 0.0..=1.0).text("wetness"));

            ui.separator();
            ui.label(egui::RichText::new("input").strong());
            if let Ok(action_state) = actions.single() {
                let movement = action_state.clamped_axis_pair(&Action::Move);
                ui.label(format!("move:   {:+.2}, {:+.2}", movement.x, movement.y));
                ui.label(format!(
                    "jump {}   sprint {}   fire {}",
                    action_state.pressed(&Action::Jump),
                    action_state.pressed(&Action::Sprint),
                    action_state.pressed(&Action::Fire),
                ));
            } else {
                ui.label("no input carrier");
            }

            ui.separator();
            ui.label(
                egui::RichText::new(
                    "WASD move · Shift sprint · Space jump · F1 free cam (RMB look)",
                )
                .small()
                .weak(),
            );
        });

    Ok(())
}

/// Live handling tuning for whatever the player is driving.
///
/// This exists because vehicle feel cannot be reasoned into place — it has to
/// be driven, adjusted, and driven again. Putting these behind a recompile
/// would mean a thirty-second loop per guess, and the car would never get good.
fn vehicle_panel(
    mut contexts: EguiContexts,
    players: Query<&Driving, With<Player>>,
    mut vehicles: Query<(&mut VehicleSpec, &VehicleState, &VehicleHealth)>,
) -> Result {
    let Ok(driving) = players.single() else {
        return Ok(());
    };
    let Ok((mut spec, state, health)) = vehicles.get_mut(driving.0) else {
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
            ui.add(
                egui::ProgressBar::new(health.fraction()).text(format!("{:.0} hp", health.current)),
            );

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
