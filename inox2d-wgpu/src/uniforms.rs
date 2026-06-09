use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Uniforms {
	pub mvp: [[f32; 4]; 4],     // 64 bytes
	pub mult_color: [f32; 4],   // 16 bytes
	pub screen_color: [f32; 4], // 16 bytes
	pub offset: [f32; 2],       // 8 bytes
	pub emission_strength: f32, // 4 bytes
	pub alpha_threshold: f32,   // 4 bytes
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct TransformUniform {
	// 4x4 Matrix: Projection * View * Model
	// Aligned to 16 bytes for WGSL
	pub mvp: [[f32; 4]; 4],

	// Tint color (RGB) + Opacity (A)
	// Inochi calculates final alpha as: texture.a * opacity
	pub mult_color: [f32; 4],

	// Screen tint (RGB) + padding
	pub screen_color: [f32; 4],
}

// WGPU default minimum uniform buffer offset alignment is 256 bytes
pub const UNIFORM_ALIGNMENT: u64 = 256;

impl Default for Uniforms {
	fn default() -> Self {
		Self {
			mvp: glam::Mat4::IDENTITY.to_cols_array_2d(),
			mult_color: [1.0, 1.0, 1.0, 1.0],   // Default White, Full Opacity
			screen_color: [0.0, 0.0, 0.0, 0.0], // No Screen Tint
			offset: [0.0, 0.0],
			emission_strength: 1.0,
			alpha_threshold: 0.0,
		}
	}
}
