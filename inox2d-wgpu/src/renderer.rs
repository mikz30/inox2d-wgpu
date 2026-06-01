use std::cell::RefCell;
use wgpu::util::DeviceExt;
use inox2d::model::Model;
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



