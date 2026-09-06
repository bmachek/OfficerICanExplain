//! What everybody in this city thinks of everybody else.
//!
//! The module owns the emotional half of the simulation: the number each flummi
//! carries around, the disposition that decides what moves it, and the face
//! that shows the answer. Nothing here is decoration — the face *is* the
//! readout, and it is the only one the game has.

pub mod apology;
pub mod face;
pub mod feeling;
pub mod grudge;
pub mod provoke;
pub mod voice;

use bevy::prelude::*;

use crate::core::schedule::GameSet;

pub struct MoodPlugin;

impl Plugin for MoodPlugin {
    fn build(&self, app: &mut App) {
        // A screenshot is taken by a process nobody is listening to, and the
        // capture run is scripted; the rest of the audio wiring bows out the
        // same way in `crate::audio`.
        if !crate::core::capture::is_capture_mode() {
            app.add_plugins(voice::VoicePlugin);
        }

        app.add_plugins((
            feeling::FeelingPlugin,
            provoke::ProvokePlugin,
            grudge::GrudgePlugin,
            apology::ApologyPlugin,
        ))
        .add_systems(Startup, build_faces)
        .add_systems(
            Update,
            // After the mood systems, which are also in `Ai`: the face is
            // read from a mood that has already settled this frame.
            face::wear_the_mood
                .in_set(GameSet::Ai)
                .after(feeling::Feeling),
        );

        // Baked in only when a capture asked for it, so an ordinary run never
        // pays for the query. See `core::capture`.
        if let Some(request) = crate::core::capture::parse_args()
            && let Some(forced) = request.mood
        {
            app.insert_resource(ForcedMood(forced)).add_systems(
                Update,
                hold_the_mood
                    .in_set(GameSet::Ai)
                    .before(face::wear_the_mood),
            );
        }
    }
}

fn build_faces(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let started = std::time::Instant::now();
    let faces = face::build_assets(&mut images, &mut materials);
    info!(
        "{} faces painted in {:.1?}",
        face::LEVELS,
        started.elapsed()
    );
    commands.insert_resource(faces);
}

/// A mood held still for a screenshot.
#[derive(Resource)]
struct ForcedMood(f32);

/// Pins every face at the requested mood, so the thirteen of them can be shot
/// without arranging for the city to actually feel that way first.
fn hold_the_mood(forced: Res<ForcedMood>, mut flummis: Query<&mut feeling::Mood>) {
    for mut mood in &mut flummis {
        mood.value = forced.0;
        mood.previous = forced.0;
    }
}
