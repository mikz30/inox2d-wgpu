#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use tracing::field::display;
use web_sys::{Element, HtmlElement};
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[derive(Clone)]
pub struct Profiler {
	start: Instant,
	timings: Vec<(String, f32)>,
	// body: HtmlElement,
	// log_div: Element,
	cpu_time_element: Element,
	// fps_element: Element,
	last_time: Instant,
	frame_count: u32,
	accumulated_cpu_time: f64,
}

// Display: Time to First Frame / CPU time per frame / Frames per Second
impl Profiler {
	pub fn new() -> Profiler {
		// just panic if anything wrong happens
		let window = web_sys::window().unwrap();
		let document = window.document().unwrap();
		let body = document.body().unwrap();
		let display_element = document.create_element("div").unwrap();
		display_element.set_attribute("id", "bench-log").unwrap();
		display_element
			.set_attribute(
				"style",
				"position: fixed; top: 0; left: 0; z-index: 9999; color: white; white-space: pre;",
			)
			.unwrap();
		let cpu_time_element = document.create_element("a").unwrap();
		cpu_time_element.set_attribute("id", "cpu_time").unwrap();
		let fps_element = document.create_element("a").unwrap();
		fps_element.set_attribute("id", "fps").unwrap();
		display_element.append_child(&cpu_time_element).unwrap();
		display_element.append_child(&fps_element).unwrap();
		body.append_child(&display_element).unwrap();

		Self {
			start: Instant::now(),
			timings: Vec::new(),
			cpu_time_element,
			last_time: Instant::now(),
			frame_count: 0,
			accumulated_cpu_time: 0.0,
		}
	}

	pub fn add_timing(&mut self, label: &str) {
		self.timings.push((label.into(), self.start.elapsed().as_secs_f32()));
	}

	pub fn display(&mut self) -> Result<(), wasm_bindgen::JsValue> {
		let window = web_sys::window().unwrap();
		let document = window.document().unwrap();
		let body = document.body().unwrap();
		let display_element = document.create_element("div")?;
		display_element.set_attribute("id", "bench-log")?;
		display_element.set_attribute("style", "position: fixed; top: 0; left: 0; z-index: 9999; color: white")?;

		self.add_timing("Set inner html begins");
		display_element.set_inner_html("Dummy text");
		self.add_timing("Set inner html ends");
		body.append_child(&display_element)?;
		self.add_timing("Append child ends");

		self.add_timing("Add timing starts");
		self.add_timing("dummy timing");
		self.add_timing("Add timing ends");
		let mut s = String::from("");
		for (label, elapsed) in &self.timings {
			s.push_str(format!("{}: {}s <br>", label, elapsed).as_str());
		}
		display_element.set_inner_html(s.as_str());
		body.append_child(&display_element)?;

		Ok(())
	}

	pub fn update_profiler_element(&mut self, cpu_time: f64) {
		self.frame_count += 1;
		self.accumulated_cpu_time += cpu_time;

		if self.frame_count >= 60 {
			let elapsed = self.last_time.elapsed().as_secs_f64();
			let fps = self.frame_count as f64 / elapsed;
			let avg_cpu_time = self.accumulated_cpu_time / self.frame_count as f64;
			self.cpu_time_element.set_text_content(Some(
				format!("CPU time per frame: {:.3}ms\nFPS: {:.1}", avg_cpu_time * 1000.0, fps).as_str(),
			));
			self.last_time = Instant::now();
			self.frame_count = 0;
			self.accumulated_cpu_time = 0.0;
		}
	}
}

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
	use glam::Vec2;
	use std::cell::RefCell;
	use std::rc::Rc;
	use wasm_bindgen::prelude::*;
	use wasm_bindgen::JsCast;
	use winit::event::{Event, WindowEvent};
	use winit::platform::web::EventLoopExtWebSys;

	use common::scene::ExampleSceneController;
	use inox2d::formats::inp::parse_inp;
	use inox2d::model::Model;
	use inox2d::puppet::Puppet;
	use inox2d::render::InoxRendererExt;

	let mut profiler = Profiler::new();

	let events = winit::event_loop::EventLoop::new().unwrap();
	let window = create_window(&events)?;

	// Make sure the context has a stencil buffer
	// let context_options = js_sys::Object::new();
	// js_sys::Reflect::set(&context_options, &"stencil".into(), &true.into()).unwrap();
	// info!("Creating canvas");
	profiler.add_timing("Create Canvas Start");
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

	profiler.add_timing("Load Model Start");
	let res = reqwest::Client::new()
		.get(format!("{}/assets/puppet.inp", base_url()))
		.send()
		.await
		.map_err(|e| e.to_string())?;
	let model_bytes = res.bytes().await.map_err(|e| e.to_string())?;

	profiler.add_timing("Parsing And Initializing Model Start");
	let mut models = Vec::new();
	for _ in 0..6 {
		let mut model = parse_inp(model_bytes.as_ref()).map_err(|e| e.to_string())?;
		model.puppet.init_transforms();
		model.puppet.init_rendering();
		model.puppet.init_params();
		model.puppet.init_physics();

		profiler.add_timing("Initializing Renderer Start");

		models.push(model);
	}
	let models_ref: Vec<&Model> = models.iter().collect();

	let mut renderer = inox2d_wgpu::from_canvas(&canvas, &models_ref, Some(width), Some(height))
		.await
		.map_err(|e| e.to_string())?;

	// Initial scale
	let scale = 0.15 * (height as f32 / 800.0);
	for camera in renderer.cameras.values_mut() {
		camera.scale = Vec2::splat(scale);
	}
	// This example only controls the leftmost puppet
	let scene_ctrl = ExampleSceneController::new(&renderer.cameras.get(&0).unwrap(), 0.5);

	let renderer = Rc::new(RefCell::new(renderer));
	let mut puppets = Vec::new();
	for model in models {
		puppets.push(Rc::new(RefCell::new(model.puppet)));
	}
	let scene_ctrl = Rc::new(RefCell::new(scene_ctrl));
	let window_ = web_sys::window().unwrap();
	let __animation_loop = {
		profiler.add_timing("Animation Loop Start");
		let anim_loop_f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
		let anim_loop_g = anim_loop_f.clone();
		let puppets = puppets.clone();
		let renderer = renderer.clone();
		let scene_ctrl = scene_ctrl.clone();
		let mut profiler = profiler.clone();
		let window_loop = window_.clone();

		*anim_loop_g.borrow_mut() = Some(Closure::new(move || {
			let (encoder, view, output, cpu_timer) = match renderer.borrow_mut().clear() {
				(Some((encoder, view, output)), cpu_timer) => (encoder, view, output, cpu_timer),
				(None, cpu_timer) => {
					profiler.update_profiler_element(cpu_timer.elapsed().as_secs_f64());
					let _ = request_animation_frame(&window_loop, anim_loop_f.borrow().as_ref().unwrap());
					return;
				}
			};
			scene_ctrl
				.borrow_mut()
				.update(&mut renderer.borrow_mut().cameras.get_mut(&0).unwrap());

			let dt = scene_ctrl.borrow().current_elapsed();

			// Render Closure
			for (puppet_id, puppet) in puppets.iter().enumerate() {
				let mut puppet = puppet.borrow_mut();
				puppet.begin_frame();
				let _ = puppet
					.param_ctx
					.as_mut()
					.unwrap()
					.set("Head:: Yaw-Pitch", Vec2::new(dt.cos(), dt.sin()));
				puppet.end_frame(dt);

				renderer.borrow_mut().on_begin_draw(&puppet, puppet_id);
				renderer.borrow_mut().draw(&puppet);
			}
			let cpu_time = renderer.borrow_mut().on_end_draw(encoder, &view, output, cpu_timer);
			profiler.update_profiler_element(cpu_time);
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
						.interact(event, &mut renderer.borrow_mut().cameras.get(&0).unwrap());
				}
			},
			Event::AboutToWait => {
				window.request_redraw();
			}
			_ => (),
		}
	});

	// profiler.display().unwrap();

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
