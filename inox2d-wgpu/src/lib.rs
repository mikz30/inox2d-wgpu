pub mod buffers;
pub mod texture;
pub mod pipeline;
pub mod uniforms;
pub mod vertex;
pub mod cmd;
pub mod error;

use std::cell::RefCell;
use wgpu::util::DeviceExt;
use inox2d::model::Model;
use inox2d::puppet::Puppet;
use inox2d::render::{InoxRenderer, TexturedMeshRenderCtx};
use inox2d::node::InoxNodeUuid;
use inox2d::node::drawables::{TexturedMeshComponents, CompositeComponents};
use inox2d::math::camera::Camera;
use inox2d::node::components::BlendMode;
use bytemuck;

use crate::error::Result;
use crate::pipeline::{PipelineManager, MaskState};
use crate::texture::TextureManager;
use crate::buffers::BufferManager;
use crate::uniforms::{Uniforms, UNIFORM_ALIGNMENT};
use crate::cmd::{RenderCommand, MaskingMode};

#[cfg(target_arch = "wasm32")]
use wgpu::web_sys;

use log::info;

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
        width: u32, 
        height: u32
    ) -> Self {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        
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

        let t_albedo = create_tex("Composite Albedo", wgpu::TextureFormat::Rgba8Unorm);
        let t_emissive = create_tex("Composite Emissive", wgpu::TextureFormat::Rgba8Unorm);
        let t_bump = create_tex("Composite Bump", wgpu::TextureFormat::Rgba8Unorm);

        let t_depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Composite Depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24PlusStencil8,
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
            label: Some("Composite Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&v_albedo) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&v_emissive) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&v_bump) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
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

    // Managers
    pub pipelines: RefCell<PipelineManager>,
    pub textures: TextureManager,
    pub buffers: BufferManager,
    pub camera: Camera,
    composite_resources: Option<CompositeResources>,

    // Uniforms
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    uniform_capacity: u64,
    uniform_staging: Vec<u8>,

    // Command Recording
    // We use RefCell to allow mutation inside the immutable InoxRenderer trait
    pub command_buffer: RefCell<Vec<RenderCommand>>,

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
}

impl WgpuRenderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        model: &Model,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
    ) -> Result<Self> {
        let textures = TextureManager::new(&device, &queue, model);
        let mut buffers = BufferManager::new(&device);
        buffers.update(&device, &queue, &model.puppet);
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
            label: Some("Inox Uniform Bind Group"),
        });

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24PlusStencil8,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());


        Ok(Self {
            device,
            queue,
            textures,
            buffers,
            camera: Camera::default(),
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
            surface_format: format,
            surface,
            surface_config,
            depth_texture,
            depth_view,
        })
    }

    /// Clear previous commands. Call this at the start of your frame loop.
    pub fn prepare(&self) {
        self.command_buffer.borrow_mut().clear();
        *self.masking_stack.borrow_mut() = vec![MaskingMode::NoMask];
        *self.current_masking_mode.borrow_mut() = MaskingMode::NoMask;
        self.mask_threshold_stack.borrow_mut().clear();
        *self.mask_counter.borrow_mut() = 0;
    }

    // 1. HELPER: Upload Uniforms
    // Call this AFTER recording commands via puppet.draw(), but BEFORE creating the RenderPass
    pub fn write_uniforms(&mut self) {
        let cmds = self.command_buffer.borrow();
        if cmds.is_empty() { return; }

        let alignment = UNIFORM_ALIGNMENT as usize;
        let uniform_size = std::mem::size_of::<Uniforms>();
        let padding = alignment - uniform_size;
        
        self.uniform_staging.clear();
        // Reserve approximate needed capacity to avoid reallocations
        self.uniform_staging.reserve(cmds.len() * alignment);

        for cmd in cmds.iter() {
            let u = match cmd {
                RenderCommand::Draw { uniforms, .. } => Some(uniforms),
                RenderCommand::EndComposite { uniforms, .. } => Some(uniforms),
                _ => None
            };

            if let Some(uniforms) = u {
                // Direct byte writing to avoid allocating small Vecs
                self.uniform_staging.extend_from_slice(bytemuck::bytes_of(uniforms));
                self.uniform_staging.extend(std::iter::repeat(0).take(padding));
            }
        }

        if self.uniform_staging.is_empty() { return; }

        let required_size = self.uniform_staging.len() as u64;

        // 2. Resize GPU buffer if needed
        if required_size > self.uniform_capacity {
            self.uniform_capacity = (required_size * 2).max(65536); // Minimum 64KB
            
            self.uniform_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Inox Dynamic Uniform Buffer"),
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
                label: Some("Inox Uniform Bind Group"),
            });
        }

        // 3. Upload
        self.queue.write_buffer(&self.uniform_buffer, 0, &self.uniform_staging);
    }

    /// The function you will call in the render loop to execute the recorded commands.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
    ) {
        let mut pipelines = self.pipelines.borrow_mut();
        let commands = self.command_buffer.borrow();

        // Ensure composite resources exist and match size
        if self.composite_resources.is_none() || 
           self.composite_resources.as_ref().unwrap().width != self.viewport_width ||
           self.composite_resources.as_ref().unwrap().height != self.viewport_height {
            self.composite_resources = Some(CompositeResources::new(
                &self.device, 
                &pipelines.composite_layout, 
                self.viewport_width, 
                self.viewport_height
            ));
        }
        let comp_res = self.composite_resources.as_ref().unwrap();

        let mut current_mask_mode = MaskingMode::NoMask;
        let mut dynamic_uniform_offset: u32 = 0;
        let mut i = 0;
        let len = commands.len();

        // Optimization: Track redundant state changes
        let mut last_pipeline_key: Option<(BlendMode, MaskState)> = None;
        let mut last_texture_index: Option<usize> = None;

        while i < len {
            // Check if we are starting a composite pass
            let is_composite_start = matches!(commands[i], RenderCommand::BeginComposite);

            if is_composite_start {
                i += 1; // Consume BeginComposite
                
                // Composite Pass resets state tracking
                last_pipeline_key = None;
                last_texture_index = None;

                // --- COMPOSITE PASS ---
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Inox Composite Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &comp_res.albedo,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
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
                
                pass.set_vertex_buffer(0, self.buffers.vertex_buffer.slice(..));
                pass.set_index_buffer(self.buffers.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                
                while i < len {
                    let cmd = &commands[i];
                    match cmd {
                        RenderCommand::EndComposite { .. } => break, // Exit to Main Pass
                        RenderCommand::BeginComposite => break, // Nested? Exit to start new pass (flattened)
                        
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
                        RenderCommand::Draw { texture_index, index_offset, index_count, blend_mode, .. } => {
                            let mask_state = match current_mask_mode {
                                MaskingMode::NoMask => MaskState::None,
                                MaskingMode::WriteMask(_) => MaskState::WriteMask,
                                MaskingMode::ReadMask(_) => MaskState::ReadMask(1),
                            };
                            
                            // Check redundancy for Pipeline
                            let current_key = (*blend_mode, mask_state);
                            if last_pipeline_key != Some(current_key) {
                                let pipeline = pipelines.get_pipeline(&self.device, wgpu::TextureFormat::Rgba8Unorm, *blend_mode, mask_state);
                                pass.set_pipeline(pipeline);
                                last_pipeline_key = Some(current_key);
                            }

                            // Check redundancy for Bind Group 0 (Texture)
                            if last_texture_index != Some(*texture_index) {
                                if let Some(bg) = self.textures.get_bind_group(*texture_index) {
                                    pass.set_bind_group(0, bg, &[]);
                                }
                                last_texture_index = Some(*texture_index);
                            }

                            pass.set_bind_group(1, &self.uniform_bind_group, &[dynamic_uniform_offset]);
                            pass.draw_indexed(*index_offset..(*index_offset + *index_count), 0, 0..1);
                            
                            dynamic_uniform_offset += UNIFORM_ALIGNMENT as u32;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            } else {
                // Main Pass resets state tracking
                last_pipeline_key = None;
                last_texture_index = None;

                // --- MAIN PASS ---
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Inox Main Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth_view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                        stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                
                pass.set_vertex_buffer(0, self.buffers.vertex_buffer.slice(..));
                pass.set_index_buffer(self.buffers.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

                while i < len {
                    let cmd = &commands[i];
                    match cmd {
                        RenderCommand::BeginComposite => break, // Switch to Composite Pass
                        
                        RenderCommand::EndComposite { vertex_offset: _, index_offset, index_count, blend_mode, .. } => {
                            // Draw Composite Result
                            let pipeline = pipelines.get_composite_pipeline(&self.device, self.surface_format, *blend_mode, MaskState::None);
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

                        RenderCommand::Draw { texture_index, index_offset, index_count, blend_mode, .. } => {
                            let mask_state = match current_mask_mode {
                                MaskingMode::NoMask => MaskState::None,
                                MaskingMode::WriteMask(_) => MaskState::WriteMask,
                                MaskingMode::ReadMask(_) => MaskState::ReadMask(1),
                            };
                            
                            // Check Redundancy: Pipeline
                            let current_key = (*blend_mode, mask_state);
                            if last_pipeline_key != Some(current_key) {
                                let pipeline = pipelines.get_pipeline(&self.device, self.surface_format, *blend_mode, mask_state);
                                pass.set_pipeline(pipeline);
                                last_pipeline_key = Some(current_key);
                            }

                            // Check Redundancy: Texture Bind Group
                            if last_texture_index != Some(*texture_index) {
                                if let Some(bg) = self.textures.get_bind_group(*texture_index) {
                                    pass.set_bind_group(0, bg, &[]);
                                }
                                last_texture_index = Some(*texture_index);
                            }

                            pass.set_bind_group(1, &self.uniform_bind_group, &[dynamic_uniform_offset]);
                            pass.draw_indexed(*index_offset..(*index_offset + *index_count), 0, 0..1);
                            
                            dynamic_uniform_offset += UNIFORM_ALIGNMENT as u32;
                            i += 1;
                        }
                        _ => i += 1,
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
                label: Some("Depth Texture"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let new_depth_view = new_depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

            self.depth_texture = new_depth_texture;
            self.depth_view = new_depth_view;           
        }
    }

    pub fn clear(&self) -> Option<(wgpu::CommandEncoder, wgpu::TextureView, wgpu::SurfaceTexture)> {
        let current_texture = self.surface.get_current_texture();
        let output = match current_texture {
            Ok(output) => output,
            Err(_) => return None,
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") });

        // Clear pass (color and depth/stencil)
        {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        Some((encoder, view, output))
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
        self.command_buffer.borrow_mut().push(RenderCommand::SetMaskingMode(mode));
        self.masking_stack.borrow_mut().push(mode);
        *self.current_masking_mode.borrow_mut() = mode;
        self.mask_threshold_stack.borrow_mut().push(masks.threshold);
    }

    fn on_end_mask(&self) {
        // This is called after a mask is drawn. The next thing to be drawn will be the masked content.
        let mut stack = self.masking_stack.borrow_mut();
        stack.pop();
        if let Some(mode) = stack.last() {
            self.command_buffer.borrow_mut().push(RenderCommand::SetMaskingMode(*mode));
            *self.current_masking_mode.borrow_mut() = *mode;
        }
    }

    fn on_begin_masked_content(&self) {
        // Now we draw the actual sprites that get clipped by the mask we just drew.
        let mask_id = *self.mask_counter.borrow();
        let mode = MaskingMode::ReadMask(mask_id);
        self.command_buffer.borrow_mut().push(RenderCommand::SetMaskingMode(mode));
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
        self.command_buffer.borrow_mut().push(RenderCommand::BeginComposite);
    }

    fn finish_composite_content(&self, _as_mask: bool, components: &CompositeComponents, _render_ctx: &inox2d::render::CompositeRenderCtx, _id: InoxNodeUuid) {
        let mut stack = self.masking_stack.borrow_mut();
        stack.pop();
        if let Some(mode) = stack.last() {
            self.command_buffer.borrow_mut().push(RenderCommand::SetMaskingMode(*mode));
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

        self.command_buffer.borrow_mut().push(RenderCommand::EndComposite {
            vertex_offset: self.buffers.composite_vertex_offset,
            index_offset: self.buffers.composite_index_offset,
            index_count: self.buffers.composite_index_count,
            blend_mode: components.drawable.blending.mode,
            opacity,
            uniforms,
        });
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
        let v_off = render_ctx.vert_offset as i32;
        let i_off = render_ctx.index_offset as u32;
        let i_cnt = render_ctx.index_len as u32;

        if i_cnt == 0 { return; }

        // 2. Calculate Uniforms
        let mut projection = self.camera.matrix(glam::Vec2::new(self.viewport_width as f32, self.viewport_height as f32));

        // WGPU Correction Matrix
        let correction = glam::Mat4::from_cols_array_2d(&[
            [1.0,  0.0, 0.0, 0.0],
            [0.0,  1.0, 0.0, 0.0], 
            [0.0,  0.0, 0.5, 0.0], // Scale Z (GL -1..1 -> WGPU 0..1)
            [0.0,  0.0, 0.5, 1.0], // Translate Z
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
        self.command_buffer.borrow_mut().push(RenderCommand::Draw {
            texture_index,
            vertex_offset: v_off,
            index_offset: i_off,
            index_count: i_cnt,
            blend_mode: components.drawable.blending.mode,
            opacity,
            uniforms,
        });
    }

    fn on_begin_mask(&self, _mask: &inox2d::node::components::Mask) { }

}

impl WgpuRenderer {
    pub fn on_begin_draw(&mut self, puppet: &Puppet) {
        self.buffers.update(&self.device, &self.queue, puppet);
        self.prepare();
    }

    pub fn on_end_draw(&mut self, mut encoder: wgpu::CommandEncoder, view: &wgpu::TextureView, output: wgpu::SurfaceTexture) {
        self.write_uniforms();
        self.render(&mut encoder, view);
    
        let _ = &self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }
}

#[cfg(target_arch = "wasm32")]
pub async fn from_canvas(canvas: &web_sys::HtmlCanvasElement, 
    model: &Model,
    width: Option<u32>, 
    height: Option<u32>
) -> Result<WgpuRenderer> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    let width = width.unwrap_or(canvas.client_width() as u32);
    let height = height.unwrap_or(canvas.client_height() as u32);

    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone())).map_err(|e| e.to_string())?;
    
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }).await.map_err(|e| e.to_string())?;

    info!("Adapter limits: {:?}", adapter.limits());

    let (device, queue) = adapter.request_device(
        &wgpu::DeviceDescriptor {
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            label: None,
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        }
    ).await.map_err(|e| e.to_string())?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
    
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

    let renderer = WgpuRenderer::new(
        device,
        queue,
        model,
        format,
        width,
        height,
        surface,
        config,
    )?;
    Ok(renderer)
    
}

pub fn from_winit_window(window: &winit::window::Window, model: &Model) -> Result<WgpuRenderer> {
    todo!()
}