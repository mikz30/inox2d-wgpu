use crate::vertex::Vertex;
use inox2d::{puppet::Puppet, render::VertexBuffers};

/// Manages vertex and index buffers for rendering an Inochi2D puppet.
///
/// This struct handles the creation, resizing, and updating of GPU buffers
/// based on the puppet's current state.
pub struct BufferManager {
	pub vertex_buffer: wgpu::Buffer,
	pub index_buffer: wgpu::Buffer,
	pub vertex_count: u32,
	pub index_count: u32,

	// Keep track of capacity to avoid reallocating buffers every frame if not needed.
	vertex_capacity: u64,
	index_capacity: u64,

	// Staging buffers to avoid repeated allocations
	vertex_staging: Vec<Vertex>,
	index_staging: Vec<u8>,

	pub composite_index_offset: u32,
	pub composite_index_count: u32,
}

impl BufferManager {
	/// Creates a new `BufferManager` with initial default capacities.
	pub fn new(device: &wgpu::Device) -> Self {
		// Initial capacity
		let vertex_capacity = 1024 * 64;
		let index_capacity = 1024 * 16;

		let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("Inox2D Vertex Buffer"),
			size: vertex_capacity,
			usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("Inox2D Index Buffer"),
			size: index_capacity,
			usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		Self {
			vertex_buffer,
			index_buffer,
			vertex_count: 0,
			index_count: 0,
			vertex_capacity,
			index_capacity,
			vertex_staging: Vec::with_capacity(4096),
			index_staging: Vec::with_capacity(4096),
			composite_index_offset: 0,
			composite_index_count: 0,
		}
	}

	/// Updates the GPU buffers with the latest vertex and index data from the puppet.
	/// Resizes buffers if the new data exceeds current capacity.
	pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, puppet: &Puppet) {
		if let Some(render_ctx) = &puppet.render_ctx {
			let buffers = &render_ctx.vertex_buffers;

			self.update_vertices(buffers);

			let padding_len = self.update_indices(buffers);

			// Append composite indices
			let base = buffers.verts.len() as u16;
			let quad_indices = [base, base + 1, base + 2, base, base + 2, base + 3];
			self.index_staging
				.extend_from_slice(bytemuck::cast_slice(&quad_indices));

			// Update Offsets
			// Note: composite_index_offset must account for padding.
			// It is an index offset (count of u16s), not byte offset.
			// buffers.indices.len() is the raw count. padding_len is bytes (0 or 2).
			let padding_indices = padding_len / 2;
			self.composite_index_offset = (buffers.indices.len() + padding_indices) as u32;
			self.composite_index_count = 6;

			let vertices_bytes = bytemuck::cast_slice(&self.vertex_staging);
			let indices_bytes = &self.index_staging;

			let required_vertex_size = vertices_bytes.len() as u64;
			let required_index_size = indices_bytes.len() as u64;

			// Resize the buffers if needed
			if required_vertex_size > self.vertex_capacity {
				Self::resize_vertex_buffer(					
					device,
					required_vertex_size,
					&mut self.vertex_capacity,
					&mut self.vertex_buffer,
				);
			}
			if required_index_size > self.index_capacity {
				Self::resize_index_buffer(
					device,
					required_index_size,
					&mut self.index_capacity,
					&mut self.index_buffer,					
				);
			}

			// Upload data to GPU
			if !vertices_bytes.is_empty() {
				queue.write_buffer(&self.vertex_buffer, 0, vertices_bytes);
			}
			if !indices_bytes.is_empty() {
				queue.write_buffer(&self.index_buffer, 0, indices_bytes);
			}

			self.vertex_count = buffers.verts.len() as u32;
			self.index_count = buffers.indices.len() as u32;
		}
	}

	fn update_vertices(&mut self, buffers: &VertexBuffers) {
		self.vertex_staging.clear();
		let vert_len = buffers.verts.len();
		// Reserve space for mesh verts + 4 composite quad verts
		self.vertex_staging.reserve(vert_len + 4);

		// Interleave data
		for i in 0..vert_len {
			self.vertex_staging.push(Vertex {
				position: buffers.verts[i].to_array(),
				uv: buffers.uvs[i].to_array(),
				deform: buffers.deforms[i].to_array(),
			});
		}

		// Append Composite Quad (Full screen NDC -1..1)
		self.vertex_staging.push(Vertex {
			position: [-1.0, 1.0],
			uv: [0.0, 0.0],
			deform: [0.0, 0.0],
		}); // TL
		self.vertex_staging.push(Vertex {
			position: [-1.0, -1.0],
			uv: [0.0, 1.0],
			deform: [0.0, 0.0],
		}); // BL
		self.vertex_staging.push(Vertex {
			position: [1.0, -1.0],
			uv: [1.0, 1.0],
			deform: [0.0, 0.0],
		}); // BR
		self.vertex_staging.push(Vertex {
			position: [1.0, 1.0],
			uv: [1.0, 0.0],
			deform: [0.0, 0.0],
		}); // TR
	}

	fn update_indices(&mut self, buffers: &VertexBuffers) -> usize {
		self.index_staging.clear();
		let indices_raw_bytes = bytemuck::cast_slice(&buffers.indices);

		// Handle alignment for WebGPU (must be multiple of 4 bytes)
		// Indices are u16 (2 bytes), so length can be 2, 6, 10... which are not div by 4.
		// We pad with 0 (2 bytes) if necessary.
		let padding_len = if indices_raw_bytes.len() % 4 != 0 { 2 } else { 0 };

		let total_index_bytes = indices_raw_bytes.len() + padding_len + (6 * 2); // 6 indices for quad * 2 bytes
		self.index_staging.reserve(total_index_bytes);

		self.index_staging.extend_from_slice(indices_raw_bytes);
		if padding_len > 0 {
			self.index_staging.extend_from_slice(&[0, 0]);
		}

		padding_len
	}

	fn resize_vertex_buffer(
		device: &wgpu::Device,
		target_size: u64,
		vertex_capacity: &mut u64,
		vertex_buffer: &mut wgpu::Buffer,
		
	) {
		*vertex_capacity = target_size.max(*vertex_capacity * 2);
		*vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("Inox2D Vertex Buffer"),
			size: *vertex_capacity,
			usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
	}

	fn resize_index_buffer(
		device: &wgpu::Device,
		target_size: u64,
		index_capacity: &mut u64,
		index_buffer: &mut wgpu::Buffer,		
	) {
		*index_capacity = target_size.max(*index_capacity * 2);
		*index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("Inox2D Index Buffer"),
			size: *index_capacity,
			usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
	}
}
