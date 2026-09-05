#!/usr/bin/env bash
# Renders the standard battery of framings, so a rendering change can be judged
# against the last one rather than against a memory of it.
#
# Every framing here exists because it is the only view that shows something:
# the aerial is the only one that shows the roofline and the shadow distance,
# `--at-car` is the only one that shows bodywork, and the night, rain and
# overcast shots are where the lighting model is actually under load. Adding a
# framing is cheap; the discipline is shooting the same ones every time.
#
# The dawn framing is the one carrying hard coordinates rather than `--at-node`,
# and they are the default seed's: it has to look *into* the low sun down an open
# street, because that is the direction the air lights up from, and a node number
# says nothing about which way its streets run.
#
# Weather is pinned in every framing, including the ones that do not mention it.
# It runs on the game clock now, so a shot without `--hour` would drift its own
# sky between two runs and turn every comparison into an argument about the
# weather rather than about the change under test.
#
#   tools/shoot.sh                       # the default preset, into shots/
#   tools/shoot.sh --quality ultra       # one named preset
#   tools/shoot.sh --all-presets         # every preset, into shots/<preset>/
#   tools/shoot.sh --out shots/before    # somewhere else, for a before/after
#   tools/shoot.sh --only street,night   # just these framings
#
# Frame times are logged for every shot and collected at the end. A screenshot
# says a change looks right; it says nothing about whether it can be afforded.
set -euo pipefail

cd "$(dirname "$0")/.."

# name|flags. Anything with a fixed --hour also freezes the clock, so the
# warmup frames cannot drift the sky between two runs of the same shot.
FRAMINGS=(
    "aerial|--at 0,620,900 --look 0,20,-200 --stream-radius 1800 --hour 10"
    "street|--at-node 300 --eye 1.7 --hour 16"
    "dusk|--at-node 300 --eye 1.7 --hour 19.4"
    "night|--at-node 300 --eye 1.7 --hour 22.5"
    "rain|--at-node 300 --eye 1.7 --hour 21.5 --wet 0.9 --cover 1"
    "dawn|--at -163.6,1.7,-744.3 --look 836,25,-604 --hour 6.4"
    "overcast|--at-node 300 --eye 1.7 --hour 13 --cover 1"
    "facade|--at -163.0,4.5,-759.5 --look -172.6,6.6,-759.5 --hour 15"
    "park|--at 522,1.8,-986 --look 610,7,-902 --hour 9"
    # Down onto the carriageway from a first-floor window. The only framing
    # that shows the road *surface*: from head height a decal is four pixels
    # tall and every one of them is on the horizon.
    "wear|--at -163.6,12,-744.3 --look -138.9,0,-740.8 --hour 11"
    "cars|--at-car --hour 11"
    "damage|--at-car --damage 0.45 --hour 11"
    "showroom|--showroom --hour 11"
    "driving|--follow --drive --frames 2000 --hour 15"
    "map|--follow --map"
)

presets=()
out=""
only=""
profile="--release"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --quality)     presets=("$2"); shift 2 ;;
        --all-presets) presets=(low medium high ultra photo); shift ;;
        --out)         out="$2"; shift 2 ;;
        --only)        only="$2"; shift 2 ;;
        --debug)       profile=""; shift ;;
        *) echo "unknown flag: $1" >&2; exit 2 ;;
    esac
done
[[ ${#presets[@]} -eq 0 ]] && presets=(high)

# The capture harness renders to an offscreen texture but still opens a window,
# so an unattended run needs something for it to open onto. Wrapping in Xvfb
# only when there is no display keeps an interactive run untouched.
runner=()
if [[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
    if command -v xvfb-run >/dev/null; then
        runner=(xvfb-run -a)
        echo "no display; running under Xvfb"
    else
        echo "warning: no display and no xvfb-run — captures will probably fail" >&2
    fi
fi

# Build once up front. Otherwise the first shot's frame times include the tail
# of a compile, which is the sort of thing that gets read as a regression.
echo "building..."
cargo build $profile

summary=$(mktemp)
trap 'rm -f "$summary"' EXIT

for preset in "${presets[@]}"; do
    dir="${out:-shots$([[ ${#presets[@]} -gt 1 ]] && echo "/$preset")}"
    mkdir -p "$dir"

    for framing in "${FRAMINGS[@]}"; do
        name="${framing%%|*}"
        flags="${framing#*|}"

        if [[ -n "$only" && ",$only," != *",$name,"* ]]; then
            continue
        fi

        path="$dir/$name.png"
        printf 'shoot   %-9s %s\n' "$name" "$path"

        # Frame times go to the summary; everything else is Bevy's startup
        # chatter and is only worth seeing when the shot fails.
        log=$(mktemp)
        if "${runner[@]}" cargo run $profile -- \
                --screenshot "$path" --quality "$preset" --fps-log $flags \
                >"$log" 2>&1; then
            grep -h "frame times" "$log" \
                | sed "s|^|$preset/$name  |" >>"$summary" || true
        else
            echo "        FAILED — last lines:" >&2
            tail -20 "$log" >&2
            rm -f "$log"
            exit 1
        fi
        rm -f "$log"
    done
done

echo
echo "frame times"
echo "-----------"
cat "$summary"
