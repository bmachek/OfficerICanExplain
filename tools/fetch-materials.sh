#!/usr/bin/env bash
# Fetches the optional scanned/recorded assets: PBR materials and sounds.
#
# Everything here is CC0 1.0 — public domain, no attribution required, no
# restrictions on use. That licence is the reason these specific sets were
# chosen: the game ships them without owing anybody anything. Materials come
# from ambientCG (https://ambientcg.com), sounds from OpenGameArt
# (https://opengameart.org).
#
# The download lands in assets/materials/ and assets/sounds/, both gitignored.
# The game runs without either: `world::texture` generates a procedural
# stand-in for every material it cannot find on disk, and `audio::bank`
# synthesises every sound `audio::files` cannot find, so a fresh clone still
# starts.
#
# KEEP IN SYNC with tools/fetch-materials.bat — the Windows twin of this
# script. Any material or sound added here must be added there too.
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
    Concrete034       # facades
    Concrete046       # facades
    Bricks097         # facades
    Bricks104         # facades
    Bricks075A        # facades
    PaintedPlaster006 # facades
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

# ------------------------------------------------------------------ sounds ----
#
# One entry per sound bank name (see `audio::bank`): "<name>|<url>". The file
# keeps its source extension; `audio::files` tries wav/flac/ogg/mp3 in turn.
# Sounds with no good CC0 recording (screech, the flummi voices) simply have
# no entry here and stay synthesised — the fallback is per sound.
SOUNDS_DEST="assets/sounds"
SOUNDS=(
    "boing|https://opengameart.org/sites/default/files/boing.flac"
    "crash|https://opengameart.org/sites/default/files/qubodup-crash.ogg"
    # A bicycle horn on a car is the joke; a car horn is only a car.
    "honk|https://opengameart.org/sites/default/files/bicycle-horn-1.wav"
    "explosion|https://opengameart.org/sites/default/files/Chunky%20Explosion.mp3"
    "birdsong|https://opengameart.org/sites/default/files/park_ambience_birds.wav"
    "spray|https://opengameart.org/sites/default/files/park_ambience_river.wav"
    "uproar|https://opengameart.org/sites/default/files/crowd_shouting_0.ogg"
)
# These two live inside one zip (qubodup's CC0 car pack): "<name>|<member>".
CAR_PACK_URL="https://opengameart.org/sites/default/files/car_sound_effects_pack.zip"
CAR_PACK=(
    "engine|Car_Engine_Loop.ogg"
    "car-door|Car_Door_Close.ogg"
)

mkdir -p "$SOUNDS_DEST"
for entry in "${SOUNDS[@]}"; do
    name="${entry%%|*}"
    url="${entry#*|}"
    ext="${url##*.}"
    target="$SOUNDS_DEST/$name.$ext"
    if [[ -f "$target" && "$force" == false ]]; then
        echo "have    $name"
        continue
    fi
    echo "fetch   $name"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$target" "$url"; then
        echo "        failed; skipping (the game synthesises it instead)" >&2
        rm -f "$target"
    fi
done

need_pack=false
for entry in "${CAR_PACK[@]}"; do
    name="${entry%%|*}"
    member="${entry#*|}"
    [[ -f "$SOUNDS_DEST/$name.${member##*.}" && "$force" == false ]] || need_pack=true
done
if [[ "$need_pack" == true ]]; then
    echo "fetch   car sound pack"
    pack="$(mktemp -t carpack.XXXXXX).zip"
    if curl -fsSL --retry 3 --retry-delay 2 -o "$pack" "$CAR_PACK_URL"; then
        for entry in "${CAR_PACK[@]}"; do
            name="${entry%%|*}"
            member="${entry#*|}"
            unzip -qop "$pack" "$member" > "$SOUNDS_DEST/$name.${member##*.}"
        done
    else
        echo "        failed; skipping (the game synthesises them instead)" >&2
    fi
    rm -f "$pack"
fi

echo
du -sh "$DEST" "$SOUNDS_DEST"
