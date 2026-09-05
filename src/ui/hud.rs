//! The heads-up display.

use bevy::prelude::*;
use bevy::text::FontSize;
use leafwing_input_manager::prelude::ActionState;

use super::minimap::{MapOpen, MinimapImage};
use crate::core::schedule::GameSet;
use crate::player::input::Action;

const PANEL: Color = Color::srgba(0.05, 0.06, 0.09, 0.62);
const INK: Color = Color::srgb(0.93, 0.95, 0.98);

#[derive(Component)]
struct MinimapFrame;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_hud).add_systems(
            Update,
            (toggle_map, size_map_frame).chain().in_set(GameSet::Ui),
        );
    }
}

/// A labelled proportion. Nothing reads one yet — the vitals it used to show
/// went with the crime loop, and the mood meters that replace them land next.
fn bar(fill: Color, marker: impl Component) -> impl Bundle {
    (
        Node {
            width: Val::Px(168.0),
            height: Val::Px(10.0),
            border: UiRect::all(Val::Px(1.0)),
            margin: UiRect::top(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.18)),
        children![(
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(fill),
            marker,
        )],
    )
}

fn label(text: &str, size: f32, color: Color, marker: impl Component) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        marker,
    )
}

fn spawn_hud(mut commands: Commands, minimap: Res<MinimapImage>) {
    commands.spawn((
        Name::new("HUD"),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            padding: UiRect::all(Val::Px(16.0)),
            justify_content: JustifyContent::SpaceBetween,
            ..default()
        },
        // The HUD is decoration; it must never eat clicks meant for the world.
        Pickable::IGNORE,
        GlobalZIndex(10),
        children![(
            // --- left column: the minimap, anchored to the bottom ---
            Node {
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                height: Val::Percent(100.0),
                ..default()
            },
            children![(
                Node {
                    width: Val::Px(170.0),
                    height: Val::Px(170.0),
                    border: UiRect::all(Val::Px(2.0)),
                    overflow: Overflow::clip(),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.22)),
                BackgroundColor(PANEL),
                MinimapFrame,
                children![(
                    ImageNode::new(minimap.0.clone()),
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                )],
            )],
        )],
    ));
}

fn toggle_map(mut map_open: ResMut<MapOpen>, actions: Query<&ActionState<Action>>) {
    let Ok(action_state) = actions.single() else {
        return;
    };
    if action_state.just_pressed(&Action::Map) {
        map_open.0 = !map_open.0;
    }
}

/// Sizes the map panel from the state rather than from the keypress, so
/// anything that sets `MapOpen` — a menu, a script, the capture tool — gets the
/// full-size map instead of a zoomed-out city crammed into a minimap frame.
fn size_map_frame(map_open: Res<MapOpen>, mut frames: Query<&mut Node, With<MinimapFrame>>) {
    if !map_open.is_changed() {
        return;
    }
    // One texture serves both views: the camera zooms out, the frame grows.
    let size = if map_open.0 { 640.0 } else { 170.0 };
    for mut node in &mut frames {
        node.width = Val::Px(size);
        node.height = Val::Px(size);
    }
}
