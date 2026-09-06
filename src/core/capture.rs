//! Screenshot mode: render a few frames into an offscreen texture, save a PNG, exit.
//!
//! Motivation: "it compiled and didn't panic" is not evidence that the world
//! renders correctly. This gives every milestone a visual check that needs no
//! human at the keyboard, and doubles as a way to eyeball city generation from
//! a fixed vantage point when tuning it.
//!
//! It renders to an offscreen image rather than to the window on purpose:
//! capturing the window surface returns black whenever the OS has not actually
//! composited the window (backgrounded, occluded, or just never focused), which
//! makes window capture useless for unattended runs.
//!
//! Usage:
//!   cargo run -- --screenshot shots/city.png
//!   cargo run -- --screenshot shots/city.png --at 0,400,600 --look 0,0,0
//!   cargo run -- --screenshot shots/city.png --frames 120
//!   cargo run -- --screenshot shots/city.png --quality ultra --fps-log
//!   cargo run -- --screenshot shots/city.png --hour 21.5 --cover 1 --wet 0.9
//!   cargo run -- --screenshot shots/faces.png --at-node 300 --eye 1.4 --mood -1

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use avian3d::prelude::{ColliderDisabled, RigidBodyDisabled};
use bevy::asset::RenderAssetUsages;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::time::Real;

use crate::player::camera::{CameraMode, CameraRig};
use crate::player::interact::{DrivenBy, Driving};
use crate::player::on_foot::Player;
use crate::render::quality::QualityPreset;
use crate::vehicle::controller::VehicleInput;
use crate::vehicle::spawn::Vehicle;

const CAPTURE_WIDTH: u32 = 1600;
const CAPTURE_HEIGHT: u32 = 900;

#[derive(Resource, Debug, Clone)]
pub struct CaptureRequest {
    pub path: PathBuf,
    /// Frames to render before capturing. Shadow maps, GPU culling and the
    /// streaming systems all need a few frames to settle first.
    pub warmup_frames: u32,
    pub eye: Option<Vec3>,
    pub look_at: Option<Vec3>,
    /// Overrides `world.stream_radius`, so an aerial shot can load more of the
    /// city than a player would ever have resident at once.
    pub stream_radius: Option<f32>,
    /// Stand at this road-graph intersection, looking down a connected street.
    /// Far easier than guessing coordinates that are not inside a building.
    pub node: Option<usize>,
    pub eye_height: f32,
    /// Freezes the clock at this hour, for checking the lighting cycle.
    pub hour: Option<f32>,
    /// Leaves the rig in follow mode, so the shot exercises the real
    /// third-person camera instead of a posed free camera.
    pub follow: bool,
    /// Opens the full-screen map.
    pub map: bool,
    /// Poses the camera three-quarters on to the nearest parked car.
    ///
    /// Bodywork is the one thing a street-level shot never shows properly: from
    /// the pavement a car is a silhouette, and from behind the wheels are
    /// hidden by its own bumper. Judging a body change needs this view.
    pub at_car: bool,
    /// Beats the framed car up by this fraction before shooting it, so damage
    /// can be judged without driving into a wall at the right angle first.
    pub damage: f32,
    /// Lines one of every archetype up down the street and shoots the row.
    /// The only way to compare bodywork without hunting the city for a pickup.
    pub showroom: bool,
    /// Soaks the ground, 0 to 1.
    pub wetness: f32,
    /// Puts this much cloud over the city, 0 to 1. Above about seven tenths it
    /// also rains — which is the only way to shoot rain, now that rainfall comes
    /// out of the sky rather than out of the ground being wet.
    pub cover: f32,
    /// Which renderer tier to shoot at. The whole point of a preset ladder is
    /// being able to put two tiers side by side in the same framing, and that
    /// needs the choice on the command line rather than in a config file.
    pub quality: QualityPreset,
    /// Holds every face in the city at this mood, −1 to 1.
    ///
    /// The faces are the one thing in the game that cannot be shot by finding
    /// the right corner to stand on: a city in a mood is a city that has to be
    /// *put* in one first. This forces it, so the whole ladder of faces can be
    /// photographed from the same spot in three runs.
    pub mood: Option<f32>,
    /// Logs frame times over the warmup run alongside the capture.
    ///
    /// A screenshot proves a change looks right; it says nothing about whether
    /// it can be afforded. From the point geometry density starts moving, this
    /// is the other half of the evidence.
    pub fps_log: bool,
    /// Puts the player in the nearest car and holds the throttle down.
    /// An end-to-end smoke test of enter -> drive -> chase camera that needs
    /// nobody at the keyboard.
    pub drive: bool,
}

#[derive(Resource)]
struct CaptureTarget(Handle<Image>);

#[derive(Resource)]
struct CaptureProgress {
    frame: u32,
    triggered: bool,
    saved: Arc<AtomicBool>,
    /// Frame durations in milliseconds, oldest first.
    ///
    /// Kept whole rather than reduced to a running mean because the number that
    /// matters for a sixty-a-second budget is not the average, it is how bad
    /// the slow frames get — a mean hides exactly the stutter that is worth
    /// knowing about.
    frame_times: Vec<f32>,
}

/// True when the process was launched to take a screenshot. Used to strip the
/// dev UI so captures show the world and nothing else.
pub fn is_capture_mode() -> bool {
    std::env::args().any(|a| a == "--screenshot")
}

/// Parses capture arguments. Returns `None` for a normal interactive run.
pub fn parse_args() -> Option<CaptureRequest> {
    let args: Vec<String> = std::env::args().collect();
    let idx = args.iter().position(|a| a == "--screenshot")?;
    let path = PathBuf::from(args.get(idx + 1)?);

    let value_of = |flag: &str| -> Option<String> {
        let i = args.iter().position(|a| a == flag)?;
        args.get(i + 1).cloned()
    };
    let vec3_of = |flag: &str| -> Option<Vec3> {
        let raw = value_of(flag)?;
        let parts: Vec<f32> = raw
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        match parts[..] {
            [x, y, z] => Some(Vec3::new(x, y, z)),
            _ => None,
        }
    };

    Some(CaptureRequest {
        path,
        warmup_frames: value_of("--frames")
            .and_then(|f| f.parse().ok())
            .unwrap_or(60),
        eye: vec3_of("--at"),
        look_at: vec3_of("--look"),
        stream_radius: value_of("--stream-radius").and_then(|v| v.parse().ok()),
        node: value_of("--at-node").and_then(|v| v.parse().ok()),
        eye_height: value_of("--eye")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.7),
        hour: value_of("--hour").and_then(|v| v.parse().ok()),
        at_car: args.iter().any(|a| a == "--at-car"),
        damage: value_of("--damage")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        showroom: args.iter().any(|a| a == "--showroom"),
        wetness: value_of("--wet")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        // A fair day unless asked otherwise. Deliberately *not* the seed's own
        // weather: every framing in the battery has to mean the same thing from
        // one run to the next, and "whatever the sky happened to be doing"
        // would make every shot an argument about the weather.
        cover: value_of("--cover")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.18),
        mood: value_of("--mood").and_then(|v| v.parse().ok()),
        quality: crate::render::preset_from_arg(value_of("--quality").as_deref()),
        fps_log: args.iter().any(|a| a == "--fps-log"),
        follow: args.iter().any(|a| a == "--follow"),
        drive: args.iter().any(|a| a == "--drive"),
        map: args.iter().any(|a| a == "--map"),
    })
}

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        let Some(request) = parse_args() else {
            return;
        };

        if let Some(parent) = request.path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }

        app.insert_resource(request)
            .insert_resource(CaptureProgress {
                frame: 0,
                triggered: false,
                saved: Arc::new(AtomicBool::new(false)),
                frame_times: Vec::new(),
            })
            .add_systems(PreStartup, apply_capture_overrides)
            .add_systems(PostStartup, retarget_camera_offscreen)
            .add_systems(Update, (pose_at_car, line_up_showroom))
            .add_systems(
                FixedUpdate,
                autodrive.before(crate::vehicle::controller::drive_vehicles),
            )
            .add_systems(Last, drive_capture);
    }
}

fn apply_capture_overrides(
    request: Res<CaptureRequest>,
    mut config: ResMut<crate::core::config::GameConfig>,
    mut map_open: ResMut<crate::ui::minimap::MapOpen>,
) {
    config.graphics = request.quality.settings();

    if let Some(radius) = request.stream_radius {
        config.world.stream_radius = radius;
    }
    if request.map {
        map_open.0 = true;
    }
    config.world.start_wetness = request.wetness;
    config.world.start_cover = request.cover;
    if let Some(hour) = request.hour {
        config.world.start_hour = hour;
        // Freeze it, so the warmup frames do not drift the sky. Weather runs on
        // the same clock, so this holds the cloud and the wetness with it.
        config.world.day_length_seconds = 0.0;
    }
}

/// Points the debug camera at an offscreen texture and applies the requested pose.
fn retarget_camera_offscreen(
    mut commands: Commands,
    request: Res<CaptureRequest>,
    mut images: ResMut<Assets<Image>>,
    city: Option<Res<crate::world::City>>,
    mut cameras: Query<(&mut RenderTarget, &mut Transform, &mut CameraRig), With<Camera>>,
) {
    let size = Extent3d {
        width: CAPTURE_WIDTH,
        height: CAPTURE_HEIGHT,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
        | TextureUsages::COPY_DST
        | TextureUsages::COPY_SRC
        | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);

    for (mut render_target, mut transform, mut rig) in &mut cameras {
        *render_target = RenderTarget::Image(ImageRenderTarget {
            handle: handle.clone(),
            scale_factor: 1.0,
        });

        if request.follow {
            // Let the real camera do its job; only nudge the orbit angle.
            rig.mode = CameraMode::Follow;
            if let Some(eye) = request.eye {
                rig.yaw = eye.x;
                rig.pitch = eye.y;
            }
            continue;
        }
        // Detached: otherwise the follow system drags the camera back to the
        // player the frame after the requested pose is applied.
        rig.mode = CameraMode::Free;

        // --at-node wins over --at, since it is the more specific request.
        let posed = request.node.zip(city.as_ref()).and_then(|(index, city)| {
            let graph = &city.graph;
            let id = crate::world::roadgraph::NodeId(index as u32 % graph.node_count() as u32);
            let here = graph.node(id).pos;
            let (down_street, _) = graph.neighbors(id).next()?;
            let there = graph.node(down_street).pos;
            Some((
                Vec3::new(here.x, request.eye_height, here.y),
                Vec3::new(there.x, request.eye_height, there.y),
            ))
        });

        if let Some((eye, target)) = posed {
            *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
            let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
            rig.yaw = yaw;
            rig.pitch = pitch;
        } else if let Some(eye) = request.eye {
            let target = request.look_at.unwrap_or(Vec3::ZERO);
            *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
            // Keep the controller's own angles in sync so it does not snap back.
            let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
            rig.yaw = yaw;
            rig.pitch = pitch;
        }
    }

    commands.insert_resource(CaptureTarget(handle));
}

fn drive_capture(
    mut commands: Commands,
    request: Res<CaptureRequest>,
    target: Res<CaptureTarget>,
    mut progress: ResMut<CaptureProgress>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
    drawables: Query<(), With<Mesh3d>>,
    cameras: Query<&Transform, With<CameraRig>>,
    subjects: Query<&Transform, With<Player>>,
) {
    progress.frame += 1;
    if request.fps_log && !progress.triggered {
        // `Time<Real>`, not the default virtual clock: that one clamps its
        // delta at 250 ms so a long frame cannot make the simulation take a
        // huge step. Perfectly correct for gameplay, and exactly wrong here —
        // it saturates on precisely the slow frames a budget is decided by, and
        // reports them all as an identical 250.00 ms.
        progress.frame_times.push(time.delta_secs() * 1000.0);
    }

    if !progress.triggered && progress.frame >= request.warmup_frames {
        progress.triggered = true;

        if request.fps_log {
            info!("{}", frame_time_summary(&progress.frame_times));
        }

        // Logged so a blank image can be told apart from an empty scene.
        let camera = cameras
            .single()
            .map(|t| format!("{:?}", t.translation))
            .unwrap_or_else(|_| "<none>".into());
        let player = subjects
            .single()
            .map(|t| format!("{:?}", t.translation))
            .unwrap_or_else(|_| "<none>".into());
        info!(
            "capturing: {} meshes, camera at {}, player at {}",
            drawables.iter().count(),
            camera,
            player
        );

        let flag = progress.saved.clone();
        commands
            .spawn(Screenshot::image(target.0.clone()))
            .observe(save_to_disk(request.path.clone()))
            .observe(move |_: On<ScreenshotCaptured>| flag.store(true, Ordering::SeqCst));
        return;
    }

    // `save_to_disk` writes synchronously inside its observer, so once the flag
    // is set the file is already on disk and it is safe to quit.
    if progress.saved.load(Ordering::SeqCst) {
        info!("capture complete: {}", request.path.display());
        exit.write(AppExit::Success);
    }
}

/// Drives the player into the nearest car and floors it.
///
/// Deliberately bypasses the normal interact range check: this is a smoke test
/// that the enter -> drive -> chase-camera path works, not a test of how close
/// you have to stand to a door.
fn autodrive(
    mut commands: Commands,
    request: Res<CaptureRequest>,
    players: Query<(Entity, &Transform, Option<&Driving>), With<Player>>,
    parked: Query<(Entity, &Transform), (With<Vehicle>, Without<DrivenBy>)>,
    mut inputs: Query<&mut VehicleInput>,
    states: Query<(&Transform, &crate::vehicle::controller::VehicleState)>,
    mut ticks: Local<u32>,
) {
    if !request.drive {
        return;
    }
    *ticks += 1;
    let Ok((player, transform, driving)) = players.single() else {
        return;
    };

    let Some(Driving(vehicle)) = driving else {
        // Let the city populate first. Stealing a car in the opening frames is
        // genuinely unwitnessed, which is correct behaviour but tests nothing.
        if *ticks < 200 {
            return;
        }
        let nearest = parked
            .iter()
            .map(|(entity, other)| (entity, other.translation.distance(transform.translation)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((vehicle, distance)) = nearest {
            info!("autodrive: taking a car {distance:.1}m away");
            commands.entity(player).insert((
                Driving(vehicle),
                RigidBodyDisabled,
                ColliderDisabled,
                Visibility::Hidden,
            ));
            commands.entity(vehicle).insert(DrivenBy(player));
        }
        return;
    };

    if let Ok(mut input) = inputs.get_mut(*vehicle) {
        // Flee, then stop. Driving away forever only demonstrates the escape
        // half of the loop; pulling up lets the pursuit catch up and exercises
        // sighting, chasing and re-escalation too.
        //
        // Deliberately not routed to the mission marker: doing that well needs
        // a competent autonomous driver, and a half-competent one just wrecks
        // the car against a building. Mission completion is covered by tests in
        // `mission` instead.
        let fleeing = *ticks < 700;
        input.throttle = if fleeing { 1.0 } else { -1.0 };
        input.steer = 0.0;
        input.handbrake = !fleeing;
    }

    if (*ticks).is_multiple_of(48)
        && let Ok((_, state)) = states.get(*vehicle)
    {
        info!(
            "t={:>4} {:>5.1} km/h  wheels {}/4",
            *ticks,
            state.speed_kph(),
            state.grounded_wheels(),
        );
    }
}

/// Frames the nearest parked car, once the parked cars exist.
///
/// Deferred to `Update` rather than done with the rest of the pose because
/// `spawn_parked_vehicles` runs in `PostStartup` alongside it, and there is no
/// ordering between them worth asserting for a debug flag.
fn pose_at_car(
    request: Res<CaptureRequest>,
    mut done: Local<bool>,
    mut impacts: MessageWriter<crate::vehicle::damage::VehicleImpact>,
    mut vehicles: Query<
        (
            Entity,
            &Transform,
            &mut crate::vehicle::damage::VehicleHealth,
        ),
        (With<Vehicle>, Without<CameraRig>),
    >,
    mut cameras: Query<(&mut Transform, &mut CameraRig)>,
) {
    if *done || !request.at_car {
        return;
    }
    let nearest = vehicles
        .iter()
        .min_by(|a, b| {
            a.1.translation
                .length_squared()
                .total_cmp(&b.1.translation.length_squared())
        })
        .map(|(entity, transform, _)| (entity, *transform));
    let Some((entity, car)) = nearest else {
        return;
    };

    if request.damage > 0.0
        && let Ok((_, _, mut health)) = vehicles.get_mut(entity)
    {
        health.current = health.max * (1.0 - request.damage).max(0.01);
        // Three blows from three sides, so the shot shows a car that has been
        // in a fight rather than one pressed neatly on the nose.
        for from in [
            Vec3::new(-0.2, 0.1, -1.0).normalize(),
            Vec3::new(1.0, 0.15, 0.3).normalize(),
            Vec3::new(-0.7, 0.0, 0.7).normalize(),
        ] {
            impacts.write(crate::vehicle::damage::VehicleImpact {
                vehicle: entity,
                position: car.translation,
                from,
                severity: 18.0 * request.damage,
            });
        }
    }

    // Off the front three-quarter, at about eye height for someone standing
    // beside it: the angle every car photograph is taken from, because it shows
    // the nose, one flank and both wheels on that side at once.
    // Far enough out that the lens is not adding drama of its own: from three
    // metres a nose fills the frame and every proportion is a lie.
    let eye = car.transform_point(Vec3::new(5.2, 1.75, -6.8));
    let target = car.translation + Vec3::Y * 0.35;

    for (mut transform, mut rig) in &mut cameras {
        rig.mode = CameraMode::Free;
        *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        rig.yaw = yaw;
        rig.pitch = pitch;
    }
    *done = true;
}

/// Parks one of every archetype in a row and frames them.
///
/// Anchored to the car the world guarantees at the player's start, because that
/// is a spot the generator has already established is a street rather than the
/// inside of a building.
fn line_up_showroom(
    mut commands: Commands,
    request: Res<CaptureRequest>,
    mut done: Local<bool>,
    assets: Option<Res<crate::vehicle::spawn::VehicleAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    vehicles: Query<&Transform, (With<Vehicle>, Without<CameraRig>)>,
    mut cameras: Query<(&mut Transform, &mut CameraRig)>,
) {
    if *done || !request.showroom {
        return;
    }
    let (Some(assets), Some(anchor)) = (
        assets,
        vehicles
            .iter()
            .min_by(|a, b| {
                a.translation
                    .length_squared()
                    .total_cmp(&b.translation.length_squared())
            })
            .copied(),
    ) else {
        return;
    };

    let classes = crate::vehicle::spec::VehicleClass::ALL;
    let spacing = 6.2;
    for (i, class) in classes.iter().enumerate() {
        let spec = class.spec();
        let along = anchor.forward() * (i as f32 * spacing);
        let at = anchor.translation + along;
        let transform =
            Transform::from_xyz(at.x, crate::vehicle::spawn::resting_height(&spec), at.z)
                .with_rotation(anchor.rotation);
        crate::vehicle::spawn::spawn_vehicle(
            &mut commands,
            &assets,
            &mut materials,
            spec,
            transform,
        );
    }

    // Off to one side and slightly up, far enough back that the row is not all
    // perspective.
    let middle = anchor.translation + anchor.forward() * (classes.len() as f32 * spacing * 0.5);
    let eye = middle + *anchor.right() * 15.0 + Vec3::Y * 6.0 - anchor.forward() * 6.0;

    for (mut transform, mut rig) in &mut cameras {
        rig.mode = CameraMode::Free;
        *transform = Transform::from_translation(eye).looking_at(middle + Vec3::Y * 0.4, Vec3::Y);
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        rig.yaw = yaw;
        rig.pitch = pitch;
    }
    *done = true;
}

/// Reduces a warmup run's frame times to the three numbers worth reporting.
///
/// The first frames of any run are pipeline compilation and streaming, not
/// rendering, and including them would make every measurement look terrible
/// regardless of the change being judged — so the opening quarter is dropped.
/// What is left is reported as a median and a 95th percentile, because a budget
/// is kept or missed by the slow frames rather than by the typical one.
fn frame_time_summary(samples: &[f32]) -> String {
    // A handful of frames is not a measurement, and a percentile over three
    // samples is arithmetic rather than evidence. Say so instead.
    const MIN_SAMPLES: usize = 8;
    if samples.len() < MIN_SAMPLES {
        return "frame times: too few frames to report".into();
    }

    let mut sorted: Vec<f32> = samples[samples.len() / 4..].to_vec();
    sorted.sort_by(f32::total_cmp);
    let at = |fraction: f32| {
        let last = sorted.len() - 1;
        sorted[((last as f32) * fraction).round() as usize]
    };

    let median = at(0.5);
    let p95 = at(0.95);
    let worst = sorted[sorted.len() - 1];
    format!(
        "frame times over {} frames: median {median:.2} ms ({:.0} fps), p95 {p95:.2} ms, worst {worst:.2} ms",
        sorted.len(),
        1000.0 / median.max(f32::EPSILON),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opening_frames_are_left_out_of_the_summary() {
        // Four catastrophic startup frames, then twelve good ones. A mean over
        // the lot would read 30 ms; the number that matters reads 10.
        let mut samples = vec![200.0; 4];
        samples.extend(std::iter::repeat_n(10.0, 12));

        let summary = frame_time_summary(&samples);
        assert!(summary.contains("median 10.00 ms"), "{summary}");
        assert!(summary.contains("over 12 frames"), "{summary}");
    }

    #[test]
    fn the_slow_frames_are_reported_rather_than_averaged_away() {
        let mut samples = vec![8.0; 96];
        samples[90] = 42.0;

        let summary = frame_time_summary(&samples);
        assert!(summary.contains("worst 42.00 ms"), "{summary}");
        assert!(summary.contains("median 8.00 ms"), "{summary}");
    }

    #[test]
    fn a_run_too_short_to_measure_says_so_rather_than_dividing_by_zero() {
        assert!(frame_time_summary(&[]).contains("too few"));
        assert!(frame_time_summary(&[16.0]).contains("too few"));
    }

    /// The headline is frames per second, and getting the reciprocal backwards
    /// is the kind of thing that survives review because both numbers look
    /// plausible.
    #[test]
    fn the_frame_rate_is_the_reciprocal_of_the_median() {
        let summary = frame_time_summary(&[16.666_667; 40]);
        assert!(summary.contains("60 fps"), "{summary}");
    }
}
