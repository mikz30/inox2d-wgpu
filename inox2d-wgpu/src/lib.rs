pub mod buffers;
pub mod cmd;
pub mod error;
pub mod pipeline;
pub mod texture;
pub mod uniforms;
pub mod vertex;

use bytemuck;
use inox2d::math::camera::Camera;
use inox2d::model::Model;
use inox2d::node::components::BlendMode;
use inox2d::node::drawables::{CompositeComponents, TexturedMeshComponents};
use inox2d::node::InoxNodeUuid;
use inox2d::puppet::Puppet;
use inox2d::render::{InoxRenderer, TexturedMeshRenderCtx};
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::buffers::BufferManager;
use crate::cmd::{MaskingMode, RenderCommand};
use crate::error::Result;
use crate::pipeline::PipelineManager;
use crate::texture::{TextureManager, DEPTH_FORMAT};
use crate::uniforms::{Uniforms, UNIFORM_ALIGNMENT};

#[cfg(target_arch = "wasm32")]
use wgpu::web_sys;

pub struct CompositeResources {
	pub albedo: wgpu::TextureView,
	pub emissive: wgpu::TextureView,
	pub bump: wgpu::TextureView,
	pub depth: wgpu::TextureView,
	pub bind_group: wgpu::BindGroup,
	pub width: u32,
	pub height: u32,
}

impl CompositeResources {
	pub fn new(
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
		format: wgpu::TextureFormat,
		width: u32,
		height: u32,
	) -> Self {
		let size = wgpu::Extent3d {
			width,
			height,
			depth_or_array_layers: 1,
		};

		let create_tex = |label: &str, format: wgpu::TextureFormat| {
			device.create_texture(&wgpu::TextureDescriptor {
				label: Some(label),
				size,
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			})
		};

		let t_albedo = create_tex("Inox2D Composite Albedo", format); //wgpu::TextureFormat::Rgba8Unorm);
		let t_emissive = create_tex("Inox2D Composite Emissive", format); //wgpu::TextureFormat::Rgba8Unorm);
		let t_bump = create_tex("Inox2D Composite Bump", format); //wgpu::TextureFormat::Rgba8Unorm);

		let t_depth = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("Inox2D Composite Depth"),
			size,
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: DEPTH_FORMAT,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});

		let v_albedo = t_albedo.create_view(&Default::default());
		let v_emissive = t_emissive.create_view(&Default::default());
		let v_bump = t_bump.create_view(&Default::default());
		let v_depth = t_depth.create_view(&Default::default());

		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			..Default::default()
		});

		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("Inox2D Composite Bind Group"),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&v_albedo),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(&v_emissive),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(&v_bump),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::Sampler(&sampler),
				},
			],
		});

		Self {
			albedo: v_albedo,
			emissive: v_emissive,
			bump: v_bump,
			depth: v_depth,
			bind_group,
			width,
			height,
		}
	}
}

pub struct WgpuRenderer {
	pub device: wgpu::Device,
	pub queue: wgpu::Queue,
	pub model_index: usize, // TODO: temporary, needs something like user id
	pub pipelines: RefCell<PipelineManager>,
	pub textures: HashMap<usize, TextureManager>,
	pub buffers: HashMap<usize, BufferManager>,
	pub cameras: HashMap<usize, Camera>,
	composite_resources: Option<CompositeResources>,

	uniform_buffer: wgpu::Buffer,
	uniform_bind_group: wgpu::BindGroup,
	uniform_capacity: u64,
	uniform_staging: Vec<u8>,

	// Command Recording
	pub command_buffer: RefCell<Vec<(usize, RenderCommand)>>,

	// State tracking during recording
	masking_stack: RefCell<Vec<MaskingMode>>,
	current_masking_mode: RefCell<MaskingMode>,
	mask_threshold_stack: RefCell<Vec<f32>>,

	// Counter to ensure each mask gets a unique stencil value (1..255)
	mask_counter: RefCell<u8>,

	// Viewport size (needed for projection matrix calculation)
	pub viewport_width: u32,
	pub viewport_height: u32,

	// The surface format is needed for pipeline creation
	surface_format: wgpu::TextureFormat,

	surface: wgpu::Surface<'static>,
	surface_config: wgpu::SurfaceConfiguration,

	depth_view: wgpu::TextureView,
	depth_texture: wgpu::Texture,

	current_puppet_id: Option<usize>,
}

impl WgpuRenderer {
	pub fn new(
		device: wgpu::Device,
		queue: wgpu::Queue,
		surface_format: wgpu::TextureFormat,
		width: u32,
		height: u32,
		surface: wgpu::Surface<'static>,
		surface_config: wgpu::SurfaceConfiguration,
	) -> Result<Self> {
		let textures = HashMap::new();
		let buffers = HashMap::new();

		let pipelines = PipelineManager::new(&device);
		let uniform_capacity = UNIFORM_ALIGNMENT * 256; // Start with space for 256 sprites
		let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("Inox Dynamic Uniform Buffer"),
			size: uniform_capacity,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			layout: &pipelines.uniform_layout,
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
					buffer: &uniform_buffer,
					offset: 0,
					size: std::num::NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64),
				}),
			}],
			label: Some("Inox2D Uniform Bind Group"),
		});

		let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("Inox2D Depth Texture"),
			size: wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: DEPTH_FORMAT,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});
		let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

		Ok(Self {
			device,
			queue,
			model_index: 0,
			textures,
			buffers,
			cameras: HashMap::new(),
			composite_resources: None,
			pipelines: RefCell::new(pipelines),
			uniform_buffer,
			uniform_bind_group,
			uniform_capacity,
			uniform_staging: Vec::with_capacity((uniform_capacity as usize) * 2),
			command_buffer: RefCell::new(Vec::with_capacity(256)),
			masking_stack: RefCell::new(vec![MaskingMode::NoMask]),
			current_masking_mode: RefCell::new(MaskingMode::NoMask),
			mask_threshold_stack: RefCell::new(Vec::with_capacity(4)),
			mask_counter: RefCell::new(0),
			viewport_width: width,
			viewport_height: height,
			surface_format,
			surface,
			surface_config,
			depth_texture,
			depth_view,
			current_puppet_id: None,
		})
	}

	pub fn init(&mut self, models: &Vec<&Model>) {
		for model in models {
			let _ = self.add_model(model, 0);
		}
	}

	pub fn add_model(&mut self, model: &Model, _key: usize) -> usize {
		let texture = TextureManager::new(&self.device, &self.queue, model);
		let mut buffer = BufferManager::new(&self.device);
		buffer.init(&self.device, &self.queue, &model.puppet);
		let mut camera = Camera::default();

		let index = self.model_index;
		// hardcode camera position to space them out for now
		camera.position.x = (index as f32 - 2.5) * 2000.0;

		self.textures.insert(index, texture);
		self.buffers.insert(index, buffer);
		self.cameras.insert(index, camera);

		self.model_index += 1;
		index
	}

	/// Clear previous commands. Call this at the start of your frame loop.
	pub fn prepare(&self) {
		self.command_buffer.borrow_mut().clear();
		*self.masking_stack.borrow_mut() = vec![MaskingMode::NoMask];
		*self.current_masking_mode.borrow_mut() = MaskingMode::NoMask;
		self.mask_threshold_stack.borrow_mut().clear();
		*self.mask_counter.borrow_mut() = 0;
	}

	// Call this AFTER recording commands via puppet.draw(), but BEFORE creating the RenderPass
	fn write_uniforms(&mut self) {
		let cmds = self.command_buffer.borrow();
		if cmds.is_empty() {
			return;
		}

		let alignment = UNIFORM_ALIGNMENT as usize;
		let uniform_size = std::mem::size_of::<Uniforms>();
		let padding = alignment - uniform_size;

		self.uniform_staging.clear();
		// Reserve approximate needed capacity to avoid reallocations
		self.uniform_staging.reserve(cmds.len() * alignment);

		for cmd in cmds.iter() {
			let u = match cmd {
				(_, RenderCommand::Draw { uniforms, .. }) => Some(uniforms),
				(_, RenderCommand::EndComposite { uniforms, .. }) => Some(uniforms),
				_ => None,
			};

			if let Some(uniforms) = u {
				// Direct byte writing to avoid allocating small Vecs
				self.uniform_staging.extend_from_slice(bytemuck::bytes_of(uniforms));
				self.uniform_staging.extend(std::iter::repeat(0).take(padding));
			}
		}

		if self.uniform_staging.is_empty() {
			return;
		}

		let required_size = self.uniform_staging.len() as u64;

		// 2. Resize GPU buffer if needed
		if required_size > self.uniform_capacity {
			self.uniform_capacity = (required_size * 2).max(65536); // Minimum 64KB

			self.uniform_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
				label: Some("Inox2D Dynamic Uniform Buffer"),
				size: self.uniform_capacity,
				usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
				mapped_at_creation: false,
			});

			// Recreate Bind Group since Buffer ID changed
			self.uniform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
				layout: &self.pipelines.borrow().uniform_layout,
				entries: &[wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
						buffer: &self.uniform_buffer,
						offset: 0,
						size: std::num::NonZeroU64::new(uniform_size as u64),
					}),
				}],
				label: Some("Inox2D Uniform Bind Group"),
			});
		}

		// 3. Upload
		self.queue.write_buffer(&self.uniform_buffer, 0, &self.uniform_staging);
	}

	/// The function you will call in the render loop to execute the recorded commands.
	fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target_view: &wgpu::TextureView) {
		let mut pipelines = self.pipelines.borrow_mut();
		let commands = self.command_buffer.borrow();

		// Ensure composite resources exist and match size
		if self.composite_resources.is_none()
			|| self.composite_resources.as_ref().unwrap().width != self.viewport_width
			|| self.composite_resources.as_ref().unwrap().height != self.viewport_height
		{
			self.composite_resources = Some(CompositeResources::new(
				&self.device,
				&pipelines.composite_layout,
				self.surface_format,
				self.viewport_width,
				self.viewport_height,
			));
		}
		let comp_res = self.composite_resources.as_ref().unwrap();

		let mut current_mask_mode = MaskingMode::NoMask;
		let mut dynamic_uniform_offset: u32 = 0;
		let mut i = 0;
		let len = commands.len();

		// Optimization: Track redundant state changes
		let mut last_pipeline_key: Option<(BlendMode, MaskingMode)>;
		let mut last_texture_index: Option<usize>;

		while i < len {
			// Check if we are starting a composite pass
			let is_composite_start = matches!(commands[i].1, RenderCommand::BeginComposite);
			if is_composite_start {
				i += 1; // Consume BeginComposite

				// Composite Pass resets state tracking
				last_pipeline_key = None;
				last_texture_index = None;

				// --- COMPOSITE PASS ---
				let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
					label: Some("Inox2D Composite Pass"),
					color_attachments: &[Some(wgpu::RenderPassColorAttachment {
						view: &comp_res.albedo,
						resolve_target: None,
						ops: wgpu::Operations {
							load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
							store: wgpu::StoreOp::Store,
						},
						depth_slice: None,
					})],
					depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
						view: &comp_res.depth,
						depth_ops: Some(wgpu::Operations {
							load: wgpu::LoadOp::Clear(1.0),
							store: wgpu::StoreOp::Store,
						}),
						stencil_ops: Some(wgpu::Operations {
							load: wgpu::LoadOp::Clear(0),
							store: wgpu::StoreOp::Store,
						}),
					}),
					timestamp_writes: None,
					occlusion_query_set: None,
					multiview_mask: None,
				});
				// need to get the puppet id from the command
				let puppet_id = commands[i].0;
				let buffer = self
					.buffers
					.get(&puppet_id)
					.expect(format!("{}:{} No buffer found for puppet id {}", file!(), line!(), puppet_id).as_str());
				pass.set_vertex_buffer(0, buffer.vertex_static_buffer.slice(..));
				pass.set_vertex_buffer(1, buffer.vertex_deform_buffer.slice(..));
				pass.set_index_buffer(buffer.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

				while i < len {
					let cmd = &commands[i].1; // TODO: Getting puppet id here can be redundant. Im just lazy right now to think through the logic.
					let puppet_id = commands[i].0;
					match cmd {
						RenderCommand::EndComposite { .. } => break, // Exit to Main Pass
						RenderCommand::BeginComposite => break, // Nested? Inox2d currently doesn't support nested composites (expect composite stack and more complicated optimization)

						RenderCommand::SetMaskingMode(mode) => {
							current_mask_mode = *mode;
							let ref_val = match mode {
								MaskingMode::WriteMask(v) => *v as u32,
								MaskingMode::ReadMask(v) => *v as u32,
								MaskingMode::NoMask => 0,
							};
							pass.set_stencil_reference(ref_val);
							i += 1;
						}
						RenderCommand::Draw {
							texture_index,
							index_offset,
							index_count,
							blend_mode,
							..
						} => {
							self.set_active_pipeline(
								&mut pass,
								&mut pipelines,
								&mut last_pipeline_key,
								self.surface_format,
								*blend_mode,
								current_mask_mode,
							);

							self.set_texture_bind_group(&mut pass, puppet_id, &mut last_texture_index, texture_index);
							self.set_uniform_bind_group(&mut pass, dynamic_uniform_offset);

							pass.draw_indexed(*index_offset..(*index_offset + *index_count), 0, 0..1);

							dynamic_uniform_offset += UNIFORM_ALIGNMENT as u32;
							i += 1;
						}
					}
				}
			} else {
				// Main Pass resets state tracking
				last_pipeline_key = None;
				last_texture_index = None;

				// --- MAIN PASS ---
				let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
					label: Some("Inox2D Main Pass"),
					color_attachments: &[Some(wgpu::RenderPassColorAttachment {
						view: target_view,
						resolve_target: None,
						ops: wgpu::Operations {
							load: wgpu::LoadOp::Load,
							store: wgpu::StoreOp::Store,
						},
						depth_slice: None,
					})],
					depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
						view: &self.depth_view,
						depth_ops: Some(wgpu::Operations {
							load: wgpu::LoadOp::Load,
							store: wgpu::StoreOp::Store,
						}),
						stencil_ops: Some(wgpu::Operations {
							load: wgpu::LoadOp::Load,
							store: wgpu::StoreOp::Store,
						}),
					}),
					timestamp_writes: None,
					occlusion_query_set: None,
					multiview_mask: None,
				});

				let puppet_id = commands[i].0;
				let buffer = self
					.buffers
					.get(&puppet_id)
					.expect(format!("{}:{} No buffer found for puppet id {}", file!(), line!(), puppet_id).as_str());

				pass.set_vertex_buffer(0, buffer.vertex_static_buffer.slice(..));
				pass.set_vertex_buffer(1, buffer.vertex_deform_buffer.slice(..));
				pass.set_index_buffer(buffer.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

				while i < len {
					let cmd = &commands[i].1;
					let puppet_id = commands[i].0;
					match cmd {
						RenderCommand::BeginComposite => break, // Switch to Composite Pass

						RenderCommand::EndComposite {
							index_offset,
							index_count,
							blend_mode,
							..
						} => {
							// Draw Composite Result
							let pipeline = pipelines.get_composite_pipeline(
								&self.device,
								self.surface_format,
								*blend_mode,
								MaskingMode::NoMask,
							);
							pass.set_pipeline(pipeline);

							// Composite binds use Group 0, clobbering texture state
							pass.set_bind_group(0, &comp_res.bind_group, &[]);
							pass.set_bind_group(1, &self.uniform_bind_group, &[dynamic_uniform_offset]);
							pass.draw_indexed(*index_offset..(*index_offset + *index_count), 0, 0..1);

							dynamic_uniform_offset += UNIFORM_ALIGNMENT as u32;

							// Invalidate trackers because pipeline and bindgroup 0 changed
							last_pipeline_key = None;
							last_texture_index = None;
							i += 1;
						}

						RenderCommand::SetMaskingMode(mode) => {
							current_mask_mode = *mode;
							let ref_val = match mode {
								MaskingMode::WriteMask(v) => *v as u32,
								MaskingMode::ReadMask(v) => *v as u32,
								MaskingMode::NoMask => 0,
							};
							pass.set_stencil_reference(ref_val);
							i += 1;
						}

						RenderCommand::Draw {
							texture_index,
							index_offset,
							index_count,
							blend_mode,
							..
						} => {
							self.set_active_pipeline(
								&mut pass,
								&mut pipelines,
								&mut last_pipeline_key,
								self.surface_format,
								*blend_mode,
								current_mask_mode,
							);

							self.set_texture_bind_group(&mut pass, puppet_id, &mut last_texture_index, texture_index);
							self.set_uniform_bind_group(&mut pass, dynamic_uniform_offset);

							pass.draw_indexed(*index_offset..(*index_offset + *index_count), 0, 0..1);

							dynamic_uniform_offset += UNIFORM_ALIGNMENT as u32;
							i += 1;
						}
					}
				}
			}
		}
	}

	pub fn resize(&mut self, width: u32, height: u32) {
		if width > 0 && height > 0 {
			self.surface_config.height = height;
			self.surface_config.width = width;
			self.surface.configure(&self.device, &self.surface_config);

			self.viewport_width = width;
			self.viewport_height = height;

			let new_depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("Inox2D Depth Texture"),
				size: wgpu::Extent3d {
					width,
					height,
					depth_or_array_layers: 1,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: DEPTH_FORMAT,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
				view_formats: &[],
			});
			let new_depth_view = new_depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

			self.depth_texture = new_depth_texture;
			self.depth_view = new_depth_view;
		}
	}

	pub fn clear(
		&self,
	) -> (
		Option<(wgpu::CommandEncoder, wgpu::TextureView, wgpu::SurfaceTexture)>,
		Instant,
	) {
		let current_texture = self.surface.get_current_texture();
		let cpu_timer = Instant::now();
		let output = match current_texture {
			Ok(output) => output,
			Err(_) => return (None, cpu_timer),
		};
		let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
		let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("Inox2D Render Encoder"),
		});

		// Clear pass (color and depth/stencil)
		{
			let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("Inox2D Clear Pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					resolve_target: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
						store: wgpu::StoreOp::Store,
					},
					depth_slice: None,
				})],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &self.depth_view,
					depth_ops: Some(wgpu::Operations {
						load: wgpu::LoadOp::Clear(1.0),
						store: wgpu::StoreOp::Store,
					}),
					stencil_ops: Some(wgpu::Operations {
						load: wgpu::LoadOp::Clear(0),
						store: wgpu::StoreOp::Store,
					}),
				}),
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
		}
		self.prepare();
		(Some((encoder, view, output)), cpu_timer)
	}

	fn set_active_pipeline(
		&self,
		pass: &mut wgpu::RenderPass,
		pipelines: &mut std::cell::RefMut<'_, PipelineManager>,
		last_pipeline_key: &mut Option<(BlendMode, MaskingMode)>,
		surface_format: wgpu::TextureFormat,
		blend_mode: BlendMode,
		mask_state: MaskingMode,
	) {
		let current_key = (blend_mode, mask_state);
		if *last_pipeline_key != Some(current_key) {
			let pipeline = pipelines.get_pipeline(&self.device, surface_format, blend_mode, mask_state);
			pass.set_pipeline(pipeline);
			*last_pipeline_key = Some(current_key);
		}
	}

	fn set_texture_bind_group(
		&self,
		pass: &mut wgpu::RenderPass,
		puppet_id: usize,
		last_texture_index: &mut Option<usize>,
		texture_index: &usize,
	) {
		if *last_texture_index != Some(*texture_index) {
			let texture = self.textures.get(&puppet_id).expect(
				format!(
					"{}:{} Cannot find texture manager for puppet id {}",
					file!(),
					line!(),
					puppet_id
				)
				.as_str(),
			);
			if let Some(bg) = texture.get_bind_group(*texture_index) {
				pass.set_bind_group(0, bg, &[]);
			}
			*last_texture_index = Some(*texture_index);
		}
	}

	fn set_uniform_bind_group(&self, pass: &mut wgpu::RenderPass, offset: u32) {
		pass.set_bind_group(1, &self.uniform_bind_group, &[offset]);
	}
}

impl InoxRenderer for WgpuRenderer {
	// -----------------------------------------------------------------
	// Masking Logic
	// -----------------------------------------------------------------

	fn on_begin_masks(&self, masks: &inox2d::node::components::Masks) {
		// Inochi2D is about to draw shapes that should act as a mask.
		// We push a command to switch pipeline state to "Write to Stencil".

		// Increment mask counter to get a unique ID for this mask group
		let mut counter = self.mask_counter.borrow_mut();
		*counter = counter.wrapping_add(1);
		let mask_id = *counter;

		let mode = MaskingMode::WriteMask(mask_id);
		self.command_buffer
			.borrow_mut()
			.push((self.current_puppet_id.unwrap(), RenderCommand::SetMaskingMode(mode)));
		self.masking_stack.borrow_mut().push(mode);
		*self.current_masking_mode.borrow_mut() = mode;
		self.mask_threshold_stack.borrow_mut().push(masks.threshold);
	}

	fn on_end_mask(&self) {
		// This is called after a mask is drawn. The next thing to be drawn will be the masked content.
		let mut stack = self.masking_stack.borrow_mut();
		stack.pop();
		if let Some(mode) = stack.last() {
			self.command_buffer
				.borrow_mut()
				.push((self.current_puppet_id.unwrap(), RenderCommand::SetMaskingMode(*mode)));
			*self.current_masking_mode.borrow_mut() = *mode;
		}
	}

	fn on_begin_masked_content(&self) {
		// Now we draw the actual sprites that get clipped by the mask we just drew.
		let mask_id = *self.mask_counter.borrow();
		let mode = MaskingMode::ReadMask(mask_id);
		self.command_buffer
			.borrow_mut()
			.push((self.current_puppet_id.unwrap(), RenderCommand::SetMaskingMode(mode)));
		*self.current_masking_mode.borrow_mut() = mode;

		// Update the stack top to reflect the phase change (Write -> Read)
		let mut stack = self.masking_stack.borrow_mut();
		if let Some(last) = stack.last_mut() {
			*last = mode;
		}

		// We are done writing the mask, so we pop the threshold that was used for it.
		self.mask_threshold_stack.borrow_mut().pop();
	}

	fn begin_composite_content(
		&self,
		_as_mask: bool,
		_components: &CompositeComponents,
		_render_ctx: &inox2d::render::CompositeRenderCtx,
		_id: InoxNodeUuid,
	) {
		// Push the current mode to the stack to preserve it across the composite group
		let mode = *self.current_masking_mode.borrow();
		self.masking_stack.borrow_mut().push(mode);
		self.command_buffer
			.borrow_mut()
			.push((self.current_puppet_id.unwrap(), RenderCommand::BeginComposite));
	}

	fn finish_composite_content(
		&self,
		_as_mask: bool,
		components: &CompositeComponents,
		_render_ctx: &inox2d::render::CompositeRenderCtx,
		_id: InoxNodeUuid,
	) {
		let mut stack = self.masking_stack.borrow_mut();
		stack.pop();
		if let Some(mode) = stack.last() {
			self.command_buffer
				.borrow_mut()
				.push((self.current_puppet_id.unwrap(), RenderCommand::SetMaskingMode(*mode)));
			*self.current_masking_mode.borrow_mut() = *mode;
		}

		let tint = components.drawable.blending.tint;
		let opacity = components.drawable.blending.opacity;
		let screen_tint = components.drawable.blending.screen_tint;
		let uniforms = Uniforms {
			mvp: glam::Mat4::IDENTITY.to_cols_array_2d(),
			mult_color: [tint[0], tint[1], tint[2], opacity],
			screen_color: [screen_tint[0], screen_tint[1], screen_tint[2], 0.0],
			..Default::default()
		};

		let puppet_id = self.current_puppet_id.unwrap();
		let buffer = self.buffers.get(&puppet_id).expect(
			format!(
				"{}:{} Cannot find buffers for puppet id {}",
				file!(),
				line!(),
				puppet_id
			)
			.as_str(),
		);

		self.command_buffer.borrow_mut().push((
			puppet_id,
			RenderCommand::EndComposite {
				index_offset: buffer.composite_index_offset,
				index_count: buffer.composite_index_count,
				blend_mode: components.drawable.blending.mode,
				opacity,
				uniforms,
			},
		));
	}

	// -----------------------------------------------------------------
	// Drawing Logic
	// -----------------------------------------------------------------

	fn draw_textured_mesh_content(
		&self,
		as_mask: bool,
		components: &TexturedMeshComponents,
		render_ctx: &TexturedMeshRenderCtx,
		_id: InoxNodeUuid, // why unused?
	) {
		// 1. Get Geometry Range
		let index_offset = render_ctx.index_offset as u32;
		let index_count = render_ctx.index_len as u32;

		if index_count == 0 {
			return;
		}

		let puppet_id = self.current_puppet_id.unwrap();
		let camera = self
			.cameras
			.get(&puppet_id)
			.expect(format!("{}:{} Cannot find camera for puppet id {}", file!(), line!(), puppet_id).as_str());

		// 2. Calculate Uniforms
		let mut projection = camera.matrix(glam::Vec2::new(self.viewport_width as f32, self.viewport_height as f32));

		// WGPU Correction Matrix
		let correction = glam::Mat4::from_cols_array_2d(&[
			[1.0, 0.0, 0.0, 0.0],
			[0.0, 1.0, 0.0, 0.0],
			[0.0, 0.0, 0.5, 0.0], // Scale Z (GL -1..1 -> WGPU 0..1)
			[0.0, 0.0, 0.5, 1.0], // Translate Z
		]);

		projection = correction * projection;

		// Convert transform to local glam::Mat4 to avoid version mismatch errors
		let transform = glam::Mat4::from_cols_array(&components.transform.to_cols_array());
		let mvp = projection * transform;
		let tint = components.drawable.blending.tint;

		let opacity = components.drawable.blending.opacity;

		let alpha_threshold = if as_mask {
			*self.mask_threshold_stack.borrow().last().unwrap_or(&0.0)
		} else {
			0.0
		};

		let uniforms = Uniforms {
			mvp: mvp.to_cols_array_2d(),
			mult_color: [tint[0], tint[1], tint[2], opacity],
			alpha_threshold,
			..Default::default()
		};

		// 3. Identify Texture
		let texture_index = components.texture.tex_albedo.raw();

		// 4. Push Command
		self.command_buffer.borrow_mut().push((
			self.current_puppet_id.unwrap(),
			RenderCommand::Draw {
				texture_index,
				index_offset,
				index_count,
				blend_mode: components.drawable.blending.mode,
				opacity,
				uniforms,
			},
		));
	}

	fn on_begin_mask(&self, _mask: &inox2d::node::components::Mask) {}
}

impl WgpuRenderer {
	// Note: per puppet
	pub fn on_begin_draw(&mut self, puppet: &Puppet, puppet_id: usize) {
		let buffer_manager = self
			.buffers
			.get_mut(&puppet_id)
			.expect(format!("{}: {} Invalid puppet id: {}", file!(), line!(), puppet_id).as_str());
		buffer_manager.update(&self.device, &self.queue, puppet);
		self.current_puppet_id = Some(puppet_id);
		// self.prepare();
	}

	pub fn on_end_draw(
		&mut self,
		mut encoder: wgpu::CommandEncoder,
		view: &wgpu::TextureView,
		output: wgpu::SurfaceTexture,
		cpu_timer: Instant,
	) -> f64 {
		self.write_uniforms();
		self.render(&mut encoder, view);
		let cpu_time = cpu_timer.elapsed().as_secs_f64();

		let _ = &self.queue.submit(std::iter::once(encoder.finish()));
		output.present();

		cpu_time
	}
}

#[cfg(target_arch = "wasm32")]
pub async fn from_canvas(
	canvas: &web_sys::HtmlCanvasElement,
	models: &Vec<&Model>,
	width: Option<u32>,
	height: Option<u32>,
) -> Result<WgpuRenderer> {
	use log::info;
	let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
		backends: wgpu::Backends::all(),
		..Default::default()
	});

	let width = width.unwrap_or(canvas.client_width() as u32);
	let height = height.unwrap_or(canvas.client_height() as u32);

	let surface = instance
		.create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
		.map_err(|e| e.to_string())?;

	let adapter = instance
		.request_adapter(&wgpu::RequestAdapterOptions {
			power_preference: wgpu::PowerPreference::default(),
			compatible_surface: Some(&surface),
			force_fallback_adapter: false,
		})
		.await
		.map_err(|e| e.to_string())?;

	info!("Adapter limits: {:?}", adapter.limits());

	let (device, queue) = adapter
		.request_device(&wgpu::DeviceDescriptor {
			required_features: wgpu::Features::empty(),
			required_limits: adapter.limits(),
			label: None,
			memory_hints: wgpu::MemoryHints::Performance,
			..Default::default()
		})
		.await
		.map_err(|e| e.to_string())?;

	let caps = surface.get_capabilities(&adapter);
	let format = caps
		.formats
		.iter()
		.copied()
		.find(|f| !f.is_srgb())
		.unwrap_or(caps.formats[0]);

	let config = wgpu::SurfaceConfiguration {
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
		format,
		width,
		height,
		present_mode: wgpu::PresentMode::Fifo,
		alpha_mode: wgpu::CompositeAlphaMode::Auto,
		view_formats: vec![],
		desired_maximum_frame_latency: 2,
	};

	surface.configure(&device, &config);

	let mut renderer = WgpuRenderer::new(device, queue, format, width, height, surface, config)?;
	renderer.init(models);
	Ok(renderer)
}

pub fn from_winit_window(_window: &winit::window::Window, _model: &Model) -> Result<WgpuRenderer> {
	todo!()
}
