mod crop;
mod dynlib;
mod egl;
mod gfx;
mod gl;
mod input;
mod renderer;
mod wayland;
mod wayland_clipboard;
mod wayland_cursor;
mod wayland_egl;
mod wayland_input;
mod wayland_overlay;
mod wayland_screencopy;
mod xkbcommon;

use std::{
    ffi::{CStr, c_char, c_void},
    ptr::{NonNull, null_mut},
    rc::Rc,
};

use anyhow::{Context as _, anyhow};
use crop::Crop;
use gfx::{DrawBuffer, Rect, RectFill, Size, Vec2};
use input::{Event, KeyboardEventKind, Scancode, SerialType};
use renderer::Renderer;

struct Libs {
    wayland: &'static wayland::Lib,
    wayland_egl: &'static wayland_egl::Lib,
    wayland_cursor: &'static wayland_cursor::Lib,
    wayland_display: NonNull<wayland::wl_display>,
    egl: &'static egl::Lib,
    egl_context: Rc<egl::Context>,
    gl: &'static gl::Lib,
    xkbcommon: &'static xkbcommon::Lib,
}

#[derive(Default)]
struct Globals {
    compositor: Option<*mut wayland::wl_compositor>,
    data_device_manager: Option<*mut wayland::wl_data_device_manager>,
    outputs: Vec<*mut wayland::wl_output>,
    seat: Option<*mut wayland::wl_seat>,
    shm: Option<*mut wayland::wl_shm>,
    fractional_scale_manager: Option<*mut wayland::wp_fractional_scale_manager_v1>,
    viewporter: Option<*mut wayland::wp_viewporter>,
    layer_shell: Option<*mut wayland::zwlr_layer_shell_v1>,
    screencopy_manager: Option<*mut wayland::zwlr_screencopy_manager_v1>,
    linux_dmabuf: Option<*mut wayland::zwp_linux_dmabuf_v1>,
}

struct Connection {
    libs: Libs,
    globals: Globals,
}

struct Screen {
    conn: Rc<Connection>,
    output: NonNull<wayland::wl_output>,

    screencopy: Option<Box<wayland_screencopy::Screencopy>>,
    overlay: Option<Box<wayland_overlay::Overlay>>,

    crop: Crop,
}

struct ScreenDrawOpts {
    draw_crop_decorations: bool,
    swap_buffers: bool,
}

impl Default for ScreenDrawOpts {
    fn default() -> Self {
        Self {
            draw_crop_decorations: true,
            swap_buffers: true,
        }
    }
}

impl Screen {
    fn draw(
        &self,
        draw_buffer: &mut DrawBuffer,
        renderer: &Renderer,
        opts: &ScreenDrawOpts,
    ) -> anyhow::Result<()> {
        let screencopy = self.screencopy.as_ref().unwrap().as_ref();
        let overlay = self.overlay.as_ref().unwrap();

        let fractional_scale = overlay.fractional_scale.unwrap_or(1.0);
        let logical_size = overlay.logical_size.unwrap();
        let view_rect = Rect::new(Vec2::ZERO, logical_size.as_vec2());

        let window_surface = overlay.window_surface.as_ref().unwrap();
        let dmabuf = screencopy.dmabuf.as_ref().unwrap();

        unsafe {
            self.conn
                .libs
                .egl_context
                .make_current(window_surface.handle)?;

            self.conn.libs.gl.ClearColor(0.0, 0.0, 0.0, 0.0);
            self.conn.libs.gl.Clear(gl::sys::COLOR_BUFFER_BIT);
        }

        draw_buffer.clear();

        draw_buffer.push_rect_filled(view_rect, RectFill::TextureHandle(dmabuf.gl_texture.handle));

        if opts.draw_crop_decorations {
            if self.crop.crop_rect.is_some() {
                self.crop.draw(draw_buffer);
            } else {
                draw_buffer.push_rect_filled(view_rect, RectFill::Color(crop::theme::OUTSIDE_BG));
            }
        }

        unsafe {
            renderer.draw(logical_size, fractional_scale, draw_buffer);
        }

        if opts.swap_buffers {
            unsafe {
                self.conn
                    .libs
                    .egl_context
                    .swap_buffers(window_surface.handle)?;
            }
        }

        Ok(())
    }
}

struct App {
    input: Box<wayland_input::Input>,
    clipboard: Box<wayland_clipboard::Clipboard>,
    draw_buffer: DrawBuffer,
    renderer: Renderer,
    screens: Vec<Screen>,
    conn: Rc<Connection>,

    quit_requested: bool,
    copy_requested: bool,
}

impl App {
    fn init_all_screens(&mut self) -> anyhow::Result<()> {
        assert!(self.screens.is_empty());
        self.screens.reserve_exact(self.conn.globals.outputs.len());
        for output in self.conn.globals.outputs.iter() {
            self.screens.push(Screen {
                conn: Rc::clone(&self.conn),
                output: NonNull::new(*output).context("whoopsie, output is null")?,

                screencopy: None,
                overlay: None,

                crop: Crop::default(),
            });
        }
        Ok(())
    }

    fn capture_all_screens(&mut self) -> anyhow::Result<()> {
        for screen in self.screens.iter_mut() {
            let screencopy = screen.screencopy.get_or_insert_with(|| {
                wayland_screencopy::Screencopy::new_boxed(&self.conn, screen.output)
            });
            unsafe { screencopy.capture()? };
        }

        loop {
            let mut pending: usize = 0;
            for (idx, screen) in self.screens.iter().enumerate() {
                use wayland_screencopy::ScreencopyState::*;
                match screen.screencopy.as_ref().unwrap().state {
                    Pending => pending += 1,
                    Ready => {}
                    Failed => return Err(anyhow!("failed to capture screen #{idx}")),
                }
            }
            if pending == 0 {
                break;
            }

            unsafe {
                (self.conn.libs.wayland.wl_display_dispatch)(
                    self.conn.libs.wayland_display.as_ptr(),
                )
            };
        }

        Ok(())
    }

    fn overlay_all_screens(&mut self) -> anyhow::Result<()> {
        for screen in self.screens.iter_mut() {
            assert!(screen.overlay.is_none());
            screen.overlay = Some(wayland_overlay::Overlay::new_boxed(
                &self.conn,
                screen.output,
            )?);
        }

        loop {
            let mut pending: usize = 0;
            for screen in self.screens.iter() {
                pending += !screen.overlay.as_ref().unwrap().acked_first_configure as usize;
            }
            if pending == 0 {
                break;
            }

            unsafe {
                (self.conn.libs.wayland.wl_display_dispatch)(
                    self.conn.libs.wayland_display.as_ptr(),
                )
            };
        }

        let screen_draw_opts = ScreenDrawOpts::default();
        for screen in self.screens.iter() {
            screen.draw(&mut self.draw_buffer, &self.renderer, &screen_draw_opts)?;
            // TODO: request frame
        }

        Ok(())
    }

    fn update(&mut self) -> anyhow::Result<()> {
        while let Some(event) = self.input.events.pop_front() {
            match event {
                Event::Keyboard(ref keyboard_event) => match keyboard_event.kind {
                    KeyboardEventKind::Press {
                        scancode: Scancode::Esc,
                    } => self.quit_requested = true,
                    KeyboardEventKind::Press {
                        scancode: Scancode::C,
                    } if keyboard_event.mods.ctrl => {
                        self.handle_copy_request()?;
                        return Ok(());
                    }
                    _ => {}
                },
                _ => {}
            }

            for i in 0..self.screens.len() {
                let screen = &mut self.screens[i];
                let overlay = screen.overlay.as_ref().unwrap();

                let screen_surface_id = wayland_input::get_surface_id(overlay.surface);
                let event_surface_id = match event {
                    Event::Keyboard(ref kev) => kev.surface_id,
                    Event::Pointer(ref pev) => pev.surface_id,
                };

                if screen_surface_id != event_surface_id {
                    continue;
                }

                let logical_size = overlay.logical_size.unwrap();
                let view_rect = Rect::new(Vec2::ZERO, logical_size.as_vec2());
                let crop_updated = screen.crop.update(view_rect, &event);

                if let Some(cursor_shape) = screen.crop.cursor {
                    self.input.set_cursor_shape(cursor_shape)?;
                }

                if crop_updated {
                    // remove crops from other screens
                    for j in 0..self.screens.len() {
                        if i == j {
                            continue;
                        }
                        let other_screen = &mut self.screens[j];
                        other_screen.crop.crop_rect = None;
                    }
                }
            }
        }

        Ok(())
    }

    fn draw(&mut self, screen_draw_opts: ScreenDrawOpts) -> anyhow::Result<()> {
        for screen in self.screens.iter() {
            screen.draw(&mut self.draw_buffer, &self.renderer, &screen_draw_opts)?;
        }
        Ok(())
    }

    fn handle_copy_request(&mut self) -> anyhow::Result<()> {
        let Some(screen_idx) = self
            .screens
            .iter()
            .enumerate()
            .find(|(_, screen)| screen.crop.crop_rect.is_some())
            .map(|(idx, _)| idx)
        else {
            return Ok(());
        };

        // hide all overlays
        for screen in self.screens.iter_mut() {
            let overlay = screen.overlay.as_mut().unwrap();
            let layer_surface = overlay.layer_surface.take().unwrap();
            unsafe {
                wayland::zwlr_layer_surface_v1_destroy(
                    self.conn.libs.wayland,
                    layer_surface.as_ptr(),
                );
            }
        }

        // read pixels
        let (pixels, size) = {
            let screen = &self.screens[screen_idx];
            let overlay = screen.overlay.as_ref().unwrap();

            screen.draw(&mut self.draw_buffer, &self.renderer, &ScreenDrawOpts {
                draw_crop_decorations: false,
                swap_buffers: false,
            })?;

            let fractional_scale = overlay.fractional_scale.unwrap_or(1.0) as f32;
            let crop_rect = screen.crop.crop_rect.unwrap() * fractional_scale;
            let view_rect = screen.crop.view_rect.unwrap() * fractional_scale;
            assert!(view_rect.min.eq(&Vec2::ZERO));

            let crop_size = Size::new(crop_rect.width() as u32, crop_rect.height() as u32);
            let view_size = Size::new(view_rect.width() as u32, view_rect.height() as u32);

            let pixels = unsafe { gl::read_pixels(self.conn.libs.gl, crop_rect, view_size) };

            (pixels, crop_size)
        };

        // destroy all overlays
        self.screens.clear();

        // TODO: encode pixels to png
        let mut data: Vec<u8> = Vec::new();
        let mut encoder = png::Encoder::new(&mut data, size.width, size.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder
            .write_header()
            .context("could not write png header")?
            .write_image_data(&pixels)
            .context("could not write png data")?;

        let serial = self
            .input
            .serial_tracker
            .get_serial(SerialType::KeyboardEnter)
            .context("no pointer enter serial found")?;
        self.clipboard
            .offer_data(serial, "image/png".to_string(), data)?;

        Ok(())
    }
}

unsafe extern "C" fn handle_registry_global(
    data: *mut c_void,
    wl_registry: *mut wayland::wl_registry,
    name: u32,
    interface: *const c_char,
    version: u32,
) {
    let conn = &mut *(data as *mut Connection);

    let interface = CStr::from_ptr(interface)
        .to_str()
        .expect("invalid interface string");

    macro_rules! bind_assign {
        ($field:ident, $interface:ident) => {{
            conn.globals.$field = Some(wayland::wl_registry_bind(
                conn.libs.wayland,
                wl_registry,
                name,
                &wayland::$interface,
                version,
            ) as _);
            assert!(conn.globals.$field.is_some_and(|field| !field.is_null()));
            log::info!("bound {interface}");
        }};
    }

    match interface {
        "wl_compositor" => bind_assign!(compositor, wl_compositor_interface),
        "wl_data_device_manager" => {
            bind_assign!(data_device_manager, wl_data_device_manager_interface)
        }
        "wl_output" => {
            conn.globals.outputs.push(wayland::wl_registry_bind(
                conn.libs.wayland,
                wl_registry,
                name,
                &wayland::wl_output_interface,
                version,
            ) as _);
        }
        "wl_seat" => bind_assign!(seat, wl_seat_interface),
        "wl_shm" => bind_assign!(shm, wl_shm_interface),
        "wp_fractional_scale_manager_v1" => bind_assign!(
            fractional_scale_manager,
            wp_fractional_scale_manager_v1_interface
        ),
        "wp_viewporter" => bind_assign!(viewporter, wp_viewporter_interface),
        "zwlr_layer_shell_v1" => bind_assign!(layer_shell, zwlr_layer_shell_v1_interface),
        "zwlr_screencopy_manager_v1" => {
            bind_assign!(screencopy_manager, zwlr_screencopy_manager_v1_interface)
        }
        "zwp_linux_dmabuf_v1" => bind_assign!(linux_dmabuf, zwp_linux_dmabuf_v1_interface),
        _ => {
            log::debug!("unused interface: {interface}");
        }
    }
}

const WL_REGISTRY_LISTENER: wayland::wl_registry_listener = wayland::wl_registry_listener {
    global: handle_registry_global,
    global_remove: wayland::noop_listener!(),
};

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let wayland_lib = wayland::Lib::load()?.leak();
    let wayland_egl_lib = wayland_egl::Lib::load()?.leak();
    let wayland_cursor_lib = wayland_cursor::Lib::load()?.leak();
    let egl_lib = unsafe { egl::Lib::load()?.leak() };
    let xkbcommon_lib = xkbcommon::Lib::load()?.leak();

    let wl_display = unsafe { (wayland_lib.wl_display_connect)(null_mut()) };
    if wl_display.is_null() {
        return Err(anyhow!("could not connect to wayland display"));
    }

    let egl_context = Rc::new(unsafe { egl::Context::create(egl_lib, wl_display as _)? });
    unsafe { egl_context.make_current_surfaceless()? };

    let gl_lib = unsafe { gl::Lib::load(egl_lib).leak() };

    let wl_registry: *mut wayland::wl_registry =
        unsafe { wayland::wl_display_get_registry(&wayland_lib, wl_display) };
    if wl_registry.is_null() {
        return Err(anyhow!("could not get registry"));
    }

    let mut conn = Rc::new(Connection {
        libs: Libs {
            wayland: wayland_lib,
            wayland_egl: wayland_egl_lib,
            wayland_cursor: wayland_cursor_lib,
            wayland_display: unsafe { NonNull::new_unchecked(wl_display) },
            egl: egl_lib,
            egl_context,
            gl: gl_lib,
            xkbcommon: xkbcommon_lib,
        },
        globals: Globals::default(),
    });

    unsafe {
        (wayland_lib.wl_proxy_add_listener)(
            wl_registry as *mut wayland::wl_proxy,
            &WL_REGISTRY_LISTENER as *const wayland::wl_registry_listener as _,
            Rc::get_mut(&mut conn).unwrap() as *mut Connection as *mut c_void,
        );
        (wayland_lib.wl_display_roundtrip)(wl_display);
    }

    let mut app = App {
        input: wayland_input::Input::new_boxed(&conn)?,
        clipboard: wayland_clipboard::Clipboard::new_boxed(&conn),
        draw_buffer: DrawBuffer::default(),
        renderer: unsafe { Renderer::new(gl_lib)? },
        screens: Vec::new(),
        conn,

        quit_requested: false,
        copy_requested: false,
    };

    app.init_all_screens()?;
    app.capture_all_screens()?;
    app.overlay_all_screens()?;

    loop {
        if app.quit_requested || app.clipboard.cancelled {
            break;
        }

        unsafe {
            (app.conn.libs.wayland.wl_display_dispatch)(app.conn.libs.wayland_display.as_ptr());
        }

        if app.copy_requested {
            continue;
        }

        app.update()?;
        app.draw(ScreenDrawOpts::default())?;
    }

    Ok(())
}
