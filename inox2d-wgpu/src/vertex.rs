use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
	pub position: [f32; 2], // @location(0)
	pub uv: [f32; 2],       // @location(1)
	pub deform: [f32; 2],   // @location(2)
}

impl Vertex {
	pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
		use memoffset;
		wgpu::VertexBufferLayout {
			array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &[
				// Location 0: Position
				wgpu::VertexAttribute {
					offset: memoffset::offset_of!(Vertex, position) as wgpu::BufferAddress,
					shader_location: 0,
					format: wgpu::VertexFormat::Float32x2,
				},
				// Location 1: UV
				wgpu::VertexAttribute {
					offset: memoffset::offset_of!(Vertex, uv) as wgpu::BufferAddress,
					shader_location: 1,
					format: wgpu::VertexFormat::Float32x2,
				},
				// Location 2: Deform
				wgpu::VertexAttribute {
					offset: memoffset::offset_of!(Vertex, deform) as wgpu::BufferAddress,
					shader_location: 2,
					format: wgpu::VertexFormat::Float32x2,
				},
			],
		}
	}
}
