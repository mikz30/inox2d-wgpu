struct VertexInput {
    @location(0) verts: vec2<f32>,
    @location(1) uvs:   vec2<f32>,
    @location(2) deform: vec2<f32>, 
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct Uniforms {
    mvp: mat4x4<f32>,     
    mult_color: vec4<f32>,    // RGB = Tint, A = Opacity
    screen_color: vec4<f32>,  // RGB = Screen Color
    offset: vec2<f32>,
    emission_strength: f32,
    alpha_threshold: f32,
};

@group(0) @binding(0) var t_albedo: texture_2d<f32>;
@group(0) @binding(1) var s_sampler: sampler;

// --- Composite Bindings (Group 0 for Composite Pipeline) ---
@group(0) @binding(0) var t_comp_albedo: texture_2d<f32>;
@group(0) @binding(1) var t_comp_emissive: texture_2d<f32>;
@group(0) @binding(2) var t_comp_bump: texture_2d<f32>;
@group(0) @binding(3) var s_comp_sampler: sampler;

@group(1) @binding(0) var<uniform> data: Uniforms;

// Optional: Add Emissive/Bump bindings here if we support them later

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Logic: (Base - Offset + Deform)
    // Note: WGPU Z is 0.0, W is 1.0
    let world_pos = in.verts - data.offset + in.deform;
    
    out.clip_position = data.mvp * vec4<f32>(world_pos, 0.0, 1.0);
    out.uv = in.uvs;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_albedo, s_sampler, in.uv);

    if (tex_color.a < data.alpha_threshold) {
        discard;
    }

    // 1. Screen Color Math (Replicating GLSL exactly)
    // Formula: 1.0 - ((1.0 - tex) * (1.0 - (screen * tex.a)))
    let screen_factor = data.screen_color.rgb * tex_color.a;
    let screen_out = vec3<f32>(1.0) - (
        (vec3<f32>(1.0) - tex_color.rgb) * 
        (vec3<f32>(1.0) - screen_factor)
    );

    // 2. Multiply Color Math
    // GLSL: vec4(screenOut, texColor.a) * vec4(multColor, 1)
    let mult_rgb = screen_out * data.mult_color.rgb;
    let final_alpha = tex_color.a; // Note: Inochi GLSL doesn't multiply alpha by mult_color.a here, only at the end

    // 3. Opacity Application (Premultiplication)
    // GLSL: ... * opacity
    let opacity = data.mult_color.a;
    
    // Final Result
    return vec4<f32>(mult_rgb * opacity, final_alpha * opacity);
}

@vertex
fn vs_composite(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // Pass straight through, standard MVP
    out.clip_position = data.mvp * vec4<f32>(in.verts, 0.0, 1.0);
    out.uv = in.uvs;
    return out;
}

@fragment
fn fs_composite(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_comp_albedo, s_comp_sampler, in.uv);
    let emissive = textureSample(t_comp_emissive, s_comp_sampler, in.uv);
    let bump = textureSample(t_comp_bump, s_comp_sampler, in.uv);

    // Screen Color Math (Same as standard)
    let screen_factor = data.screen_color.rgb * tex_color.a;
    let screen_out = vec3<f32>(1.0) - (
        (vec3<f32>(1.0) - tex_color.rgb) * 
        (vec3<f32>(1.0) - screen_factor)
    );

    // Multiply Color Math
    let mult_rgb = screen_out * data.mult_color.rgb;
    
    // Apply Opacity
    let opacity = data.mult_color.a;
    let final_albedo = vec4<f32>(mult_rgb * opacity, tex_color.a * opacity);

    // Combine emissive for visual result
    let result = final_albedo.rgb + emissive.rgb * final_albedo.a;
    
    return vec4<f32>(result, final_albedo.a);
}