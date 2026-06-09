use crate::vertex::Vertex;
use inox2d::node::components::BlendMode;
use std::borrow::Cow;
use std::collections::HashMap;

// Helper enum to define Masking State
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaskState {
	None,
	WriteMask,    // Drawing into the Stencil Buffer
	ReadMask(u8), // Drawing content, comparing against Stencil Ref
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
	blend_id: u8, // BlendMode -> u8 to for eq and hash traits
	mask: MaskState,
	format: wgpu::TextureFormat, // Render Target Format
	is_composite: bool,
}

pub struct PipelineManager {
	pub texture_layout: wgpu::BindGroupLayout,
	pub composite_layout: wgpu::BindGroupLayout,
	pub uniform_layout: wgpu::BindGroupLayout,
	shader: wgpu::ShaderModule,
	composite_shader: wgpu::ShaderModule,
	cache: HashMap<PipelineKey, wgpu::RenderPipeline>,
}

impl PipelineManager {
	pub fn new(device: &wgpu::Device) -> Self {
		// 1. Shader
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("Inox2D Shader"),
			source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shader.wgsl"))),
		});

		let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("Inox2D Composite Shader"),
			source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("composite.wgsl"))),
		});

		// TODO: texture layout and composite layout will be the same after adding back the missing components;
		// 		 Then we will refactor this file entirely
		// 2. Bind Group Layouts
		// Group 0: Texture (View + Sampler)
		// TODO: ENCAPSULATE FOLLOWING
		let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Inox2D Texture Layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						multisampled: false,
						view_dimension: wgpu::TextureViewDimension::D2,
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						multisampled: false,
						view_dimension: wgpu::TextureViewDimension::D2,
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 2,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						multisampled: false,
						view_dimension: wgpu::TextureViewDimension::D2,
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 3,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		});

		// Group 0: Composite Textures (Albedo, Emissive, Bump, Sampler)
		let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Inox2D Composite Layout"),
			entries: &[
				// Albedo
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						multisampled: false,
						view_dimension: wgpu::TextureViewDimension::D2,
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
					},
					count: None,
				},
				// Emissive
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						multisampled: false,
						view_dimension: wgpu::TextureViewDimension::D2,
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
					},
					count: None,
				},
				// Bump
				wgpu::BindGroupLayoutEntry {
					binding: 2,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						multisampled: false,
						view_dimension: wgpu::TextureViewDimension::D2,
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
					},
					count: None,
				},
				// Sampler
				wgpu::BindGroupLayoutEntry {
					binding: 3,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		});

		// Group 1: Uniforms
		let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Inox2D Uniform Layout"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: true,
					min_binding_size: None,
				},
				count: None,
			}],
		});

		Self {
			texture_layout,
			composite_layout,
			uniform_layout,
			shader,
			composite_shader,
			cache: HashMap::new(),
		}
	}

	pub fn get_pipeline(
		&mut self,
		device: &wgpu::Device,
		surface_format: wgpu::TextureFormat,
		blend_mode: BlendMode,
		mask: MaskState,
	) -> &wgpu::RenderPipeline {
		// Use the ID for the cache key
		let key = PipelineKey {
			blend_id: Self::get_blend_id(blend_mode),
			mask,
			format: surface_format,
			is_composite: false,
		};

		if !self.cache.contains_key(&key) {
			// Pass the original `blend` enum to creation logic
			let pipeline = self.create_pipeline(device, surface_format, blend_mode, mask);
			self.cache.insert(key, pipeline);
		}

		self.cache.get(&key).unwrap()
	}

	pub fn get_composite_pipeline(
		&mut self,
		device: &wgpu::Device,
		surface_format: wgpu::TextureFormat,
		blend_mode: BlendMode,
		mask: MaskState,
	) -> &wgpu::RenderPipeline {
		let key = PipelineKey {
			blend_id: Self::get_blend_id(blend_mode),
			mask,
			format: surface_format,
			is_composite: true,
		};

		if !self.cache.contains_key(&key) {
			let pipeline = self.create_composite_pipeline(device, surface_format, blend_mode, mask);
			self.cache.insert(key, pipeline);
		}

		self.cache.get(&key).unwrap()
	}

	fn create_pipeline(
		&self,
		device: &wgpu::Device,
		format: wgpu::TextureFormat,
		blend_mode: BlendMode,
		mask_state: MaskState,
	) -> wgpu::RenderPipeline {
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("Inox2D Pipeline Layout"),
			bind_group_layouts: &[&self.texture_layout, &self.uniform_layout],
			immediate_size: 0,
		});

		// Map Inox2D BlendMode to WGPU BlendState
		// The shader outputs Premultiplied Alpha, so we use PREMULTIPLIED settings for Normal
		let blend_state = Self::get_blend_state(blend_mode);

		// Map MaskState to DepthStencil
		let (depth_stencil, color_write) = Self::get_depth_stencil(mask_state);

		device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(&format!(
				"Inox2D Pipeline BlendMode:{:?} MaskState:{:?}",
				Self::get_blend_id(blend_mode),
				mask_state
			)),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &self.shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[Vertex::desc()],
			},
			fragment: Some(wgpu::FragmentState {
				module: &self.shader,
				entry_point: Some("fs_main"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format,
					blend: blend_state,
					write_mask: color_write,
				})],
			}),
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::TriangleList,
				strip_index_format: None,
				front_face: wgpu::FrontFace::Ccw,
				cull_mode: None,
				unclipped_depth: false,
				polygon_mode: wgpu::PolygonMode::Fill,
				conservative: false,
			},
			depth_stencil,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		})
	}

	fn create_composite_pipeline(
		&self,
		device: &wgpu::Device,
		format: wgpu::TextureFormat,
		blend_mode: BlendMode,
		mask_state: MaskState,
	) -> wgpu::RenderPipeline {
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("Inox2D Composite Pipeline Layout"),
			bind_group_layouts: &[&self.composite_layout, &self.uniform_layout],
			immediate_size: 0,
		});

		let blend_state = Self::get_blend_state(blend_mode);

		let (depth_stencil, color_write) = Self::get_depth_stencil(mask_state);

		device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(&format!(
				"Inox2D Composite Pipeline BlendMode:{:?} MaskState:{:?}",
				Self::get_blend_id(blend_mode),
				mask_state
			)),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &self.composite_shader,
				entry_point: Some("vs_composite"),
				compilation_options: Default::default(),
				buffers: &[Vertex::desc()],
			},
			fragment: Some(wgpu::FragmentState {
				module: &self.composite_shader,
				entry_point: Some("fs_composite"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format,
					blend: blend_state,
					write_mask: color_write,
				})],
			}),
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::TriangleList,
				strip_index_format: None,
				front_face: wgpu::FrontFace::Ccw,
				cull_mode: None,
				unclipped_depth: false,
				polygon_mode: wgpu::PolygonMode::Fill,
				conservative: false,
			},
			depth_stencil,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		})
	}

	// Helper to handle the missing Hash/Eq on external BlendMode
	fn get_blend_id(mode: BlendMode) -> u8 {
		match mode {
			BlendMode::Normal => 0,
			BlendMode::Multiply => 1,
			BlendMode::ColorDodge => 2,
			BlendMode::LinearDodge => 3,
			BlendMode::Screen => 4,
			BlendMode::ClipToLower => 5,
			BlendMode::SliceFromLower => 6,
		}
	}

	fn get_blend_state(blend_mode: BlendMode) -> Option<wgpu::BlendState> {
		let blend_state = match blend_mode {
			BlendMode::Normal => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
			BlendMode::Multiply => Some(wgpu::BlendState {
				color: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::Dst,
					dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
					operation: wgpu::BlendOperation::Add,
				},
				alpha: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::Dst,
					dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
					operation: wgpu::BlendOperation::Add,
				},
			}),
			BlendMode::ColorDodge => Some(wgpu::BlendState {
				color: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::Dst,
					dst_factor: wgpu::BlendFactor::One,
					operation: wgpu::BlendOperation::Add,
				},
				alpha: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::Dst,
					dst_factor: wgpu::BlendFactor::One,
					operation: wgpu::BlendOperation::Add,
				},
			}),
			BlendMode::LinearDodge => Some(wgpu::BlendState {
				color: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::One,
					dst_factor: wgpu::BlendFactor::One,
					operation: wgpu::BlendOperation::Add,
				},
				alpha: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::One,
					dst_factor: wgpu::BlendFactor::One,
					operation: wgpu::BlendOperation::Add,
				},
			}),
			BlendMode::Screen => Some(wgpu::BlendState {
				color: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::One,
					dst_factor: wgpu::BlendFactor::OneMinusSrc,
					operation: wgpu::BlendOperation::Add,
				},
				alpha: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::One,
					dst_factor: wgpu::BlendFactor::OneMinusSrc,
					operation: wgpu::BlendOperation::Add,
				},
			}),
			BlendMode::ClipToLower => Some(wgpu::BlendState {
				color: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::DstAlpha,
					dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
					operation: wgpu::BlendOperation::Add,
				},
				alpha: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::DstAlpha,
					dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
					operation: wgpu::BlendOperation::Add,
				},
			}),
			BlendMode::SliceFromLower => Some(wgpu::BlendState {
				color: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
					dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
					operation: wgpu::BlendOperation::Subtract,
				},
				alpha: wgpu::BlendComponent {
					src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
					dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
					operation: wgpu::BlendOperation::Subtract,
				},
			}),
		};

		blend_state
	}

	fn get_depth_stencil(mask: MaskState) -> (Option<wgpu::DepthStencilState>, wgpu::ColorWrites) {
		match mask {
			MaskState::None => {
				return (
					Some(wgpu::DepthStencilState {
						format: wgpu::TextureFormat::Depth24PlusStencil8,
						depth_write_enabled: false,
						depth_compare: wgpu::CompareFunction::Always,
						stencil: wgpu::StencilState {
							front: wgpu::StencilFaceState::IGNORE,
							back: wgpu::StencilFaceState::IGNORE,
							read_mask: 0,
							write_mask: 0,
						},
						bias: wgpu::DepthBiasState::default(),
					}),
					wgpu::ColorWrites::ALL,
				)
			}
			MaskState::WriteMask => {
				return (
					Some(wgpu::DepthStencilState {
						format: wgpu::TextureFormat::Depth24PlusStencil8,
						depth_write_enabled: false,
						depth_compare: wgpu::CompareFunction::Always,
						stencil: wgpu::StencilState {
							front: wgpu::StencilFaceState {
								compare: wgpu::CompareFunction::Always,
								fail_op: wgpu::StencilOperation::Keep,
								depth_fail_op: wgpu::StencilOperation::Keep,
								pass_op: wgpu::StencilOperation::Replace, // WRITE Ref
							},
							back: wgpu::StencilFaceState {
								compare: wgpu::CompareFunction::Always,
								fail_op: wgpu::StencilOperation::Keep,
								depth_fail_op: wgpu::StencilOperation::Keep,
								pass_op: wgpu::StencilOperation::Replace, // WRITE Ref
							},
							read_mask: 0xFF,
							write_mask: 0xFF,
						},
						bias: wgpu::DepthBiasState::default(),
					}),
					wgpu::ColorWrites::empty(), // Mask doesn't draw color
				);
			}
			MaskState::ReadMask(_ref_val) => {
				return (
					Some(wgpu::DepthStencilState {
						format: wgpu::TextureFormat::Depth24PlusStencil8,
						depth_write_enabled: false,
						depth_compare: wgpu::CompareFunction::Always,
						stencil: wgpu::StencilState {
							front: wgpu::StencilFaceState {
								compare: wgpu::CompareFunction::Equal, // CHECK == REF
								fail_op: wgpu::StencilOperation::Keep,
								depth_fail_op: wgpu::StencilOperation::Keep,
								pass_op: wgpu::StencilOperation::Keep,
							},
							back: wgpu::StencilFaceState {
								compare: wgpu::CompareFunction::Equal, // CHECK == REF
								fail_op: wgpu::StencilOperation::Keep,
								depth_fail_op: wgpu::StencilOperation::Keep,
								pass_op: wgpu::StencilOperation::Keep,
							},
							read_mask: 0xFF,
							write_mask: 0xFF,
						},
						bias: wgpu::DepthBiasState::default(),
					}),
					wgpu::ColorWrites::ALL,
				);
			}
		};
	}
}
