pub mod buffers;
pub mod texture;
pub mod pipeline;
pub mod uniforms;
pub mod vertex;
pub mod renderer;
pub mod cmd;
pub mod error;

mod tests;

// pub struct WgpuRenderer {
//     // Resources
//     device: wgpu::Device,
//     queue: wgpu::Queue,

//     // Internal State
//     pipelines: PipelinesManager,
//     textures: TextureManager,
//     buffers: BufferManager,

//     // Command Recorder (Internal Mutability need for InoxRenderer trait)
//     command_buffer: std::cell::RefCell<Vec<DrawCommand>>,

//     // Configuration
//     format: wgpu::TextureFormat,
// }

// impl WgpuRenderer {
//     /// Initialize resources, compile shaders, upload static textures
//     pub fn new(device: wgpu::Device, 
//         queue: wgpu::Queue, 
//         model: &Model,
//         format: wgpu::TextureFormat
//     ) -> Result<Self, WgpuRendererError>;

//     /// Update internal buffers (geometry) if the puppet has changed
//     /// Shoud be called before encoding the frame
//     pub fn update_buffers(&mut self, puppet: &Puppet);

//     /// Execute the rendering
//     /// 1. Calls puppet.draw(self) to record commands via InoxRenderer trait
//     /// 2. Encodes commands into the provided RenderPass
//     pub fn render<'rpass>(
//         &'rpass self,
//         rpass: &mut wgpu::RenderPass<'rpass>,
//         puppet: &Puppet
//     );
// }