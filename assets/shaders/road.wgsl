// Standing water on the road.
//
// Wetness used to be one number applied to the whole road material: darken it,
// drop its roughness, done. That is right about what water does and wrong about
// where it is. A street does not go uniformly glossy in the rain — it puddles,
// in the ruts and the settled patches and against the kerb, and the parts
// between stay damp matte. A uniformly polished road reads as varnish.
//
// The mask has to be computed in world space. The road is a single quad forty
// kilometres across with its UVs multiplied by about six thousand, so anything
// sampled in UV repeats every six metres — puddles on a six-metre grid are a
// pattern, not weather. World space costs nothing extra here and the puddles
// come out metres across, which is the size they actually are.
//
// This runs in both pipelines for the same reason `facade.wgsl` does: the road
// is opaque, so it shades through the g-buffer, and screen-space reflections
// read that g-buffer. A puddle whose low roughness never reached the g-buffer
// would reflect nothing at all, which is the entire point of it.

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

struct RoadSettings {
    // How wet the road is overall, 0 to 1.
    wetness: f32,
    // Metres across one repeat of the puddle field.
    tile: f32,
    // Seconds, for the ripple. Held at zero when it is not actually raining, so
    // a merely damp road is still rather than trembling.
    time: f32,
    // How hard the rain is falling, which is what decides ripple strength.
    fall: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> road: RoadSettings;

// Value noise on a wrapping lattice, matching `world::texture`'s so the two
// agree about what a metre of grain looks like.
fn hash2(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    // Smoothstep rather than linear, or the lattice shows as a diamond grid.
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2(1.0, 0.0));
    let c = hash2(i + vec2(0.0, 1.0));
    let d = hash2(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var at = p;
    for (var octave = 0; octave < 4; octave += 1) {
        sum += value_noise(at) * amplitude;
        at *= 2.0;
        amplitude *= 0.5;
    }
    return sum;
}

// How deep the water is here, 0 to 1.
//
// Thresholded rather than used raw: water finds a level, so a puddle has an
// edge. A smooth gradient of wetness across the road is what varnish looks
// like, and the whole reason for this function is not to produce one.
fn depth(world_position: vec2<f32>, wetness: f32) -> f32 {
    let low = fbm(world_position / road.tile);
    // Rising wetness floods progressively more of the road: at a drizzle only
    // the lowest patches hold water, and by the time it is pouring most of the
    // surface is under it.
    let level = mix(0.72, 0.28, wetness);
    return smoothstep(level, level + 0.13, low);
}

fn wet(input: PbrInput) -> PbrInput {
    var pbr_input = input;
    if road.wetness <= 0.001 {
        return pbr_input;
    }

    let here = pbr_input.world_position.xz;
    let pool = depth(here, road.wetness);

    // Two states, blended. Damp asphalt is darker and a little glossier;
    // standing water is much darker and close to a mirror. Interpolating
    // between them rather than scaling one is what keeps the puddle edge
    // visible instead of washing it into a gradient.
    let damp = road.wetness * 0.35;
    let soak = max(damp, pool * road.wetness);

    let darken = 1.0 - soak * 0.55;
    pbr_input.material.base_color = vec4(
        pbr_input.material.base_color.rgb * darken,
        pbr_input.material.base_color.a,
    );

    // Not to zero. Water lying on asphalt still has the road's texture under
    // it, and a true mirror finish reads as sheet ice rather than as a puddle.
    pbr_input.material.perceptual_roughness =
        mix(pbr_input.material.perceptual_roughness, 0.08, soak);

    // Water fills the texture it is lying in. Where it is deep, the surface
    // light reflects off is the top of the water, not the aggregate under it,
    // so the road's own normal map has to be flattened back towards the plane.
    //
    // This is not a nicety. Dropping the roughness without doing it leaves the
    // asphalt's relief driving a near-mirror, and every grain of chipping
    // throws its own highlight: the road comes out glittering like crushed
    // glass, which is exactly what the first pass looked like. Damp asphalt
    // keeps its texture; a puddle does not have one.
    pbr_input.N = normalize(mix(pbr_input.N, pbr_input.world_normal, pool * road.wetness));

    // Rings, only where there is actually water to ring and only while it is
    // still falling. Two sets at different rates, because one is visibly a
    // single expanding pattern.
    if road.fall > 0.01 && pool > 0.01 {
        let ripple_a = sin(dot(here, vec2(5.7, 4.1)) - road.time * 9.0);
        let ripple_b = sin(dot(here, vec2(-3.9, 6.3)) - road.time * 12.7);
        let shake = (ripple_a + ripple_b) * 0.5 * road.fall * pool * 0.035;
        let tilt = vec3(shake, 0.0, shake * 0.7);
        pbr_input.N = normalize(pbr_input.N + tilt);
    }

    return pbr_input;
}

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let in = vertex_output;

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color =
        alpha_discard(pbr_input.material, pbr_input.material.base_color);

    pbr_input = wet(pbr_input);

#ifdef PREPASS_PIPELINE
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
