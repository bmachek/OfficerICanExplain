// Scanned wall grain over a painted facade.
//
// The facade texture drawn in `world::texture` covers a whole building face, so
// it holds the things that are *about* the building — where the windows are,
// which of them are lit, where the floor lines fall. What it cannot hold is
// material detail: stretched over forty metres of tower it gives about fifty
// pixels per metre, and a concrete scan needs four times that before it reads
// as concrete rather than as noise.
//
// So the grain is sampled separately, in world space, at its own true size.
// Nothing about it depends on how big the building is, which means one material
// per district and palette still covers the city — the alternative was a
// material per size bucket as well, and several hundred draw calls with it.
//
// The windows are the other half. What the painted texture can say about a
// pane is what colour it is, and a flat colour is what gives a generated
// facade away: real glass has a room behind it that moves against its own
// frame as you walk past. `glaze` puts one there, by following the view ray
// into a box behind the glass — no geometry, and no second draw.
//
// One file, two pipelines. Under `PREPASS_PIPELINE` this runs as the deferred
// fragment and writes a g-buffer; otherwise it shades forward as before. Both
// the grain and the rooms are computed once, above the branch, and neither
// pipeline has its own copy — which matters more than it sounds, because the
// mean-normalisation, the relief basis and the room's axes below are each
// subtle enough that two copies would drift.

#import bevy_pbr::{
    pbr_types::PbrInput,
    pbr_functions::alpha_discard,
    pbr_fragment::pbr_input_from_standard_material,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#endif

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither;
#endif

// Field order is `world::facade::FacadeSettings`'s, and has to stay it: a
// uniform is laid out by declaration order, and the two vectors lead because
// they align to sixteen bytes and would each open a hole anywhere else.
struct FacadeSettings {
    // Where the glass sits inside one cell above the ground floor, as
    // fractions of that cell: (u0, u1, v0, v1).
    pane: vec4<f32>,
    // The same for the ground storey, which is a shopfront in most classes.
    ground: vec4<f32>,
    // Bays across a building face, and storeys up it.
    grid: vec2<f32>,
    // Metres of wall covered by one repeat of the grain.
    tile: f32,
    // How far the grain is allowed to modulate the wall's colour.
    strength: f32,
    // How far its normal map is allowed to tilt the surface.
    relief: f32,
    // Above 0.5, the grain is sampled turned ninety degrees.
    swap: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> settings: FacadeSettings;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var grain_color: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var grain_color_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var grain_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var grain_normal_sampler: sampler;

// Lays the scanned grain over a facade's own painted surface.
//
// Takes and returns the whole `PbrInput` rather than writing through a pointer,
// so the forward and deferred branches below are each one line and there is no
// second copy of any of this to fall out of step.
fn dress(input: PbrInput) -> PbrInput {
    var pbr_input = input;

    // Every wall in this city stands on the street grid, so the dominant axis
    // of the normal *is* the plane the wall lies in. That makes the usual
    // triplanar blend unnecessary: there is no diagonal face for its seams to
    // appear on, and picking one projection outright costs two texture fetches
    // instead of six.
    let facing = abs(pbr_input.world_normal);
    var plane: vec2<f32>;
    var tangent: vec3<f32>;
    if facing.y > max(facing.x, facing.z) {
        plane = pbr_input.world_position.xz;
        tangent = vec3(1.0, 0.0, 0.0);
    } else if facing.x > facing.z {
        plane = pbr_input.world_position.zy;
        tangent = vec3(0.0, 0.0, 1.0);
    } else {
        plane = pbr_input.world_position.xy;
        tangent = vec3(1.0, 0.0, 0.0);
    }
    // Turning the grain a quarter turn is enough to stop two walls cut from the
    // same photograph reading as the same wall — cheaper than a second scan,
    // and it costs nothing per fragment.
    var uv = plane / settings.tile;
    if settings.swap > 0.5 {
        uv = uv.yx;
    }

    // Glass is the metallic part of a facade and the wall is not, so metalness
    // is already a mask for "is this a window" — no extra channel needed.
    let wall = 1.0 - saturate(pbr_input.material.metallic * 2.0);

    // Normalised against the *texture's* average, so what the photograph
    // contributes is its variation and not its grey — the district's colour has
    // to survive. The average comes free: the top of the mip chain is a one-
    // texel reduction of the whole image, so asking for an absurd level of
    // detail returns exactly it.
    let grain = textureSample(grain_color, grain_color_sampler, uv).rgb;
    let average = textureSampleLevel(grain_color, grain_color_sampler, uv, 24.0).rgb;
    let mean = max(dot(average, vec3(0.3333)), 0.001);
    let modulation = mix(vec3(1.0), grain / mean, settings.strength * wall);
    pbr_input.material.base_color = vec4(
        pbr_input.material.base_color.rgb * modulation,
        pbr_input.material.base_color.a,
    );

    var packed = textureSample(grain_normal, grain_normal_sampler, uv).xyz * 2.0 - 1.0;
    if settings.swap > 0.5 {
        // The relief has to turn with the colour, or the mortar shadows fall
        // across courses that are not there.
        packed = vec3(packed.y, packed.x, packed.z);
    }
    let bitangent = cross(pbr_input.world_normal, tangent);
    let tilt = (tangent * packed.x + bitangent * packed.y) * settings.relief * wall;
    pbr_input.N = normalize(pbr_input.N + tilt);

    return pbr_input;
}

// ---------------------------------------------------------------------------
// What is behind the glass
// ---------------------------------------------------------------------------

// How deep the room behind a pane is, as a multiple of the pane's own width.
const ROOM_DEPTH: f32 = 1.35;
// How many rooms have something drawn across the window instead.
const BLINDS: f32 = 0.34;

struct Face {
    u: vec3<f32>,
    v: vec3<f32>,
}

// The world-space axes of one unit of facade UV.
//
// A wall is planar, so its world position is an affine function of its UV, and
// the screen-space derivatives of the two are a two-by-two system in the axes.
// Solving it recovers how many metres a whole building face measures — which
// the shader has no other way of knowing, because one material is shared by
// every building in a district and no two of them are the same size.
fn face_axes(world: vec3<f32>, uv: vec2<f32>) -> Face {
    let dpx = dpdx(world);
    let dpy = dpdy(world);
    let dux = dpdx(uv);
    let duy = dpdy(uv);
    let det = dux.x * duy.y - dux.y * duy.x;
    // Degenerate where the wall is edge-on to the screen, which is the one
    // place none of this is visible anyway.
    if abs(det) < 1e-12 {
        return Face(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0));
    }
    return Face(
        (dpx * duy.y - dpy * dux.y) / det,
        (dpy * dux.x - dpx * duy.x) / det,
    );
}

// Hoskins' hash, for a number per room rather than per city.
fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(vec3(p.x, p.y, p.x) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

// Puts a room behind every pane of glass.
//
// A window painted as one flat colour is what gives a generated facade away,
// and it gives it away worst after dark, when the lit ones are the brightest
// thing in the frame and every one of them is a flat rectangle. This is
// interior mapping: the view ray is followed into a box behind the glass, and
// the wall it lands on decides the colour — so the room slides against its own
// frame as the camera moves past it, the way a real one does. No geometry, no
// texture, no second draw. One ray against six planes.
fn glaze(input: PbrInput, uv: vec2<f32>) -> PbrInput {
    var pbr_input = input;

    // Taken before the branch below, because half a facade's fragments are not
    // glass and a derivative inside non-uniform control flow is undefined.
    let axes = face_axes(pbr_input.world_position.xyz, uv);

    // The same mask `dress` uses, read the other way up: glass is the metallic
    // part of a facade and the wall is not.
    if saturate(pbr_input.material.metallic * 2.0) < 0.5 {
        return pbr_input;
    }

    let cell = uv * settings.grid;
    let index = floor(cell);
    let within = cell - index;

    // The ground storey is a shopfront in every class but the house, and a
    // shopfront is a different rectangle in its cell than a window is.
    var pane = settings.pane;
    if index.y < 0.5 {
        pane = settings.ground;
    }
    let span = max(vec2(pane.y - pane.x, pane.w - pane.z), vec2(1e-3));
    // Where on the glass this fragment sits: across, and up.
    let at = saturate((within - vec2(pane.x, pane.z)) / span);

    // The pane's true size in metres. A room is not measured in texture space:
    // a shopfront is four metres across and a bathroom window is one, and what
    // is behind them has to be deep in the same proportion.
    let width = length(axes.u) * span.x / settings.grid.x;
    let height = length(axes.v) * span.y / settings.grid.y;
    let depth = width * ROOM_DEPTH;

    // A number that is exactly constant across one pane and different for the
    // next. The wall's distance from the origin identifies the face, and it is
    // constant over a flat one — where anything taken from the fragment's own
    // position would wobble in the last decimal and put a seam down the middle
    // of a window.
    let wall = round(
        (dot(pbr_input.world_position.xyz, pbr_input.world_normal)
            + pbr_input.world_normal.x * 17.0
            + pbr_input.world_normal.z * 31.0) * 4.0,
    );
    let key = index + vec2(wall * 0.13, wall * 0.37);
    let hue = hash21(key);
    let lamp = hash21(key + 19.7);
    let drawn = hash21(key + 41.3) < BLINDS;

    // Into the wall, in room units: x and y across the pane, z from the glass
    // back to the far wall.
    let into = -pbr_input.V;
    let step = vec3(
        dot(into, normalize(axes.u)) / width,
        dot(into, normalize(axes.v)) / height,
        dot(into, -pbr_input.world_normal) / depth,
    );
    // Zero anywhere in here is a division away from a NaN across a whole window.
    let bounded = max(abs(step), vec3(1e-5));
    let ray = select(bounded, -bounded, step < vec3(0.0));

    let start = vec3(at, 0.0);
    let exit = max((vec3(0.0) - start) / ray, (vec3(1.0) - start) / ray);
    let hit = min(exit.x, min(exit.y, exit.z));

    // Which of the five surfaces it landed on. The back wall faces the window
    // and catches what light there is; a side wall is turned away from it, a
    // ceiling is painted white and a floor is carpet. The spread between them
    // is the whole illusion — it is the only thing telling the eye that the
    // corner it can see is a corner.
    var tone: f32;
    if exit.z <= exit.x && exit.z <= exit.y {
        tone = 1.0;
    } else if exit.y <= exit.x {
        tone = select(0.74, 0.28, ray.y < 0.0);
    } else {
        tone = 0.44;
    }
    // Everything recedes: the far corner of a room is darker than the wall
    // beside the window, whatever it is made of. Divided rather than
    // subtracted because a glancing ray crosses several room-widths before it
    // hits anything, and a subtraction would take that past black.
    tone /= 1.0 + hit * 0.25;

    // Interiors are painted and papered and lit by whatever is inside them, so
    // they are not one grey. A little spread is what stops a facade reading as
    // a printed sheet of identical holes.
    //
    // Dark, though — much darker than any wall really is. The room is shaded
    // by the same sun and the same normal as the wall around it, because it is
    // the same fragment, so its colour has to stand for the albedo *and* for
    // the fraction of the daylight that ever reaches the back of a room, which
    // is a few percent. Painted at the value a wall is actually painted, every
    // window in the city lights up like a lightbox.
    var room = mix(vec3(0.052, 0.060, 0.074), vec3(0.098, 0.082, 0.064), hue)
        * (0.55 + lamp * 0.8);
    if drawn {
        // A blind, and the cheapest variety there is: a room you cannot see
        // into still reads as a room, and it reads as a different one.
        room = vec3(0.20, 0.19, 0.175) * (0.78 + lamp * 0.44);
        tone = 1.0;
    }
    // The ground storey is a shop: lit, stocked, and painted to be looked into.
    if index.y < 0.5 {
        room *= 1.7;
    }

    pbr_input.material.base_color = vec4(room * tone, pbr_input.material.base_color.a);
    // Glass is a dielectric. Metalness was standing in for one to get any
    // reflection out of it at all, and with a room behind it that trade goes
    // the wrong way: a metal's base colour tints its reflection instead of
    // showing through, so the room would have come out as a coloured mirror.
    // A dielectric at this roughness still mirrors the street at a glancing
    // angle, which is when a window actually reflects anything.
    pbr_input.material.metallic = 0.0;
    // And a lit room lights its own back wall, so the parallax survives after
    // dark — which is the hour this whole function exists for.
    pbr_input.material.emissive = vec4(
        pbr_input.material.emissive.rgb * tone,
        pbr_input.material.emissive.a,
    );

    return pbr_input;
}

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var in = vertex_output;

    // Halfway through a level-of-detail crossfade, drop this fragment or the
    // one from the other level, by a screen-space pattern. Without it a
    // building swaps detail in one frame and the whole street blinks.
#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color =
        alpha_discard(pbr_input.material, pbr_input.material.base_color);

    pbr_input = dress(pbr_input);
#ifdef VERTEX_UVS
    pbr_input = glaze(pbr_input, in.uv);
#endif

#ifdef PREPASS_PIPELINE
    // Write the grain into the g-buffer and let the deferred lighting pass
    // shade it. The relief survives, because the gbuffer stores `N` rather than
    // the geometric normal; so does the modulated colour.
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif

    return out;
}
