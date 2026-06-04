use crate::uniforms::Uniforms;
use inox2d::node::components::BlendMode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaskingMode {
	NoMask,
	WriteMask(u8), // We are currently drawing into the stencil buffer (with ref value)
	ReadMask(u8),  // We are drawing content that is clipped by the stencil (with ref value)
}

/// A command captures all data required to issue a single draw call (or state change) later.
pub enum RenderCommand {
	/// Change the masking state (e.g., start writing to stencil)
	SetMaskingMode(MaskingMode),

	Draw {
		/// Texture index to bind
		texture_index: usize,

		index_offset: u32,
		index_count: u32,

		/// Pipeline configuration, opacity
		blend_mode: BlendMode,
		opacity: f32,

		/// The calculated MVP matrix and other uniforms for this specific mesh
		uniforms: Uniforms,
	},

	BeginComposite,
	EndComposite {
		index_offset: u32,
		index_count: u32,
		blend_mode: BlendMode,
		opacity: f32,
		uniforms: Uniforms,
	},
}
