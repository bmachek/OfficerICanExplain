# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo run                      # debug; ~2-10s incremental after the first build
cargo run --release            # smoother frame rate, slower to compile
cargo run --features dev       # bevy dynamic_linking — fastest iteration
cargo test                     # ~266 unit tests, all inline #[cfg(test)] modules
cargo test citygen             # one module's tests (filter by name substring)
cargo clippy --all-targets -- -D warnings
cargo fmt
tools/fetch-materials.sh       # ~200 MB of CC0 PBR sets into assets/materials/ (optional)
```

Optional cargo features: `raytracing` (pulls in `bevy_solari`), `dlss` (needs an
NVIDIA GPU and the vendor SDK). Neither is on by default.

### Verifying rendering changes

There is no way to eyeball this game from a terminal except the capture harness.
`core::capture` renders to an offscreen texture (window capture returns black when
the OS never composited the window), holds a few warmup frames, writes a PNG and
exits:

```sh
cargo run -- --screenshot shots/street.png --at-node 300 --eye 1.7 --hour 21.5
cargo run -- --screenshot shots/city.png --at 0,620,900 --look 0,20,-200 --stream-radius 1800
tools/shoot.sh                 # the whole battery of framings into shots/
tools/shoot.sh --only street,night --out shots/after
```

`tools/shoot.sh` exists so a rendering change is judged against the last render
rather than against a memory of it: shoot the same framings before and after.
Pin `--hour` on any shot being compared — the clock and the weather run together,
so an unpinned shot drifts its own sky between runs. `--fps-log` reports median,
p95 and worst frame time. Full flag table is in README.md.

Capture mode is not just a camera: `core::capture::is_capture_mode()` gates the
dev panel off (`ui`) and mutes audio, and several systems check it. Anything that
would spoil an unattended shot should check it too.

## Architecture

Bevy 0.19 app; `main.rs` installs `DefaultPlugins` then one plugin per top-level
module. Physics is Avian 3D, input is leafwing-input-manager, the dev panel is
bevy_egui, saves are RON.

| Module | What lives there |
|---|---|
| `core` | States, schedule sets, `GameConfig` tunables, persisted settings/keybindings (`core::settings`), deterministic RNG, asset-root resolution, the screenshot harness |
| `world` | City generator, road graph, chunk streaming, day/night, weather, facades/LOD shells, window interiors, road wear, vegetation, props, procedural + scanned textures |
| `bounce` | The elastic simulation: bounce controller, impact response, launch, squash |
| `player` | Input mapping, on-foot movement, camera rig, enter/exit |
| `vehicle` | Arcade vehicle physics, specs, bodywork, damage, lights, parked-car spawning |
| `ai` | Traffic, pedestrians, shared steering, walk cycles |
| `render` | Quality presets, atmosphere, exposure, bloom, shadows, volumetrics, post stack |
| `ui` | HUD, minimap, egui dev tuning panel, the `Escape` pause menu |
| `audio` | Startup waveform synthesis, the sound bank, triggers |
| `save` | RON quick save / load |

The README's Layout table is ahead of the tree: it lists `crime`, `combat` and
`mission` modules, and controls for firing and a crosshair, none of which exist
in `src/` today (the crosshair and police vehicles were removed in a26c996).
Treat `src/` as the truth and README prose as design intent.

### Determinism is the load-bearing constraint

The whole city is regenerated from `GameConfig::world_seed` on demand — chunks
are respawned, not stored — so generation must be pure and reproducible. Hence
`core::rng`: every subsystem draws from its own stream (`stream_for(seed, key)`,
`stream_for_chunk(...)` for per-chunk work) with a fixed key from `rng::stream`.
Never share a stream between subsystems and never reuse a key value: with one
shared RNG, adding a draw in the building generator silently reshuffles every
street downstream. Subsystems that sample noise rather than draw use `key_for`.

Because the world derives from the seed, `save::SaveGame` stores only what does
not: seed, player position, hour. Bump `SAVE_VERSION` on any incompatible change.

### Schedule

`core::schedule::GameSet` is the one ordering for game logic:
`Input → Ai → Simulation → Camera → Ui`, chained in `Update`, with `Ai`,
`Simulation` and `Camera` gated on both `AppState::InGame` and
`InGameState::Playing`. Put new gameplay systems in a set rather than growing
`.after()` chains across plugins. Physics is deliberately outside it — Avian
owns `PhysicsSchedule`, and vehicle forces are applied in `FixedUpdate`
(`vehicle::controller`) because Avian clears forces each tick — which is also
why `ui::menu` pauses `Time<Physics>` itself rather than relying on the
`GameSet` gate alone when `Escape` opens the pause menu (`InGameState::Paused`).
States are `AppState` (Loading/Menu/InGame) with an `InGameState` sub-state;
startup currently skips straight into the game.

### Layout vs. entities

`world::citygen` builds the whole layout (blocks, streets, road graph) once and
keeps it resident — traffic, pursuit and the minimap query parts of the city the
player cannot see. Only meshes and colliders stream, per 250 m chunk, in
`world::streaming`.

### Rendering is preset-driven

Never switch a renderer feature on directly. A `QualityPreset` (low → photo)
resolves in `render::quality` to a flat `GraphicsSettings` block, which
`GraphicsSettings::downgrade` walks back to what the GPU actually reports, and
`render::sync_camera_stack` then makes the camera match — so preset changes take
effect live from the dev panel. `GraphicsSettings` is serialised into saves,
which is why the types are ours rather than Bevy's, and preset/downgrade rules
are pure functions so they can be unit-tested without a GPU.

### Assets are generated, or absent

No third-party art ships. Every texture is painted per-pixel at startup
(`world::texture`) and every sound is synthesised into a buffer
(`audio::synth`). Scanned CC0 PBR sets are an *optional* upgrade:
`world::material` returns `Option` for every lookup and callers fall back to the
procedural version, so a fresh clone with no `assets/materials/` runs identically
and just looks worse. Adding a scanned set means adding its name to both
`world::material::set` and `tools/fetch-materials.sh`. Bevy has no runtime mip
generator, so both modules build mip chains on the CPU (averaged in linear space
for sRGB images).

The asset root is resolved explicitly in `core::assets::root()` and handed to
`AssetPlugin`, because Bevy's default resolves against the executable and
`target/release/assets/` does not exist — the failure is silent.

Custom shaders live in `assets/shaders/` (`facade.wgsl`, `road.wgsl`) behind
`MaterialExtension`s. Uniform struct field order in Rust must match the WGSL
declaration order.

## Conventions

- Comments here explain *why*, often at length, and frequently record what was
  tried and rejected. Match that: a change that reverses a documented decision
  should update the comment that documents it.
- Crate-level lints in `main.rs` allow `dead_code` (foundations land a milestone
  before their callers), `clippy::type_complexity` and `clippy::too_many_arguments`
  (Bevy query filters and system params are the meaning). Clippy must otherwise
  pass with `-D warnings`.
- Tests are inline `#[cfg(test)]` modules next to the code, with sentence-shaped
  names (`the_root_is_the_source_trees_assets_directory`). Pure logic — layout,
  preset tables, road-graph queries — is tested; rendering is verified by capture.
- Feel constants belong in `core::config::GameConfig` so the dev panel can tune
  them at runtime, not as literals at the use site.
