//! The pause menu: `Escape` opens it, and it is where save, load, and every
//! player-facing setting live.
//!
//! Pausing freezes the world rather than merely overlaying a screen on top of
//! it: `GameSet::Ai`/`Simulation`/`Camera` stop running (see
//! `core::schedule`) and [`avian3d::prelude::Time<Physics>`] is paused, so
//! nothing moves, nothing decides, and the mouse stops steering the view —
//! which is what frees it for the menu in the first place.

use avian3d::prelude::*;
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use leafwing_input_manager::prelude::ActionState;

use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::core::settings::{self, KeyBindings, RebindableAction};
use crate::core::states::{AppState, InGameState};
use crate::player::input::Action;
use crate::player::on_foot::Player;
use crate::render::quality::{Capabilities, QualityPreset};
use crate::world::timeofday::TimeOfDay;

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
enum MenuScreen {
    #[default]
    Root,
    SaveLoad,
    Settings,
    Controls,
}

/// The rebindable action currently waiting for a key press, if any. Escape
/// cancels it instead of being captured, so a player can always back out
/// without binding anything to it.
#[derive(Resource, Default)]
struct AwaitingRebind(Option<RebindableAction>);

/// Feedback line shown under the save/load buttons.
#[derive(Resource, Default)]
struct SaveLoadStatus(Option<String>);

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<MenuScreen>()
            .init_resource::<AwaitingRebind>()
            .init_resource::<SaveLoadStatus>()
            .add_systems(
                Update,
                (handle_pause_input, capture_rebind_key)
                    .chain()
                    .in_set(GameSet::Ui)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(
                OnEnter(InGameState::Paused),
                (open_menu, pause_physics, free_cursor),
            )
            .add_systems(OnExit(InGameState::Paused), unpause_physics)
            .add_systems(OnEnter(InGameState::Playing), grab_cursor)
            .add_systems(
                EguiPrimaryContextPass,
                pause_menu_ui.run_if(in_state(InGameState::Paused)),
            );
    }
}

fn open_menu(mut screen: ResMut<MenuScreen>, mut rebind: ResMut<AwaitingRebind>) {
    *screen = MenuScreen::Root;
    rebind.0 = None;
}

fn pause_physics(mut physics_time: ResMut<Time<Physics>>) {
    physics_time.pause();
}

fn unpause_physics(mut physics_time: ResMut<Time<Physics>>) {
    physics_time.unpause();
}

fn free_cursor(mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    for mut cursor in &mut windows {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

/// Locks the cursor to the window and hides it whenever gameplay is actually
/// running. Without this, mouse look could drag the pointer clean off the
/// window — nothing catches it again until the player alt-tabs back.
fn grab_cursor(mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    for mut cursor in &mut windows {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

/// `Escape`: open the menu, or navigate back out of it one level at a time —
/// out of a key-capture, then out of a submenu, then out of the menu itself.
fn handle_pause_input(
    actions: Query<&ActionState<Action>>,
    state: Res<State<InGameState>>,
    mut next: ResMut<NextState<InGameState>>,
    mut screen: ResMut<MenuScreen>,
    mut rebind: ResMut<AwaitingRebind>,
    config: Res<GameConfig>,
    keybindings: Res<KeyBindings>,
) {
    let Ok(action_state) = actions.single() else {
        return;
    };
    if !action_state.just_pressed(&Action::Pause) {
        return;
    }
    match state.get() {
        InGameState::Playing => next.set(InGameState::Paused),
        InGameState::Paused => {
            if rebind.0.take().is_some() {
                // Cancelled a capture; stay on the controls screen.
            } else if *screen == MenuScreen::Root {
                next.set(InGameState::Playing);
            } else {
                leave_settings_screen(&mut screen, &config, &keybindings);
            }
        }
    }
}

/// Takes the next key pressed while an action is awaiting a rebind. Escape is
/// reserved for cancelling (handled by `handle_pause_input`) rather than ever
/// being bound to something else.
fn capture_rebind_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut rebind: ResMut<AwaitingRebind>,
    mut keybindings: ResMut<KeyBindings>,
) {
    let Some(action) = rebind.0 else {
        return;
    };
    for key in keys.get_just_pressed() {
        if *key == KeyCode::Escape {
            continue;
        }
        keybindings.0.insert(action, *key);
        rebind.0 = None;
        break;
    }
}

/// Leaves a settings-like screen for the root menu, persisting whatever was
/// changed along the way. Used by both the "Zurück" buttons and the Escape
/// back-navigation, so nothing depends on remembering to click the button.
fn leave_settings_screen(screen: &mut MenuScreen, config: &GameConfig, keybindings: &KeyBindings) {
    *screen = MenuScreen::Root;
    settings::save(config, keybindings);
}

fn pause_menu_ui(
    mut contexts: EguiContexts,
    mut screen: ResMut<MenuScreen>,
    mut next_state: ResMut<NextState<InGameState>>,
    mut exit: MessageWriter<AppExit>,
    mut config: ResMut<GameConfig>,
    mut keybindings: ResMut<KeyBindings>,
    mut rebind: ResMut<AwaitingRebind>,
    mut status: ResMut<SaveLoadStatus>,
    caps: Res<Capabilities>,
    mut clock: ResMut<TimeOfDay>,
    mut players: Query<&mut Transform, With<Player>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    // Dims the game behind the menu, and — being interactable — eats clicks
    // that would otherwise land on the (frozen, but still rendered) world
    // beneath it.
    let viewport = ctx.viewport_rect();
    egui::Area::new(egui::Id::new("pause-dim"))
        .order(egui::Order::Background)
        .fixed_pos(viewport.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.allocate_response(viewport.size(), egui::Sense::click());
            ui.painter()
                .rect_filled(viewport, 0.0, egui::Color32::from_black_alpha(160));
        });

    egui::Window::new("pause-menu")
        .id(egui::Id::new("pause-menu"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(match *screen {
                    MenuScreen::Root => "Pausiert",
                    MenuScreen::SaveLoad => "Speichern & Laden",
                    MenuScreen::Settings => "Einstellungen",
                    MenuScreen::Controls => "Tastenbelegung",
                });
            });
            ui.separator();

            match *screen {
                MenuScreen::Root => root_screen(
                    ui,
                    &mut screen,
                    &mut next_state,
                    &mut exit,
                    &config,
                    &keybindings,
                ),
                MenuScreen::SaveLoad => save_load_screen(
                    ui,
                    &mut screen,
                    &config,
                    &keybindings,
                    &mut clock,
                    &mut players,
                    &mut status,
                ),
                MenuScreen::Settings => {
                    settings_screen(ui, &mut screen, &mut config, &keybindings, &caps)
                }
                MenuScreen::Controls => {
                    controls_screen(ui, &mut screen, &config, &mut keybindings, &mut rebind)
                }
            }
        });

    Ok(())
}

fn root_screen(
    ui: &mut egui::Ui,
    screen: &mut MenuScreen,
    next_state: &mut NextState<InGameState>,
    exit: &mut MessageWriter<AppExit>,
    config: &GameConfig,
    keybindings: &KeyBindings,
) {
    let full_width = egui::Vec2::new(ui.available_width(), 32.0);
    if ui
        .add_sized(full_width, egui::Button::new("Fortsetzen"))
        .clicked()
    {
        next_state.set(InGameState::Playing);
    }
    if ui
        .add_sized(full_width, egui::Button::new("Speichern & Laden"))
        .clicked()
    {
        *screen = MenuScreen::SaveLoad;
    }
    if ui
        .add_sized(full_width, egui::Button::new("Einstellungen"))
        .clicked()
    {
        *screen = MenuScreen::Settings;
    }
    if ui
        .add_sized(full_width, egui::Button::new("Tastenbelegung"))
        .clicked()
    {
        *screen = MenuScreen::Controls;
    }
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    if ui
        .add_sized(full_width, egui::Button::new("Spiel beenden"))
        .clicked()
    {
        settings::save(config, keybindings);
        exit.write(AppExit::Success);
    }
}

fn save_load_screen(
    ui: &mut egui::Ui,
    screen: &mut MenuScreen,
    config: &GameConfig,
    keybindings: &KeyBindings,
    clock: &mut TimeOfDay,
    players: &mut Query<&mut Transform, With<Player>>,
    status: &mut SaveLoadStatus,
) {
    let full_width = egui::Vec2::new(ui.available_width(), 28.0);

    if ui
        .add_sized(full_width, egui::Button::new("Speichern"))
        .clicked()
        && let Ok(transform) = players.single()
    {
        status.0 = Some(match crate::save::write_save(config, clock, transform) {
            Ok(()) => "Gespeichert.".to_string(),
            Err(error) => format!("Fehler beim Speichern: {error}"),
        });
    }

    let can_load = crate::save::save_exists();
    if ui
        .add_enabled(can_load, egui::Button::new("Laden").min_size(full_width))
        .clicked()
        && let Ok(mut transform) = players.single_mut()
    {
        status.0 = Some(match crate::save::read_save(clock, &mut transform) {
            Ok(()) => "Geladen.".to_string(),
            Err(error) => format!("Fehler beim Laden: {error}"),
        });
    }
    if !can_load {
        ui.label(
            egui::RichText::new("Kein Spielstand vorhanden.")
                .small()
                .weak(),
        );
    }

    if let Some(message) = &status.0 {
        ui.add_space(6.0);
        ui.label(message);
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    if ui
        .add_sized(full_width, egui::Button::new("Zurück"))
        .clicked()
    {
        leave_settings_screen(screen, config, keybindings);
    }
}

fn settings_screen(
    ui: &mut egui::Ui,
    screen: &mut MenuScreen,
    config: &mut GameConfig,
    keybindings: &KeyBindings,
    caps: &Capabilities,
) {
    ui.label(egui::RichText::new("Steuerung").strong());
    ui.add(
        egui::Slider::new(&mut config.camera.mouse_sensitivity, 0.0005..=0.01)
            .text("Mausempfindlichkeit"),
    );
    ui.checkbox(&mut config.camera.invert_look_y, "Maus Y invertieren");

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Audio").strong());
    ui.add(egui::Slider::new(&mut config.audio.master, 0.0..=1.0).text("Gesamt"));
    ui.add(egui::Slider::new(&mut config.audio.effects, 0.0..=1.5).text("Effekte"));
    ui.add(egui::Slider::new(&mut config.audio.ambience, 0.0..=1.5).text("Umgebung"));

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Grafik").strong());
    let requested = config.graphics.requested;
    let mut chosen = requested;
    egui::ComboBox::from_label("Qualität")
        .selected_text(requested.name())
        .show_ui(ui, |ui| {
            for preset in QualityPreset::ALL {
                ui.selectable_value(&mut chosen, preset, preset.name());
            }
        });
    if chosen != requested {
        config.graphics = chosen.settings().downgrade(*caps);
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    let full_width = egui::Vec2::new(ui.available_width(), 28.0);
    if ui
        .add_sized(full_width, egui::Button::new("Zurück"))
        .clicked()
    {
        leave_settings_screen(screen, config, keybindings);
    }
}

fn controls_screen(
    ui: &mut egui::Ui,
    screen: &mut MenuScreen,
    config: &GameConfig,
    keybindings: &mut KeyBindings,
    rebind: &mut AwaitingRebind,
) {
    egui::Grid::new("keybind-grid")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for action in RebindableAction::ALL {
                ui.label(action.label());
                let awaiting = rebind.0 == Some(action);
                let text = if awaiting {
                    "Taste drücken…".to_string()
                } else {
                    key_label(keybindings.key_for(action))
                };
                if ui
                    .add_sized([140.0, 22.0], egui::Button::new(text).selected(awaiting))
                    .clicked()
                {
                    rebind.0 = Some(action);
                }
                ui.end_row();
            }
        });

    if rebind.0.is_some() {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Drücke eine Taste, oder ESC zum Abbrechen.")
                .small()
                .weak(),
        );
    }

    ui.add_space(8.0);
    if ui.button("Zurücksetzen").clicked() {
        *keybindings = KeyBindings::default();
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    let full_width = egui::Vec2::new(ui.available_width(), 28.0);
    if ui
        .add_sized(full_width, egui::Button::new("Zurück"))
        .clicked()
    {
        leave_settings_screen(screen, config, keybindings);
    }
}

/// `KeyCode`'s `Debug` output is fine as a label except for letter keys,
/// where `KeyF` reads worse than the `F` a keyboard actually shows.
fn key_label(key: KeyCode) -> String {
    let raw = format!("{key:?}");
    raw.strip_prefix("Key").map(str::to_string).unwrap_or(raw)
}
