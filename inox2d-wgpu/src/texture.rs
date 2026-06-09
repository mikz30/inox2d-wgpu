use inox2d::model::Model;
use inox2d::texture::decode_model_textures;

/// Represents a single texture loaded onto the GPU, including its view, sampler, and bind group.
pub struct Texture {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	pub sampler: wgpu::Sampler,
	pub bind_group: wgpu::BindGroup,
}

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// Manages the lifecycle and binding of textures for an Inochi2D model.
pub struct TextureManager {
	pub textures: Vec<Texture>,
	pub bind_group_layout: wgpu::BindGroupLayout,
}

impl TextureManager {
	/// Creates a new `TextureManager` by loading and uploading all textures from the provided model.
	///
	/// This function decodes the texture data, uploads it to the GPU, creates texture views and samplers,
	/// and initializes the bind groups required for rendering.
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, model: &Model) -> Self {
		let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Inox2D Texture Bind Group Layout"),
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

		let shallow_textures = decode_model_textures(model.textures.iter());
		let mut textures = Vec::with_capacity(shallow_textures.len());

		for (i, shallow_tex) in shallow_textures.iter().enumerate() {
			let width = shallow_tex.width();
			let height = shallow_tex.height();
			let size = wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			};

			// Used when texture doesn't have emissive or bump
			let fallback_size = wgpu::Extent3d {
				width: 1,
				height: 1,
				depth_or_array_layers: 1
			};

			// Create the texture resource on the GPU.
			let tex_albedo = create_texture(&device, &format!("Inox2D Texture #{} Albedo", i), size);
			let tex_emissive = create_texture(&device, &format!("Inox2D Texture #{} Emissive", i), fallback_size); // temporary
			let tex_bump = create_texture(&device, &format!("Inox2D Texture #{} Bump", i), fallback_size); // temporary
			
			// Upload texture data (Add emissive and bump after supported).
			queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &tex_albedo,
					mip_level: 0,
					origin: wgpu::Origin3d::ZERO,
					aspect: wgpu::TextureAspect::All,
				},
				shallow_tex.pixels(),
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(4 * width),
					rows_per_image: Some(height),
				},
				size,
			);

			let view_albedo = tex_albedo.create_view(&wgpu::TextureViewDescriptor::default());
			let view_emissive = tex_emissive.create_view(&wgpu::TextureViewDescriptor::default());
			let view_bump = tex_bump.create_view(&wgpu::TextureViewDescriptor::default());

			// Create a sampler with linear filtering and clamp-to-edge wrapping.
			let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
				label: Some(&format!("Inox2D Sampler for Texture #{}", i)),
				address_mode_u: wgpu::AddressMode::ClampToEdge,
				address_mode_v: wgpu::AddressMode::ClampToEdge,
				address_mode_w: wgpu::AddressMode::ClampToEdge,
				mag_filter: wgpu::FilterMode::Linear,
				min_filter: wgpu::FilterMode::Linear,
				mipmap_filter: wgpu::MipmapFilterMode::Linear,
				..Default::default()
			});

			// Create a bind group for this specific texture.
			let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some(&format!("Inox2D Bind Group for Texture #{}", i)),
				layout: &bind_group_layout,
				entries: &[
					wgpu::BindGroupEntry {
						binding: 0,
						resource: wgpu::BindingResource::TextureView(&view_albedo),
					},
					wgpu::BindGroupEntry {
						binding: 1,
						resource: wgpu::BindingResource::TextureView(&view_emissive),
					},
					wgpu::BindGroupEntry {
						binding: 2,
						resource: wgpu::BindingResource::TextureView(&view_bump),
					},
					wgpu::BindGroupEntry {
						binding: 3,
						resource: wgpu::BindingResource::Sampler(&sampler),
					},
				],
			});

			textures.push(Texture {
				texture: tex_albedo,
				view: view_albedo,
				sampler,
				bind_group,
			});
		}

		Self {
			textures,
			bind_group_layout,
		}
	}

	/// Retrieves the bind group associated with a specific texture ID.
	pub fn get_bind_group(&self, index: usize) -> Option<&wgpu::BindGroup> {
		self.textures.get(index).map(|t| &t.bind_group)
	}
}

fn create_texture(device: &wgpu::Device, label: &str, size: wgpu::Extent3d) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some(label),
		size,
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8Unorm,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	})
}
