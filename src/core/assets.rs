//! Finding `assets/`.
//!
//! Bevy resolves its asset root against the *executable*, falling back to
//! `BEVY_ASSET_ROOT` or the manifest directory. `cargo run --release` from the
//! project root puts the binary in `target/release`, so the default looks for
//! `target/release/assets/` — which does not exist, and the only symptom is
//! that every scanned texture silently fails to load and the fallbacks quietly
//! take over. That is a bad failure: the game still runs, just worse, and
//! nothing in the picture says why.
//!
//! So the root is resolved once, explicitly, and the same answer is given to
//! both `AssetPlugin` and the code that checks whether a material was ever
//! downloaded.

use std::path::{Path, PathBuf};

/// The `assets` directory, as an absolute path where one can be found.
///
/// Candidates in order of authority: an explicit `BEVY_ASSET_ROOT`, the source
/// tree this binary was built from, the directory the binary lives in, and the
/// working directory. The first that exists wins; if none do, the last is
/// returned so the asset server reports a sensible path when it complains.
pub fn root() -> PathBuf {
    let candidates = [
        std::env::var_os("BEVY_ASSET_ROOT").map(PathBuf::from),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        std::env::current_dir().ok(),
    ];

    let mut last = PathBuf::from("assets");
    for base in candidates.into_iter().flatten() {
        let assets = base.join("assets");
        if assets.is_dir() {
            return assets;
        }
        last = assets;
    }
    last
}

/// Whether `path`, relative to the asset root, is a readable file.
pub fn has(path: &str) -> bool {
    root().join(path).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_the_source_trees_assets_directory() {
        // The manifest directory is a candidate, and this test runs from a
        // checkout, so the directory the fetch script writes into must win.
        assert!(root().is_dir(), "assets/ should exist in a checkout");
        assert!(root().ends_with("assets"));
    }

    #[test]
    fn a_missing_file_is_reported_missing() {
        assert!(!has("materials/NotAMaterial/nope.jpg"));
    }
}
