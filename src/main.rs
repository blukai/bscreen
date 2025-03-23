mod crop;
mod dynlib;
mod egl;
mod fontprovider;
mod fonttexturecache;
mod genvec;
mod gfx;
mod gl;
mod input;
mod ntree;
mod renderer;
mod texturepacker;
mod wayland;
mod wayland_clipboard;
mod wayland_cursor;
mod wayland_egl;
mod wayland_input;
mod wayland_overlay;
mod wayland_screencopy;
mod welcome;
mod xkbcommon;

use std::{
    ffi::{CStr, c_char, c_void},
    ptr::{NonNull, null_mut},
    rc::Rc,
};

use anyhow::{Context as _, anyhow};
use crop::{Crop, CropUpdateData};
use fontprovider::{Font, FontProvider};
use fonttexturecache::FontTextureCache;
use genvec::Handle;
use gfx::{DrawBuffer, Rect, RectFill, Size, Vec2};
use input::{Event, KeyboardEventKind, Scancode, SerialType};
use renderer::Renderer;
use welcome::{Welcome, WelcomeUpdateData};

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
    output: NonNull<wayland::wl_output>,

    screencopy: Option<Box<wayland_screencopy::Screencopy>>,
    overlay: Option<Box<wayland_overlay::Overlay>>,

    welcome: Welcome,
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

struct App {
    input: Box<wayland_input::Input>,
    clipboard: Box<wayland_clipboard::Clipboard>,
    draw_buffer: DrawBuffer,
    renderer: Renderer,
    screens: Vec<Screen>,
    conn: Rc<Connection>,

    font_provider: FontProvider,
    font_texture_cache: FontTextureCache,
    font_handle: Handle<Font>,

    quit_requested: bool,
    copy_requested: bool,
}

impl App {
    fn init_all_screens(&mut self) -> anyhow::Result<()> {
        assert!(self.screens.is_empty());
        self.screens.reserve_exact(self.conn.globals.outputs.len());
        for output in self.conn.globals.outputs.iter() {
            self.screens.push(Screen {
                output: NonNull::new(*output).context("whoopsie, output is null")?,

                screencopy: None,
                overlay: None,

                welcome: Welcome::default(),
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
        for i in 0..self.screens.len() {
            self.draw_screen_at_index(i, &screen_draw_opts)?;
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
                // NOTE: this is ugly, but i don't really care.
                //
                // i want to be able to iterate all the screens and remove crops from other screens
                // that weren't updated.
                // to ensure that this one will not be updated i check i == j.
                let screen = unsafe { &mut *(&mut self.screens[i] as *mut _) as &mut Screen };

                let overlay = screen.overlay.as_ref().unwrap();

                // NOTE: keyboard surface id may not match with pointer surface id; i want to
                // operate on pointer-focused surface.
                let screen_surface_id = wayland_input::get_surface_id(overlay.surface);
                let Some(pointer_surface_id) = self.input.pointer_focused_surface_id else {
                    continue;
                };
                let this_screen_focused = screen_surface_id == pointer_surface_id;

                let logical_size = overlay.logical_size.unwrap();
                let view_rect = Rect::new(Vec2::ZERO, logical_size.as_vec2());

                if this_screen_focused {
                    let crop_updated = screen.crop.update(&event, CropUpdateData { view_rect });

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

                screen.welcome.update(&event, WelcomeUpdateData {
                    view_rect,
                    any_crop_has_selection: self
                        .screens
                        .iter()
                        .any(|screen| screen.crop.crop_rect.is_some()),
                    this_screen_focused,
                    font_provider: &self.font_provider,
                    font_handle: self.font_handle,
                });
            }
        }

        Ok(())
    }

    fn draw_screen_at_index(
        &mut self,
        index: usize,
        draw_opts: &ScreenDrawOpts,
    ) -> anyhow::Result<()> {
        let screen = &mut self.screens[index];

        let screencopy = screen.screencopy.as_ref().unwrap().as_ref();
        let overlay = screen.overlay.as_ref().unwrap();

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

        self.draw_buffer.clear();

        self.draw_buffer
            .push_rect_filled(view_rect, RectFill::Texture {
                handle: dmabuf.gl_texture.handle,
                coords: Rect::new(Vec2::splat(0.0), Vec2::splat(1.0)),
            });

        if draw_opts.draw_crop_decorations {
            if screen.crop.crop_rect.is_some() {
                screen.crop.draw(&mut self.draw_buffer);
            } else {
                // TODO: should this be state of the crop?
                self.draw_buffer
                    .push_rect_filled(view_rect, RectFill::Color(crop::theme::OUTSIDE_BG));
            }
        }

        screen
            .welcome
            .draw(&mut self.draw_buffer, welcome::WelcomeDrawData {
                font_provider: &self.font_provider,
                font_texture_cache: &mut self.font_texture_cache,
                font_handle: self.font_handle,
                gl_lib: self.conn.libs.gl,
            });

        unsafe {
            self.renderer
                .draw(logical_size, fractional_scale, &self.draw_buffer);
        }

        if draw_opts.swap_buffers {
            unsafe {
                self.conn
                    .libs
                    .egl_context
                    .swap_buffers(window_surface.handle)?;
            }
        }

        Ok(())
    }

    fn draw(&mut self, screen_draw_opts: ScreenDrawOpts) -> anyhow::Result<()> {
        for i in 0..self.screens.len() {
            self.draw_screen_at_index(i, &screen_draw_opts)?;
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
            self.draw_screen_at_index(screen_idx, &ScreenDrawOpts {
                draw_crop_decorations: false,
                swap_buffers: false,
            })?;

            let screen = &self.screens[screen_idx];
            let overlay = screen.overlay.as_ref().unwrap();

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

    let mut font_provider = FontProvider::default();
    let font_handle = font_provider
        .create_font(include_bytes!("../assets/JetBrainsMono-Regular.ttf"), 24.0)
        .context("could not create font")?;
    let font_texture_cache = FontTextureCache::default();

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

        font_provider,
        font_handle,
        font_texture_cache,

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
