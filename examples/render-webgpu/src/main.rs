#[cfg(target_arch = "wasm32")]
fn create_window(event: &winit::event_loop::EventLoop<()>) -> Result<winit::window::Window, winit::error::OsError> {
	use winit::dpi::PhysicalSize;
	use winit::platform::web::WindowExtWebSys;
	use winit::window::WindowBuilder;

	let window = WindowBuilder::new()
		.with_resizable(false)
		.with_inner_size(PhysicalSize::new(1280, 720))
		.build(event)?;

	web_sys::window()
		.and_then(|win| win.document())
		.and_then(|doc| doc.body())
		.and_then(|body| {
			let canvas = web_sys::Element::from(window.canvas().unwrap());
			canvas.set_id("canvas");
			body.append_child(&canvas).ok()
		})
		.expect("couldn't append canvas to document body");

	Ok(window)
}

#[cfg(target_arch = "wasm32")]
fn request_animation_frame(window: &web_sys::Window, f: &wasm_bindgen::prelude::Closure<dyn FnMut()>) -> i32 {
	use wasm_bindgen::JsCast;
	window
		.request_animation_frame(f.as_ref().unchecked_ref())
		.expect("Couldn't register `requestAnimationFrame`")
}

#[cfg(target_arch = "wasm32")]
pub fn base_url() -> String {
	web_sys::window().unwrap().location().origin().unwrap()
}

#[cfg(target_arch = "wasm32")]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
	use glam::{uvec2, Vec2};
	use std::cell::RefCell;
	use std::collections::HashMap;
	use std::rc::Rc;
	use tracing::info;
	use wasm_bindgen::prelude::*;
	use wasm_bindgen::JsCast;
	use winit::event::{Event, WindowEvent};
	use winit::platform::web::EventLoopExtWebSys;

	use common::scene::ExampleSceneController;
	use inox2d::formats::inp::parse_inp;
	use inox2d::puppet::Puppet;
	use inox2d::render::InoxRendererExt;
	use inox2d_wgpu::WgpuRenderer;

	let events = winit::event_loop::EventLoop::new().unwrap();
	let window = create_window(&events)?;

	// Make sure the context has a stencil buffer
	// let context_options = js_sys::Object::new();
	// js_sys::Reflect::set(&context_options, &"stencil".into(), &true.into()).unwrap();
	info!("Creating canvas");
	let canvas = web_sys::window()
		.unwrap()
		.document()
		.unwrap()
		.get_element_by_id("canvas")
		.unwrap()
		.dyn_into::<web_sys::HtmlCanvasElement>()
		.unwrap();

	let width = canvas.client_width() as u32;
	let height = canvas.client_height() as u32;
	canvas.set_width(width);
	canvas.set_height(height);

	let res = reqwest::Client::new()
		.get(format!("{}/assets/puppet.inp", base_url()))
		.send()
		.await
		.map_err(|e| e.to_string())?;
	let model_bytes = res.bytes().await.map_err(|e| e.to_string())?;

	let mut model = parse_inp(model_bytes.as_ref()).map_err(|e| e.to_string())?;
	model.puppet.init_transforms();
	model.puppet.init_rendering();
	model.puppet.init_params();
	model.puppet.init_physics();

	let mut renderer = inox2d_wgpu::from_canvas(&canvas, &model, Some(width), Some(height))
		.await
		.map_err(|e| e.to_string())?;

	// Initial scale
	let scale = 0.15 * (height as f32 / 800.0);
	renderer.camera.scale = Vec2::splat(scale);

	let scene_ctrl = ExampleSceneController::new(&renderer.camera, 0.5);

	let renderer = Rc::new(RefCell::new(renderer));
	let puppet = Rc::new(RefCell::new(model.puppet));
	let scene_ctrl = Rc::new(RefCell::new(scene_ctrl));
	let window_ = web_sys::window().unwrap();
	let animation_loop = {
		let anim_loop_f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
		let anim_loop_g = anim_loop_f.clone();
		let puppet = puppet.clone();
		let renderer = renderer.clone();
		let scene_ctrl = scene_ctrl.clone();

		let window_loop = window_.clone();

		*anim_loop_g.borrow_mut() = Some(Closure::new(move || {
			scene_ctrl.borrow_mut().update(&mut renderer.borrow_mut().camera);

			let (mut encoder, view, output) = match renderer.borrow_mut().clear() {
				Some((encoder, view, output)) => (encoder, view, output),
				None => {
					let _ = request_animation_frame(&window_loop, anim_loop_f.borrow().as_ref().unwrap());
					return;
				}
			};

			let dt = scene_ctrl.borrow().current_elapsed();

			// Render Closure
			{
				let mut puppet = puppet.borrow_mut();
				puppet.begin_frame();
				let _ = puppet
					.param_ctx
					.as_mut()
					.unwrap()
					.set("Head:: Yaw-Pitch", Vec2::new(dt.cos(), dt.sin()));
				puppet.end_frame(dt);

				renderer.borrow_mut().on_begin_draw(&puppet);
				renderer.borrow_mut().draw(&puppet);
			}
			renderer.borrow_mut().on_end_draw(encoder, &view, output);
			let _ = request_animation_frame(&window_loop, anim_loop_f.borrow().as_ref().unwrap());
		}));
		let _ = request_animation_frame(&window_, anim_loop_g.borrow().as_ref().unwrap());
		anim_loop_g
	};

	// Event loop
	events.spawn(move |event, elwt| {
		// it needs to be present
		let _window = &window;
		elwt.set_control_flow(winit::event_loop::ControlFlow::Wait);
		match event {
			Event::WindowEvent { ref event, .. } => match event {
				WindowEvent::Resized(physical_size) => {
					// Handle window resizing
					renderer.borrow_mut().resize(physical_size.width, physical_size.height); //, &surface, &mut config, &mut depth_texture, &mut depth_view);
					canvas.set_width(physical_size.width);
					canvas.set_height(physical_size.height);
					window.request_redraw();
				}
				WindowEvent::CloseRequested => elwt.exit(),
				_ => {
					let scene_ctrl = scene_ctrl.clone();
					let renderer = renderer.clone();
					scene_ctrl
						.borrow_mut()
						.interact(event, &mut renderer.borrow_mut().camera);
				}
			},
			Event::AboutToWait => {
				window.request_redraw();
			}
			_ => (),
		}
	});

	Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn runwrap() {
	match run().await {
		Ok(_) => tracing::info!("Shutdown"),
		Err(e) => tracing::error!("Fatal crash: {}", e),
	}
}

#[cfg(target_arch = "wasm32")]
fn main() {
	console_error_panic_hook::set_once();
	tracing_wasm::set_as_global_default();
	wasm_bindgen_futures::spawn_local(runwrap());
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
	panic!("This is a WASM example. You need to build it for the WASM target.");
}
