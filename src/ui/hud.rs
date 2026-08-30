//! The heads-up display.

use bevy::prelude::*;
use bevy::text::FontSize;
use leafwing_input_manager::prelude::ActionState;

use super::minimap::{MapOpen, MinimapImage};
use crate::combat::health::Health;
use crate::combat::weapons::Weapon;
use crate::core::schedule::GameSet;
use crate::crime::wanted::Wanted;
use crate::mission::Money;
use crate::mission::framework::ActiveMission;
use crate::player::input::Action;
use crate::player::on_foot::Player;

const PANEL: Color = Color::srgba(0.05, 0.06, 0.09, 0.62);
const INK: Color = Color::srgb(0.93, 0.95, 0.98);
const HEALTH: Color = Color::srgb(0.55, 0.82, 0.42);
const ARMOR: Color = Color::srgb(0.45, 0.66, 0.92);
const STAR_LIT: Color = Color::srgb(1.0, 0.78, 0.20);
const STAR_DARK: Color = Color::srgba(1.0, 0.78, 0.20, 0.16);

#[derive(Component)]
struct HealthFill;
#[derive(Component)]
struct ArmorFill;
#[derive(Component)]
struct StarsText;
#[derive(Component)]
struct MoneyText;
#[derive(Component)]
struct WeaponText;
#[derive(Component)]
struct ObjectiveText;
#[derive(Component)]
struct MinimapFrame;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_hud).add_systems(
            Update,
            (refresh_hud, toggle_map, size_map_frame)
                .chain()
                .in_set(GameSet::Ui),
        );
    }
}

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
        children![
            // --- left column: minimap and vitals, anchored to the bottom ---
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::FlexEnd,
                    height: Val::Percent(100.0),
                    ..default()
                },
                children![
                    (
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
                    ),
                    bar(HEALTH, HealthFill),
                    bar(ARMOR, ArmorFill),
                ],
            ),
            // --- right column: stars, money, weapon ---
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexEnd,
                    justify_content: JustifyContent::SpaceBetween,
                    height: Val::Percent(100.0),
                    ..default()
                },
                children![
                    (
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::FlexEnd,
                            row_gap: Val::Px(2.0),
                            ..default()
                        },
                        children![
                            label("", 30.0, STAR_LIT, StarsText),
                            label("$0", 26.0, INK, MoneyText),
                        ],
                    ),
                    label("Pistol  90", 18.0, INK, WeaponText),
                ],
            ),
        ],
    ));

    // Objective banner, centred at the top.
    commands.spawn((
        Name::new("Objective"),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(18.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-180.0)),
            width: Val::Px(360.0),
            justify_content: JustifyContent::Center,
            padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
            border_radius: BorderRadius::all(Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(PANEL),
        Pickable::IGNORE,
        GlobalZIndex(10),
        children![label("", 17.0, INK, ObjectiveText)],
    ));
}

fn refresh_hud(
    wanted: Res<Wanted>,
    money: Res<Money>,
    active: Option<Res<ActiveMission>>,
    players: Query<(&Health, &Weapon), With<Player>>,
    mut fills: ParamSet<(
        Query<&mut Node, With<HealthFill>>,
        Query<&mut Node, With<ArmorFill>>,
    )>,
    mut texts: ParamSet<(
        Query<(&mut Text, &mut TextColor), With<StarsText>>,
        Query<&mut Text, With<MoneyText>>,
        Query<&mut Text, With<WeaponText>>,
        Query<&mut Text, With<ObjectiveText>>,
    )>,
) {
    if let Ok((health, weapon)) = players.single() {
        if let Ok(mut node) = fills.p0().single_mut() {
            node.width = Val::Percent(health.fraction() * 100.0);
        }
        if let Ok(mut node) = fills.p1().single_mut() {
            // Armour is scaled against the same 100 points as health, so the
            // two bars read as comparable amounts of protection.
            node.width = Val::Percent((health.armor / health.max * 100.0).clamp(0.0, 100.0));
        }
        if let Ok(mut text) = texts.p2().single_mut() {
            **text = format!("{}  {}", weapon.kind.name(), weapon.ammo);
        }
    }

    let stars = wanted.stars();
    if let Ok((mut text, mut color)) = texts.p0().single_mut() {
        **text = "★".repeat(stars as usize) + &"★".repeat((5 - stars) as usize);
        // Dim the whole row when clean rather than hiding it, so the player
        // learns where to look before they ever have a star.
        color.0 = if stars > 0 { STAR_LIT } else { STAR_DARK };
    }
    if let Ok(mut text) = texts.p1().single_mut() {
        **text = format!("${}", money.0);
    }

    if let Ok(mut text) = texts.p3().single_mut() {
        **text = match active.as_ref().and_then(|a| a.current()) {
            Some(objective) => objective.describe(),
            None => String::new(),
        };
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
