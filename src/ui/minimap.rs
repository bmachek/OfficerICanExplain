//! Rotating minimap.
//!
//! Rendered by a second orthographic camera looking straight down at the
//! player, into an offscreen texture the HUD then displays. Drawing the road
//! graph by hand into a UI canvas would be more code and would show a diagram
//! of the city rather than the city — traffic, police and the player's own car
//! come along for free this way, which is most of what a minimap is for.
//!
//! The camera's yaw tracks the player so the map turns under a fixed heading
//! marker, the way every game in this genre does it.

use bevy::asset::RenderAssetUsages;
use bevy::camera::{ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

use crate::core::schedule::GameSet;
use crate::player::camera::CameraRig;
use crate::player::interact::Driving;
use crate::player::on_foot::Player;

const TEXTURE_SIZE: u32 = 320;
/// How high the map camera sits. Must clear the tallest downtown tower.
const ALTITUDE: f32 = 260.0;
/// World metres visible across the minimap.
pub const CLOSE_ZOOM: f32 = 190.0;
/// World metres visible on the full map screen.
pub const MAP_ZOOM: f32 = 1600.0;

#[derive(Component)]
pub struct MinimapCamera;

#[derive(Resource, Clone)]
pub struct MinimapImage(pub Handle<Image>);

/// True while the full-screen map is open.
#[derive(Resource, Default, Debug)]
pub struct MapOpen(pub bool);

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_minimap_camera)
            .add_systems(Update, track_player.in_set(GameSet::Ui));
    }
}

fn spawn_minimap_camera(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = Extent3d {
        width: TEXTURE_SIZE,
        height: TEXTURE_SIZE,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);

    commands.spawn((
        Name::new("Minimap Camera"),
        MinimapCamera,
        Camera3d::default(),
        Camera {
            // Negative order so it renders before the main view.
            order: -1,
            ..default()
        },
        // The world is drawn deferred (see `render`), and a deferred material
        // is skipped outright by the forward opaque pass rather than falling
        // back to it. Without a g-buffer of its own this camera would render a
        // black square, so it gets one — at 320 by 320 that is nothing, and the
        // alternative is keeping a second renderer path alive for one widget.
        //
        // Everything else the main camera carries is still deliberately absent:
        // this is a flat top-down diagram, and bloom or ambient occlusion would
        // cost real time to make it worse.
        bevy::core_pipeline::prepass::DepthPrepass,
        bevy::core_pipeline::prepass::DeferredPrepass,
        RenderTarget::Image(ImageRenderTarget {
            handle: handle.clone(),
            scale_factor: 1.0,
        }),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: CLOSE_ZOOM,
            },
            near: 0.1,
            far: ALTITUDE * 2.0,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, ALTITUDE, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));

    commands.insert_resource(MinimapImage(handle));
}

fn track_player(
    map_open: Res<MapOpen>,
    players: Query<(&Transform, Option<&Driving>), (With<Player>, Without<MinimapCamera>)>,
    rigs: Query<&CameraRig>,
    // Explicitly disjoint from the camera query below: the minimap camera has a
    // Transform too, and Bevy cannot prove the marker filters are exclusive.
    vehicles: Query<&Transform, (Without<Player>, Without<MinimapCamera>)>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<MinimapCamera>>,
) {
    let Ok((player, driving)) = players.single() else {
        return;
    };
    let Ok((mut transform, mut projection)) = cameras.single_mut() else {
        return;
    };

    // Centre on the vehicle when driving, so the map leads where you are going.
    let focus = driving
        .and_then(|d| vehicles.get(d.0).ok())
        .map(|v| v.translation)
        .unwrap_or(player.translation);

    // Heading comes from the camera rig, not the body: the map should follow
    // where the player is looking, which is how they are actually navigating.
    let yaw = rigs.single().map(|rig| rig.yaw).unwrap_or(0.0);

    transform.translation = Vec3::new(focus.x, ALTITUDE, focus.z);
    // Look straight down, rolled so that the player's forward points up the map.
    transform.rotation =
        Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);

    if let Projection::Orthographic(ortho) = &mut *projection {
        ortho.scaling_mode = ScalingMode::FixedVertical {
            viewport_height: if map_open.0 { MAP_ZOOM } else { CLOSE_ZOOM },
        };
    }
}
