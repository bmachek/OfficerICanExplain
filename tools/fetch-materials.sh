#!/usr/bin/env bash
# Fetches the optional scanned/recorded assets: PBR materials and sounds.
#
# Everything here is CC0 1.0 — public domain, no attribution required, no
# restrictions on use. The project is FOSS and any licence-compatible source
# would do (CC-BY with credit included), but so far every sound worth having
# has turned up under CC0, so the stronger guarantee is kept while it costs
# nothing. Materials come from ambientCG (https://ambientcg.com), sounds from
# OpenGameArt (https://opengameart.org) and Freesound (https://freesound.org).
#
# The freesound.org entries point at cdn previews (128kbps mp3) rather than
# the originals, because original downloads sit behind a login and the
# previews do not. The licence is the sound's licence either way, and the
# bank remixes everything to mono at its own rate on load, so the transcode
# is not the bottleneck.
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
# Sounds with no entry here stay synthesised — the fallback is per sound, and
# `audio::files` says so in the log at startup. That is currently the screech
# (deliberately synth: it has to track slip continuously), the spoken flummi
# voices (instruments by design, see `audio::bank`), and the three mouth
# noises nobody has recorded under a compatible licence yet: raspberry, fart
# and sorry. The bank loads each by name the moment a file appears, so a
# recording dropped into assets/sounds/ by hand works without touching this
# script.
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
    # The cheer, whistled by actual people (all CC0): elle-trudgett's
    # innocent whistle, elijahgoodson's tune, Willygoat's calm one. Three
    # takes because one whistle repeated is a doorbell — see `audio::bank`.
    "whistle-0|https://cdn.freesound.org/previews/146/146887_197046-hq.mp3"
    "whistle-1|https://cdn.freesound.org/previews/411/411578_7994683-hq.mp3"
    "whistle-2|https://cdn.freesound.org/previews/411/411062_7963328-hq.mp3"
    # An actual ACME swanee whistle sliding up (v0idation, CC0), for the arc
    # through the air; and a jaw-harp twang (magnuswaker, CC0) for a street
    # prop leaving its bolts.
    "wheee|https://cdn.freesound.org/previews/497/497092_942821-hq.mp3"
    "sproing|https://cdn.freesound.org/previews/540/540790_11537497-hq.mp3"
)
# These two live inside one zip (qubodup's CC0 car pack): "<name>|<member>".
CAR_PACK_URL="https://opengameart.org/sites/default/files/car_sound_effects_pack.zip"
CAR_PACK=(
    "engine|Car_Engine_Loop.ogg"
    "car-door|Car_Door_Close.ogg"
)
# And the taunt rotation's recordable half, from rubberduck's CC0 creature
# pack. cough_03 is the double cough — performed, like the synthesised one —
# and spit_01 is the closest to the synthesised length.
CREATURE_PACK_URL="https://opengameart.org/sites/default/files/80-CC0-creature-SFX_0.zip"
CREATURE_PACK=(
    "cough|cough_03.ogg"
    "spit|spit_01.ogg"
)
# Footsteps and the traffic bed, from rubberduck's second CC0 SFX hundred.
# The highway loop *is* the city ambience: what the mood mixer wants from
# `ambience` is anonymous distant traffic, which is exactly this.
SFX100_PACK_URL="https://opengameart.org/sites/default/files/sfx_100_v2.zip"
SFX100_PACK=(
    "footstep|sfx100v2_footstep_01.ogg"
    "ambience|sfx100v2_loop_highway.ogg"
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

# The same dance for the creature pack. Duplicated rather than factored into a
# function because macOS still ships bash 3.2, which has no array namerefs.
need_pack=false
for entry in "${CREATURE_PACK[@]}"; do
    name="${entry%%|*}"
    member="${entry#*|}"
    [[ -f "$SOUNDS_DEST/$name.${member##*.}" && "$force" == false ]] || need_pack=true
done
if [[ "$need_pack" == true ]]; then
    echo "fetch   creature sound pack"
    pack="$(mktemp -t creaturepack.XXXXXX).zip"
    if curl -fsSL --retry 3 --retry-delay 2 -o "$pack" "$CREATURE_PACK_URL"; then
        for entry in "${CREATURE_PACK[@]}"; do
            name="${entry%%|*}"
            member="${entry#*|}"
            unzip -qop "$pack" "$member" > "$SOUNDS_DEST/$name.${member##*.}"
        done
    else
        echo "        failed; skipping (the game synthesises them instead)" >&2
    fi
    rm -f "$pack"
fi

# And once more for the SFX hundred (same bash-3.2 duplication as above).
need_pack=false
for entry in "${SFX100_PACK[@]}"; do
    name="${entry%%|*}"
    member="${entry#*|}"
    [[ -f "$SOUNDS_DEST/$name.${member##*.}" && "$force" == false ]] || need_pack=true
done
if [[ "$need_pack" == true ]]; then
    echo "fetch   sfx hundred pack"
    pack="$(mktemp -t sfx100pack.XXXXXX).zip"
    if curl -fsSL --retry 3 --retry-delay 2 -o "$pack" "$SFX100_PACK_URL"; then
        for entry in "${SFX100_PACK[@]}"; do
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
