struct Globals {
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) local: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) kind: f32,
    @location(5) color: vec4<f32>,
    @location(6) grad_a: vec4<f32>,
    @location(7) grad_b: vec4<f32>,
    @location(8) p0: vec2<f32>,
    @location(9) p1: vec2<f32>,
    @location(10) radial_radius: f32,
    @location(11) uv: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) radius: f32,
    @location(3) kind: f32,
    @location(4) color: vec4<f32>,
    @location(5) grad_a: vec4<f32>,
    @location(6) grad_b: vec4<f32>,
    @location(7) p0: vec2<f32>,
    @location(8) p1: vec2<f32>,
    @location(9) radial_radius: f32,
    @location(10) uv: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let n = in.position / globals.size;
    out.position = vec4<f32>(n.x * 2.0 - 1.0, 1.0 - n.y * 2.0, 0.0, 1.0);
    out.local = in.local;
    out.half_size = in.half_size;
    out.radius = in.radius;
    out.kind = in.kind;
    out.color = in.color;
    out.grad_a = in.grad_a;
    out.grad_b = in.grad_b;
    out.p0 = in.p0;
    out.p1 = in.p1;
    out.radial_radius = in.radial_radius;
    out.uv = in.uv;
    return out;
}

// Signed distance to a rounded box centred at the origin.
fn rounded_box_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Rects use the SDF for coverage; paths and textured quads are already
    // bounded by their geometry.
    var coverage = 1.0;
    if in.radius >= 0.0 {
        let d = rounded_box_sdf(in.local, in.half_size, max(in.radius, 0.0));
        coverage = 1.0 - smoothstep(-0.5, 0.5, d);
    }

    var color = in.color;
    let kind = i32(in.kind + 0.5);

    // Sample unconditionally: `textureSample` computes derivatives and WGSL
    // requires uniform control flow, which a per-kind branch does not provide.
    // `textureSampleLevel` takes an explicit LOD and is legal in divergent
    // control flow, so the textured path below is well-defined.
    let sampled = textureSampleLevel(atlas, atlas_sampler, in.uv.xy, 0.0);

    if kind == 1 {
        // Linear gradient from p0 to p1.
        let dir = in.p1 - in.p0;
        let denom = max(dot(dir, dir), 0.0001);
        let t = clamp(dot(in.local + in.half_size - in.p0, dir) / denom, 0.0, 1.0);
        color = mix(in.grad_a, in.grad_b, t);
    } else if kind == 2 {
        // Radial gradient centred on p0.
        let d = length(in.local + in.half_size - in.p0) / max(in.radial_radius, 0.0001);
        color = mix(in.grad_a, in.grad_b, clamp(d, 0.0, 1.0));
    } else if kind == 3 {
        // Glyph masks are white with coverage in alpha; images are
        // premultiplied RGBA. In both cases tinting is a multiply.
        color = sampled * in.color;
    }

    return vec4<f32>(color.rgb, color.a * coverage);
}
