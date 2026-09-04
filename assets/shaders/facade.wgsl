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
// One file, two pipelines. Under `PREPASS_PIPELINE` this runs as the deferred
// fragment and writes a g-buffer; otherwise it shades forward as before. The
// grain itself is computed once, in `dress`, and neither branch has its own
// copy — which matters more than it sounds, because the mean-normalisation and
// the relief basis below are subtle enough that two copies would drift.

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

struct FacadeSettings {
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
