//! The heads-up display.
//!
//! There is no health, no money and no wanted level to show, because none of
//! those exist any more. What is left is the only number the game is about: how
//! the city feels. It is shown three ways — the player's own face, their own
//! mood, and the average of everyone resident — because the joke is the gap
//! between them. A delighted face in a furious street is funnier than either.
//!
//! The words on screen are German, matching `ui::menu`. Everything a player
//! reads is; everything a developer reads — the dev panel, the logs, the code
//! itself — is English.

use bevy::prelude::*;
use bevy::text::FontSize;
use leafwing_input_manager::prelude::ActionState;

use super::minimap::{MapOpen, MinimapImage};
use crate::core::schedule::GameSet;
use crate::mood::face::{self, FaceAssets};
use crate::mood::feeling::CityMood;
use crate::player::input::Action;

const PANEL: Color = Color::srgba(0.05, 0.06, 0.09, 0.62);
const INK: Color = Color::srgb(0.93, 0.95, 0.98);
/// The bar colours at furious, indifferent and delighted.
const SOUR: Color = Color::srgb(0.86, 0.19, 0.16);
const FLAT: Color = Color::srgb(0.95, 0.72, 0.16);
const SWEET: Color = Color::srgb(0.36, 0.78, 0.34);

#[derive(Component)]
struct MinimapFrame;

/// Which mood a bar is showing. One marker with a discriminant rather than two
/// marker types, so the widget code is one loop.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Meter {
    Own,
    City,
}

/// On the caption above a bar. Carried so the text is addressable later; the
/// bars themselves are found by their [`Meter`].
#[derive(Component)]
struct Caption;

#[derive(Component)]
struct FacePortrait;

#[derive(Component)]
struct RageBanner;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_hud).add_systems(
            Update,
            (toggle_map, size_map_frame, show_the_mood)
                .chain()
                .in_set(GameSet::Ui),
        );
    }
}

/// A proportion, drawn as a filled track.
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

/// The colour a mood reads at, red through amber to green.
fn tint(mood: f32) -> Color {
    let mood = mood.clamp(-1.0, 1.0);
    let (from, to, t) = if mood < 0.0 {
        (SOUR, FLAT, mood + 1.0)
    } else {
        (FLAT, SWEET, mood)
    };
    from.mix(&to, t)
}

/// How much of a bar a mood fills. The track runs the whole range, so
/// indifference is a half-full bar rather than an empty one — an empty bar
/// reads as "nothing here" and a mood of zero is not nothing.
fn fill_fraction(mood: f32) -> f32 {
    (mood.clamp(-1.0, 1.0) + 1.0) * 0.5
}

fn spawn_hud(mut commands: Commands, minimap: Res<MinimapImage>, faces: Res<FaceAssets>) {
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
        children![
            (
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
            ),
            // --- right column: the face, and the two moods it sits between ---
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexEnd,
                    ..default()
                },
                children![(
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(10.0),
                        padding: UiRect::all(Val::Px(10.0)),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL),
                    children![
                        (
                            ImageNode::new(faces.portrait(face::LEVELS / 2)),
                            Node {
                                width: Val::Px(52.0),
                                height: Val::Px(52.0),
                                ..default()
                            },
                            FacePortrait,
                        ),
                        (
                            Node {
                                flex_direction: FlexDirection::Column,
                                ..default()
                            },
                            children![
                                label("Du", 12.0, INK, Caption),
                                bar(FLAT, Meter::Own),
                                label("Die Stadt", 12.0, INK, Caption),
                                bar(FLAT, Meter::City),
                            ],
                        ),
                    ],
                )],
            ),
            // --- and the announcement when the street turns on itself ---
            (
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(40.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Visibility::Hidden,
                RageBanner,
                children![(
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(30.0),
                        ..default()
                    },
                    TextColor(SOUR),
                )],
            ),
        ],
    ));
}

/// Puts the city's temperature on screen: the player's own face and mood, the
/// average of everybody resident, and a shout when a lot of them go red at once.
fn show_the_mood(
    city: Res<CityMood>,
    faces: Res<FaceAssets>,
    mut worn: Local<Option<usize>>,
    mut portraits: Query<&mut ImageNode, With<FacePortrait>>,
    mut fills: Query<(&Meter, &mut Node, &mut BackgroundColor)>,
    mut banners: Query<(&mut Visibility, &Children), With<RageBanner>>,
    mut shouts: Query<&mut Text>,
) {
    for (meter, mut node, mut colour) in &mut fills {
        let mood = match meter {
            Meter::Own => city.player,
            Meter::City => city.average,
        };
        node.width = Val::Percent(fill_fraction(mood) * 100.0);
        colour.0 = tint(mood);
    }

    let level = face::level_of(city.player);
    if *worn != Some(level) {
        *worn = Some(level);
        for mut portrait in &mut portraits {
            portrait.image = faces.portrait(level);
        }
    }

    for (mut visibility, children) in &mut banners {
        let showing = city.wave > 0.0;
        *visibility = if showing {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !showing {
            continue;
        }
        for &child in children {
            if let Ok(mut text) = shouts.get_mut(child) {
                let count = city.wave_size;
                **text = format!("Wut-Welle! {count} Bürger");
            }
        }
    }
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
