#!/usr/bin/env bash
# Fetches the scanned PBR materials the city is textured with.
#
# Everything here is from ambientCG (https://ambientcg.com) and is released
# under CC0 1.0 — public domain, no attribution required, no restrictions on
# use. That licence is the reason these specific sets were chosen: the game
# ships them without owing anybody anything.
#
# The download is ~143 MB and lands in assets/materials/, which is gitignored.
# The game runs without it: `world::texture` generates a procedural stand-in for
# every material it cannot find on disk, so a fresh clone still starts.
#
#   tools/fetch-materials.sh          # fetch anything missing
#   tools/fetch-materials.sh --force  # re-fetch everything
set -euo pipefail

cd "$(dirname "$0")/.."
DEST="assets/materials"
RESOLUTION="2K-JPG"

# The set the renderer looks for. Adding one here is all it takes for
# `world::texture` to prefer it over its procedural version.
MATERIALS=(
    Asphalt031        # road surface
    PavingStones138   # pavement slabs
    Concrete034       # downtown and midtown facades
    Bricks097         # residential facades
    Gravel023         # flat roofs
    Grass005          # parks
)

force=false
[[ "${1:-}" == "--force" ]] && force=true

mkdir -p "$DEST"
for material in "${MATERIALS[@]}"; do
    target="$DEST/$material"
    if [[ -d "$target" && "$force" == false ]]; then
        echo "have    $material"
        continue
    fi

    archive="$(mktemp -t "$material.XXXXXX").zip"
    url="https://ambientcg.com/get?file=${material}_${RESOLUTION}.zip"

    echo "fetch   $material"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$archive" "$url"; then
        echo "        failed; skipping" >&2
        rm -f "$archive"
        continue
    fi

    rm -rf "$target"
    mkdir -p "$target"
    # -j: the archives are flat already, but this guarantees it.
    unzip -qoj "$archive" -d "$target"
    rm -f "$archive"

    # Drop what the renderer will never read, so the directory is a manifest of
    # what is actually used rather than of what happened to be in the zip.
    find "$target" -type f ! -name '*.jpg' -delete
done

echo
du -sh "$DEST"
