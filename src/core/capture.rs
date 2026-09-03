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

use crate::player::camera::{CameraMode, CameraRig};
use crate::player::interact::{DrivenBy, Driving};
use crate::player::on_foot::Player;
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
            })
            .add_systems(PreStartup, apply_capture_overrides)
            .add_systems(PostStartup, retarget_camera_offscreen)
            .add_systems(Update, pose_at_car)
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
    if let Some(radius) = request.stream_radius {
        config.world.stream_radius = radius;
    }
    if request.map {
        map_open.0 = true;
    }
    if let Some(hour) = request.hour {
        config.world.start_hour = hour;
        // Freeze it, so the warmup frames do not drift the sky.
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
    mut exit: MessageWriter<AppExit>,
    drawables: Query<(), With<Mesh3d>>,
    cameras: Query<&Transform, With<CameraRig>>,
    subjects: Query<(&Transform, &crate::combat::health::Health), With<Player>>,
) {
    progress.frame += 1;

    if !progress.triggered && progress.frame >= request.warmup_frames {
        progress.triggered = true;

        // Logged so a blank image can be told apart from an empty scene.
        let camera = cameras
            .single()
            .map(|t| format!("{:?}", t.translation))
            .unwrap_or_else(|_| "<none>".into());
        let player = subjects
            .single()
            .map(|(t, h)| format!("{:?} hp {:.0}", t.translation, h.current))
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
    wanted: Res<crate::crime::wanted::Wanted>,
    police: Query<(
        &crate::ai::police::PoliceUnit,
        &Transform,
        &crate::vehicle::controller::VehicleState,
    )>,
    mut crimes: MessageWriter<crate::crime::events::CrimeReported>,
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

    // Once stopped, keep committing crimes. Gunfire needs no witness, so this
    // holds the heat up and forces the pursuit half of the loop to actually
    // run instead of quietly cooling off.
    if *ticks > 760 && (*ticks).is_multiple_of(64) {
        crimes.write(crate::crime::events::CrimeReported {
            kind: crate::crime::events::CrimeKind::Gunfire,
            position: transform.translation,
        });
    }

    if (*ticks).is_multiple_of(48)
        && let Ok((car, state)) = states.get(*vehicle)
    {
        let chasing = police.iter().filter(|(unit, _, _)| unit.has_sight).count();
        let nearest = police
            .iter()
            .map(|(unit, police_transform, police_state)| {
                (
                    police_transform.translation.distance(transform.translation),
                    unit.state,
                    police_state.speed_kph(),
                    unit.route.len(),
                )
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        info!(
            "t={:>4} {:>5.1} km/h  wheels {}/4  |  {} stars, heat {:>5.1}, unseen {:>4.1}s  |               police {} ({} with eyes on)",
            *ticks,
            state.speed_kph(),
            state.grounded_wheels(),
            wanted.stars(),
            wanted.heat(),
            wanted.since_seen,
            police.iter().len(),
            chasing,
        );
        if let Some((distance, state, speed, route)) = nearest {
            info!(
                "      nearest unit {distance:>6.1}m  {state:?}  {speed:>5.1} km/h  route {route}"
            );
        }
        let _ = car;
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
