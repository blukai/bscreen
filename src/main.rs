mod crop;
mod dynlib;
mod egl;
mod gfx;
mod gl;
mod input;
mod renderer;
mod xkbcommon;

use std::collections::{HashMap, VecDeque};
use std::ffi::c_int;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

use anyhow::{anyhow, Context};
use crop::Crop;
use gfx::{DrawBuffer, Rect, RectFill, Size};
use glam::Vec2;
use input::{
    CursorShape, Event, KeyboardEvent, KeyboardEventKind, PointerButton, PointerButtons,
    PointerEvent, PointerEventKind, Scancode,
};
use renderer::Renderer;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_cursor::CursorTheme;
use wayland_egl::WlEglSurface as WlEglWindow;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    self, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{self, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, ZwlrLayerSurfaceV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self, ZwlrScreencopyFrameV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

const DRM_FORMAT_XRGB8888: u32 = 0x34325258;

enum ScreencopyState {
    Pending,
    Ready,
    Failed,
}

#[derive(PartialEq)]
struct ScreencopyDmabufDescriptor {
    format: u32,
    width: u32,
    height: u32,
}

struct ScreencopyDmabuf {
    gl_texture: gl::Texture2D,
    _egl_image_khr: egl::ImageKhr,
    wl_buffer: WlBuffer,
}

impl ScreencopyDmabuf {
    fn new(
        app: &App,
        width: u32,
        height: u32,
        format: u32,
        qhandle: &QueueHandle<App>,
    ) -> anyhow::Result<Self> {
        let gl_texture = unsafe {
            gl::Texture2D::new(
                app.gl_lib,
                width,
                height,
                match format {
                    DRM_FORMAT_XRGB8888 => gfx::TextureFormat::Bgra8Unorm,
                    format => unimplemented!("unhandled fourcc format {format}"),
                },
                None,
            )
        };
        let egl_image_khr =
            unsafe { egl::ImageKhr::new(app.egl_lib, app.egl_context, &gl_texture)? };

        let mut fourcc: c_int = 0;
        let mut num_planes: c_int = 0;
        let mut modifiers: egl::sys::types::EGLuint64KHR = 0;
        if unsafe {
            app.egl_lib.ExportDMABUFImageQueryMESA(
                app.egl_context.display,
                egl_image_khr.handle,
                &mut fourcc,
                &mut num_planes,
                &mut modifiers,
            )
        } == egl::sys::FALSE
        {
            return Err(app.egl_lib.unwrap_err()).context("could not retrieve pixel format");
        }
        // TODO: can there me other number of planes?
        assert!(num_planes == 1);

        let mut fd: c_int = 0;
        let mut stride: egl::sys::types::EGLint = 0;
        let mut offset: egl::sys::types::EGLint = 0;
        if unsafe {
            app.egl_lib.ExportDMABUFImageMESA(
                app.egl_context.display,
                egl_image_khr.handle,
                &mut fd,
                &mut stride,
                &mut offset,
            )
        } == egl::sys::FALSE
        {
            return Err(app.egl_lib.unwrap_err()).context("could not retrieve dmabuf fd");
        }

        let params = app
            .linux_dmabuf
            .as_ref()
            .context("linux dmabuf is unavail")?
            .create_params(qhandle, ());
        params.add(
            unsafe { BorrowedFd::borrow_raw(fd) },
            0,
            offset as _,
            stride as _,
            (modifiers >> 32) as u32,
            (modifiers & (u32::MAX as u64)) as u32,
        );

        let wl_buffer = params.create_immed(
            width as _,
            height as _,
            format,
            zwp_linux_buffer_params_v1::Flags::empty(),
            qhandle,
            (),
        );

        Ok(Self {
            gl_texture,
            _egl_image_khr: egl_image_khr,
            wl_buffer,
        })
    }
}

struct Screencopy {
    state: ScreencopyState,
    dmabuf_desc: Option<ScreencopyDmabufDescriptor>,
    dmabuf: Option<ScreencopyDmabuf>,
}

struct Screen {
    output: WlOutput,
    screencopy: Option<Screencopy>,
    surface: Option<WlSurface>,
    fractional_scale: Option<f64>,
    viewport: Option<WpViewport>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    layer_surface_configured: bool,
    logical_size: Option<Size>,
    egl_window: Option<WlEglWindow>,
    egl_window_surface: Option<egl::WindowSurface>,
    crop: Crop,
}

struct ScreenDrawOpts {
    draw_crop_rect: bool,
    swap_buffers: bool,
}

impl Default for ScreenDrawOpts {
    fn default() -> Self {
        Self {
            draw_crop_rect: true,
            swap_buffers: true,
        }
    }
}

#[derive(PartialEq, Eq, Hash)]
enum SerialType {
    KeyboardEnter,
    PointerEnter,
}

#[derive(Default)]
struct SerialTracker {
    serial_map: HashMap<SerialType, u32>,
}

impl SerialTracker {
    fn update_serial(&mut self, ty: SerialType, serial: u32) {
        self.serial_map.insert(ty, serial);
    }

    fn reset_serial(&mut self, ty: SerialType) {
        self.serial_map.remove(&ty);
    }

    fn get_serial(&self, ty: SerialType) -> Option<u32> {
        self.serial_map.get(&ty).cloned()
    }
}

fn get_surface_id(surface: &WlSurface) -> u64 {
    let mut s = DefaultHasher::new();
    surface.hash(&mut s);
    s.finish()
}

#[derive(Default)]
struct Keyboard {
    xkb_context: Option<xkbcommon::Context>,
}

struct Pointer {
    pointer: WlPointer,

    position: Vec2,
    buttons: PointerButtons,
    frame_events: VecDeque<PointerEvent>,

    cursor_theme: CursorTheme,
    cursor_surface: WlSurface,
}

impl Pointer {
    fn new(
        pointer: WlPointer,
        conn: &Connection,
        qhandle: &QueueHandle<App>,
        shm: WlShm,
        compositor: WlCompositor,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            pointer,

            position: Default::default(),
            buttons: Default::default(),
            frame_events: Default::default(),

            // NOTE: it seems like people on the internet default to 24.
            //
            // TODO: do i need to take scale (/fractional scaling) into account?
            cursor_theme: CursorTheme::load(conn, shm, 24)?,
            cursor_surface: compositor.create_surface(qhandle, ()),
        })
    }

    fn set_cursor(&mut self, shape: CursorShape, serial_tracker: &SerialTracker) {
        let Some(serial) = serial_tracker.get_serial(SerialType::PointerEnter) else {
            log::warn!("no pointer enter serial found");
            return;
        };

        let Some(cursor) = self.cursor_theme.get_cursor(shape.name()) else {
            log::warn!("could not find {} cursor", shape.name());
            return;
        };

        // i fucking hate this
        let cursor_img_buf = &cursor[0];
        // TODO: might need to take into account wl.Output.Event.scale event to set
        // buffer scale.
        self.cursor_surface.attach(Some(cursor_img_buf), 0, 0);
        // NOTE: pre version 4 wl_surface::damage must be used instead.
        assert!(self.cursor_surface.version() >= 4);
        let (width, height) = cursor_img_buf.dimensions();
        self.cursor_surface
            .damage_buffer(0, 0, width as i32, height as i32);
        self.cursor_surface.commit();

        let (hotspot_x, hotspot_y) = cursor_img_buf.hotspot();
        self.pointer.set_cursor(
            serial,
            Some(&self.cursor_surface),
            hotspot_x as i32,
            hotspot_y as i32,
        );
    }
}

struct ClipboardDataOffer {
    data_source: WlDataSource,

    mime_type: String,
    data: Vec<u8>,
}

struct Clipboard {
    data_device: WlDataDevice,

    data_offer: Option<ClipboardDataOffer>,
}

struct App {
    egl_lib: &'static egl::Lib,
    egl_context: &'static egl::Context,
    gl_lib: &'static gl::Lib,
    xkbcommon_lib: &'static xkbcommon::Lib,

    compositor: Option<WlCompositor>,
    fractional_scale_manager: Option<WpFractionalScaleManagerV1>,
    layer_shell: Option<ZwlrLayerShellV1>,
    linux_dmabuf: Option<ZwpLinuxDmabufV1>,
    screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    seat: Option<WlSeat>,
    viewporter: Option<WpViewporter>,
    shm: Option<WlShm>,
    data_device_manager: Option<WlDataDeviceManager>,

    screens: Vec<Screen>,

    // TODO: introduce input state or something
    keyboard: Option<Keyboard>,
    pointer: Option<Pointer>,
    serial_tracker: SerialTracker,
    keyboard_focused_surface_id: Option<u64>,
    pointer_focused_surface_id: Option<u64>,
    clipboard: Option<Clipboard>,

    events: VecDeque<Event>,
    draw_buffer: DrawBuffer,
    renderer: Renderer,

    copy: bool,
    quit: bool,
}

impl App {
    fn capture_all_screens(
        &mut self,
        event_queue: &mut EventQueue<Self>,
        qhandle: &QueueHandle<Self>,
    ) -> anyhow::Result<()> {
        let Some(screencopy_manager) = self.screencopy_manager.as_ref() else {
            return Err(anyhow!("screencopy manager is unavail"));
        };

        for (idx, screen) in self.screens.iter_mut().enumerate() {
            assert!(screen.screencopy.is_none());
            screencopy_manager.capture_output(1, &screen.output, qhandle, idx);
            screen.screencopy.replace(Screencopy {
                state: ScreencopyState::Pending,
                dmabuf_desc: None,
                dmabuf: None,
            });
        }

        loop {
            let mut pending = 0;
            for screen in self.screens.iter() {
                let Some(screencopy) = screen.screencopy.as_ref() else {
                    unreachable!()
                };
                match screencopy.state {
                    ScreencopyState::Failed => return Err(anyhow!("screencopy failed")),
                    ScreencopyState::Pending => pending += 1,
                    _ => {}
                }
            }
            if pending == 0 {
                break Ok(());
            }
            event_queue.blocking_dispatch(self)?;
        }
    }

    fn overlay_all_screens(
        &mut self,
        event_queue: &mut EventQueue<Self>,
        qhandle: &QueueHandle<Self>,
    ) -> anyhow::Result<()> {
        let Some(compositor) = self.compositor.as_ref() else {
            return Err(anyhow!("compositor is unavail"));
        };
        let Some(layer_shell) = self.layer_shell.as_ref() else {
            return Err(anyhow!("layer shell is unavail"));
        };
        let Some(fractional_scale_manager) = self.fractional_scale_manager.as_ref() else {
            return Err(anyhow!("fractional scale manager is unavail"));
        };
        let Some(viewporter) = self.viewporter.as_ref() else {
            return Err(anyhow!("viewporter is unavail;"));
        };

        for (idx, screen) in self.screens.iter_mut().enumerate() {
            let surface = compositor.create_surface(qhandle, ());
            fractional_scale_manager.get_fractional_scale(&surface, qhandle, idx);
            let viewport = viewporter.get_viewport(&surface, qhandle, ());
            let layer_surface = layer_shell.get_layer_surface(
                &surface,
                Some(&screen.output),
                zwlr_layer_shell_v1::Layer::Overlay,
                "bscreen".to_string(),
                qhandle,
                idx,
            );

            layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::all());
            // > If set to -1, the surface indicates that it would not like to be moved to
            // accommodate for other surfaces, and the compositor should extend it all the way to
            // the edges it is anchored to.
            layer_surface.set_exclusive_zone(-1);
            layer_surface.set_keyboard_interactivity(
                zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
            );

            // > After creating a layer_surface object and setting it up, the client
            // must perform an initial commit without any buffer attached. The
            // compositor will reply with a layer_surface.configure event.
            surface.commit();

            screen.surface.replace(surface);
            screen.viewport.replace(viewport);
            screen.layer_surface.replace(layer_surface);
        }

        loop {
            let mut pending = 0;
            for screen in self.screens.iter() {
                pending += !screen.layer_surface_configured as usize;
            }
            if pending == 0 {
                break Ok(());
            }
            event_queue.blocking_dispatch(self)?;
        }
    }

    fn update_all_screens(&mut self, event: &Event) {
        let surface_id = match event {
            Event::Pointer(ref pointer_event) => pointer_event.surface_id,
            Event::Keyboard(ref keyboard_event) => keyboard_event.surface_id,
        };
        let screen_idx = self
            .screens
            .iter()
            .enumerate()
            .find(|(_, screen)| {
                let surface = screen
                    .surface
                    .as_ref()
                    .expect("screen surface to have been created");
                get_surface_id(surface) == surface_id
            })
            .map(|(idx, _)| idx)
            .expect("find screen by surface id");

        let screen = self.screens.get_mut(screen_idx).unwrap();
        let view_rect = Rect::new(
            Vec2::ZERO,
            screen.logical_size.unwrap().as_uvec2().as_vec2(),
        );
        let crop_mutated = screen.crop.update(view_rect, event);
        if let Some(shape) = screen.crop.cursor.as_ref() {
            if let Some(pointer) = self.pointer.as_mut() {
                pointer.set_cursor(*shape, &self.serial_tracker);
            }
        }

        // maybe clear crops on other screens
        if crop_mutated {
            for other_idx in 0..self.screens.len() {
                if other_idx == screen_idx {
                    continue;
                }
                let other = self.screens.get_mut(other_idx).unwrap();
                _ = other.crop = Default::default();
            }
        }
    }

    fn draw_screen(&mut self, screen_idx: usize, opts: ScreenDrawOpts) -> anyhow::Result<()> {
        let screen = &self.screens[screen_idx];

        let egl_window_surface = screen.egl_window_surface.as_ref().unwrap();
        let screencopy = screen.screencopy.as_ref().unwrap();
        let screencopy_dmabuf = screencopy.dmabuf.as_ref().unwrap();
        let logical_size = screen.logical_size.unwrap();
        let fractional_scale = screen.fractional_scale.unwrap_or(1.0);
        let view_rect = Rect::new(Vec2::splat(0.0), logical_size.as_uvec2().as_vec2());

        unsafe {
            self.egl_context.make_current(egl_window_surface.handle)?;

            self.gl_lib.ClearColor(0.0, 0.0, 0.0, 0.0);
            self.gl_lib.Clear(gl::sys::COLOR_BUFFER_BIT);
        }

        self.draw_buffer.clear();

        self.draw_buffer.push_rect_filled(
            view_rect,
            RectFill::TextureHandle(screencopy_dmabuf.gl_texture.handle),
        );

        if opts.draw_crop_rect {
            screen.crop.draw(&mut self.draw_buffer);

            // maybe darken if any other screen has crop
            if screen.crop.crop_rect.is_none() {
                let other_has_crop_rect =
                    self.screens.iter().enumerate().any(|(other_idx, other)| {
                        other_idx != screen_idx && other.crop.crop_rect.is_some()
                    });
                if other_has_crop_rect {
                    self.draw_buffer
                        .push_rect_filled(view_rect, RectFill::Color(crop::theme::OUTSIDE_BG));
                }
            }
        }

        unsafe {
            self.renderer
                .draw(logical_size, fractional_scale, &self.draw_buffer);

            if opts.swap_buffers {
                assert!(!self.copy);

                self.egl_context.swap_buffers(egl_window_surface.handle)?;
            }
        }

        Ok(())
    }

    fn offer_clipboard_data(
        &mut self,
        event_queue: &mut EventQueue<Self>,
        qhandle: &QueueHandle<Self>,
        mime_type: String,
        data: Vec<u8>,
    ) {
        let Some(serial) = self.serial_tracker.get_serial(SerialType::KeyboardEnter) else {
            log::warn!("failed to write clipboard data (no keyboard press serial found)");
            return;
        };

        let data_device_manager = self
            .data_device_manager
            .as_ref()
            .expect("data device manager is unavail");
        let seat = self.seat.as_ref().expect("seat is unavail");

        let clipboard = self.clipboard.get_or_insert_with(|| Clipboard {
            data_device: data_device_manager.get_data_device(seat, qhandle, ()),
            data_offer: None,
        });

        if let Some(data_offer) = clipboard.data_offer.take() {
            data_offer.data_source.destroy();
        }

        let data_source = data_device_manager.create_data_source(qhandle, ());
        data_source.offer("image/png".to_string());

        clipboard
            .data_device
            .set_selection(Some(&data_source), serial);

        clipboard.data_offer = Some(ClipboardDataOffer {
            data_source,

            mime_type,
            data,
        });

        event_queue.flush().expect("could not flush");
    }

    fn handle_copy_request(
        &mut self,
        event_queue: &mut EventQueue<Self>,
        qhandle: &QueueHandle<Self>,
    ) {
        let Some(idx) = self
            .screens
            .iter_mut()
            .enumerate()
            .find(|(_, screen)| screen.crop.crop_rect.is_some())
            .map(|(idx, _)| idx)
        else {
            return;
        };
        assert!(self.screens[idx].crop.view_rect.is_some());

        self.copy = true;

        for screen in self.screens.iter_mut() {
            let layer_surface = screen.layer_surface.take().unwrap();
            // NOTE: drop(layer_surface) doesn't do the thing.
            layer_surface.destroy();
            screen.layer_surface_configured = false;
        }
        log::info!("destroyed layer surfaces on all screens");

        self.draw_screen(
            idx,
            ScreenDrawOpts {
                draw_crop_rect: false,
                swap_buffers: false,
            },
        )
        .expect("failed to draw");

        let (pixels, size) = {
            let screen = &self.screens[idx];
            let fractional_scale = screen.fractional_scale.unwrap_or(1.0);
            let read_rect = screen.crop.crop_rect.clone().unwrap() * fractional_scale as f32;
            let view_size = screen
                .logical_size
                .as_ref()
                .unwrap()
                .to_physical(fractional_scale);

            let pixels = unsafe { gl::read_pixels(self.gl_lib, read_rect, view_size) };

            (pixels, read_rect.size())
        };

        self.screens.clear();
        log::info!("destroyed all screens");

        // TODO: encode pixels to png
        let mut data: Vec<u8> = Vec::new();

        {
            let mut encoder = png::Encoder::new(&mut data, size.x as u32, size.y as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);

            let mut writer = encoder.write_header().expect("could not write png header");
            writer
                .write_image_data(&pixels)
                .expect("could not write png data");
        }

        // TODO: send a clipboard data offer
        self.offer_clipboard_data(event_queue, qhandle, "image/png".to_string(), data);
    }
}

impl Dispatch<WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: <WlRegistry as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        use wl_registry::Event::*;

        log::trace!("wl_registry::Event::{event:?}");

        match event {
            Global {
                interface,
                name,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    state
                        .compositor
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state
                        .layer_shell
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                "zwp_linux_dmabuf_v1" => {
                    state
                        .linux_dmabuf
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                "zwlr_screencopy_manager_v1" => {
                    state
                        .screencopy_manager
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                "wl_output" => {
                    state.screens.push(Screen {
                        output: registry.bind(name, version, qhandle, ()),
                        screencopy: None,
                        surface: None,
                        fractional_scale: None,
                        viewport: None,
                        layer_surface: None,
                        layer_surface_configured: false,
                        logical_size: None,
                        egl_window: None,
                        egl_window_surface: None,
                        crop: Crop::default(),
                    });
                }
                "wl_seat" => {
                    state
                        .seat
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                "wp_viewporter" => {
                    state
                        .viewporter
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                "wp_fractional_scale_manager_v1" => {
                    state.fractional_scale_manager.replace(registry.bind(
                        name,
                        version,
                        qhandle,
                        (),
                    ));
                }
                "wl_shm" => {
                    state.shm.replace(registry.bind(name, version, qhandle, ()));
                }
                "wl_data_device_manager" => {
                    state
                        .data_device_manager
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                _ => {}
            },
            _ => unimplemented!("unhandled wl_registry event {event:?}"),
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, usize> for App {
    fn event(
        state: &mut Self,
        proxy: &ZwlrScreencopyFrameV1,
        event: <ZwlrScreencopyFrameV1 as Proxy>::Event,
        data: &usize,
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event::*;

        log::trace!("zwlr_screencopy_frame_v1::Event::{event:?}");

        let idx = *data;
        let Some(mut screencopy) = state.screens[idx].screencopy.take() else {
            unreachable!();
        };

        match event {
            LinuxDmabuf {
                format,
                width,
                height,
            } => {
                let next_dmabuf_desc = ScreencopyDmabufDescriptor {
                    format,
                    width,
                    height,
                };
                if !screencopy
                    .dmabuf_desc
                    .as_ref()
                    .is_some_and(|prev_dmabuf_desc| prev_dmabuf_desc.eq(&next_dmabuf_desc))
                {
                    screencopy.dmabuf_desc.replace(next_dmabuf_desc);
                    screencopy.dmabuf.replace(
                        ScreencopyDmabuf::new(state, width, height, format, qhandle)
                            .expect("failed to create screencopy dmabuf"),
                    );
                }
            }
            BufferDone => {
                proxy.copy(&screencopy.dmabuf.as_ref().unwrap().wl_buffer);
            }
            Ready { .. } => {
                screencopy.state = ScreencopyState::Ready;
            }
            Failed => {
                screencopy.state = ScreencopyState::Failed;
            }
            _ => {}
        }

        state.screens[idx].screencopy.replace(screencopy);
    }
}

impl Dispatch<WpFractionalScaleV1, usize> for App {
    fn event(
        state: &mut Self,
        _proxy: &WpFractionalScaleV1,
        event: <WpFractionalScaleV1 as Proxy>::Event,
        data: &usize,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wp_fractional_scale_v1::Event::*;

        log::trace!("wp_fractional_scale_v1::Event::{event:?}");

        let idx = *data;
        let screen = &mut state.screens[idx];

        match event {
            PreferredScale { scale } => {
                // > The sent scale is the numerator of a fraction with a denominator of 120.
                let fractional_scale = scale as f64 / 120.0;
                log::debug!("recv fractional scale {fractional_scale} for output {idx}");
                screen.fractional_scale.replace(fractional_scale);
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, usize> for App {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: <ZwlrLayerSurfaceV1 as Proxy>::Event,
        data: &usize,
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        use zwlr_layer_surface_v1::Event::*;

        log::trace!("zwlr_layer_surface_v1::Event::{event:?}");

        let idx = *data;

        match event {
            Configure {
                serial,
                width,
                height,
            } => {
                log::debug!("recv layer surface configre {width}x{height} for output {idx}");
                proxy.ack_configure(serial);

                let logical_size = Size::new(width, height);
                let fractional_scale = state.screens[idx].fractional_scale.unwrap_or(1.0);
                let physical_size = logical_size.to_physical(fractional_scale);

                let surface_id = state.screens[idx].surface.as_ref().unwrap().id();
                let egl_window = WlEglWindow::new(
                    surface_id,
                    physical_size.width as i32,
                    physical_size.height as i32,
                )
                .expect("failed to create wl egl window");
                let egl_window_surface = unsafe {
                    egl::WindowSurface::new(state.egl_lib, state.egl_context, egl_window.ptr())
                        .expect("failed to create egl window surface")
                };

                state.screens[idx]
                    .viewport
                    .as_ref()
                    .unwrap()
                    .set_destination(logical_size.width as i32, logical_size.height as i32);
                state.screens[idx].surface.as_ref().unwrap().commit();

                state.screens[idx].layer_surface_configured = true;
                state.screens[idx].logical_size.replace(logical_size);
                state.screens[idx].egl_window.replace(egl_window);
                state.screens[idx]
                    .egl_window_surface
                    .replace(egl_window_surface);

                state
                    .draw_screen(idx, ScreenDrawOpts::default())
                    .expect("failed to draw");

                state.screens[idx]
                    .surface
                    .as_ref()
                    .unwrap()
                    .frame(qhandle, idx);
            }
            Closed => unimplemented!(),
            _ => unreachable!(),
        }
    }
}

impl Dispatch<WlCallback, usize> for App {
    fn event(
        state: &mut Self,
        _proxy: &WlCallback,
        event: <WlCallback as Proxy>::Event,
        data: &usize,
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        use wl_callback::Event::*;

        log::trace!("wl_callback::Event::{event:?}");

        let idx = *data;

        match event {
            Done { .. } => {
                if state.copy {
                    return;
                }

                state
                    .draw_screen(idx, ScreenDrawOpts::default())
                    .expect("failed to draw");

                let surface = &state.screens[idx].surface.as_ref().unwrap();
                surface.frame(qhandle, idx);
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<WlSeat, ()> for App {
    fn event(
        state: &mut Self,
        proxy: &WlSeat,
        event: <WlSeat as Proxy>::Event,
        _data: &(),
        conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        log::trace!("wl_seat::Event::{event:?}");

        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        let capabilities = wl_seat::Capability::from_bits_truncate(capabilities.into());
        if !capabilities.contains(wl_seat::Capability::Keyboard) {
            panic!("no keyboard capability");
        }
        if !capabilities.contains(wl_seat::Capability::Pointer) {
            panic!("no pointer capability");
        }

        assert!(state.keyboard.is_none());
        proxy.get_keyboard(qhandle, ());
        state.keyboard = Some(Keyboard::default());

        assert!(state.pointer.is_none());
        state.pointer = Some(
            Pointer::new(
                proxy.get_pointer(qhandle, ()),
                conn,
                qhandle,
                state.shm.clone().expect("shm is unavail"),
                state.compositor.clone().expect("compositor is unavail"),
            )
            .expect("could not construct pointer"),
        );
    }
}

impl Dispatch<WlKeyboard, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &WlKeyboard,
        event: <WlKeyboard as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wl_keyboard::Event::*;

        log::trace!("wl_keyboard::Event::{event:?}");

        let Some(keyboard) = state.keyboard.as_mut() else {
            return;
        };

        match event {
            Enter {
                serial, surface, ..
            } => {
                state.keyboard_focused_surface_id = Some(get_surface_id(&surface));
                state
                    .serial_tracker
                    .update_serial(SerialType::KeyboardEnter, serial);
            }
            Leave { .. } => {
                state.keyboard_focused_surface_id = None;
                state.serial_tracker.reset_serial(SerialType::KeyboardEnter);
            }
            Keymap { format, fd, size } => match format {
                WEnum::Value(value) => match value {
                    wl_keyboard::KeymapFormat::NoKeymap => {
                        log::warn!("non-xkb keymap");
                    }
                    wl_keyboard::KeymapFormat::XkbV1 => {
                        assert!(keyboard.xkb_context.is_none());
                        let xkb_context = unsafe {
                            xkbcommon::Context::from_fd(state.xkbcommon_lib, fd.as_fd(), size)
                                .expect("failed to create xkb context")
                        };
                        keyboard.xkb_context.replace(xkb_context);
                        log::info!("created xkb context");
                    }
                    _ => unreachable!(),
                },
                WEnum::Unknown(value) => {
                    log::warn!("unknown keymap format 0x{:x}", value);
                }
            },
            Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                let Some(xkb_context) = keyboard.xkb_context.as_mut() else {
                    return;
                };
                unsafe {
                    xkb_context.update_mods(mods_depressed, mods_latched, mods_locked, 0, 0, group)
                };
            }
            Key {
                key,
                state: key_state,
                ..
            } => {
                let Some(xkb_context) = keyboard.xkb_context.as_mut() else {
                    return;
                };
                let scancode = Scancode::from_int(key);
                let event = KeyboardEvent {
                    kind: match key_state {
                        WEnum::Value(wl_keyboard::KeyState::Pressed) => {
                            KeyboardEventKind::Press { scancode }
                        }
                        WEnum::Value(wl_keyboard::KeyState::Released) => {
                            KeyboardEventKind::Release { scancode }
                        }
                        _ => unreachable!(),
                    },
                    surface_id: state
                        .keyboard_focused_surface_id
                        .expect("keyboard didn't enter?"),
                    mods: xkb_context.mods.clone(),
                };
                state.events.push_back(Event::Keyboard(event));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &WlPointer,
        event: <WlPointer as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wl_pointer::Event::*;

        log::trace!("wl_keyboard::Event::{event:?}");

        let Some(pointer) = state.pointer.as_mut() else {
            return;
        };

        match event {
            Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                pointer.position = Vec2::new(surface_x as f32, surface_y as f32);
                state.pointer_focused_surface_id = Some(get_surface_id(&surface));
                state
                    .serial_tracker
                    .update_serial(SerialType::PointerEnter, serial);
            }
            Leave { .. } => {
                state.pointer_focused_surface_id = None;
                state.serial_tracker.reset_serial(SerialType::PointerEnter);
            }
            Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let prev_position = pointer.position;
                let next_position = Vec2::new(surface_x as f32, surface_y as f32);
                let frame_event = PointerEvent {
                    kind: input::PointerEventKind::Motion {
                        delta: next_position - prev_position,
                    },
                    surface_id: state
                        .pointer_focused_surface_id
                        .expect("pointer didn't enter?"),
                    position: next_position,
                    buttons: pointer.buttons.clone(),
                };

                pointer.position = next_position;
                pointer.frame_events.push_back(frame_event);
            }
            Button {
                button,
                state: button_state,
                ..
            } => {
                let button = PointerButton::from_int(button);
                let event_kind = match button_state {
                    WEnum::Value(wl_pointer::ButtonState::Pressed) => {
                        PointerEventKind::Press { button }
                    }
                    WEnum::Value(wl_pointer::ButtonState::Released) => {
                        PointerEventKind::Release { button }
                    }
                    _ => unreachable!(),
                };
                let pressed = event_kind == PointerEventKind::Press { button };
                let next_buttons = {
                    let mut nb = pointer.buttons.clone();
                    match button {
                        PointerButton::Left => nb.left = pressed,
                        _ => {}
                    }
                    nb
                };
                let frame_event = PointerEvent {
                    kind: event_kind,
                    surface_id: state
                        .pointer_focused_surface_id
                        .expect("pointer didn't enter?"),
                    position: pointer.position.clone(),
                    buttons: next_buttons.clone(),
                };

                pointer.buttons = next_buttons;
                pointer.frame_events.push_back(frame_event);
            }
            Frame => {
                state
                    .events
                    .extend(pointer.frame_events.drain(..).map(Event::Pointer));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &WlDataSource,
        event: <WlDataSource as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        use wl_data_source::Event::*;

        log::trace!("wl_keyboard::Event::{event:?}");

        let Some(clipboard) = state.clipboard.as_ref() else {
            log::warn!("received data source event, but clipboard is None");
            return;
        };
        let Some(data_offer) = clipboard.data_offer.as_ref() else {
            log::warn!("received data source event, but clipboard data offer is None");
            return;
        };

        match event {
            Send { mime_type, fd } => {
                // TODO: can we receive request for other mime, not the one that was
                // offered? probably not?
                assert!(mime_type.eq(&data_offer.mime_type));

                unsafe {
                    let n = libc::write(
                        fd.as_raw_fd(),
                        data_offer.data.as_ptr() as _,
                        data_offer.data.len(),
                    );

                    // TODO: do i need to handle cases when n is not equal to len of data?
                    assert!(n as usize == data_offer.data.len());
                }
            }
            Cancelled => {
                state.quit = true;
            }
            _ => {}
        }
    }
}

delegate_noop!(App: ignore WlBuffer);
delegate_noop!(App: ignore WlCompositor);
delegate_noop!(App: ignore WlDataDevice);
delegate_noop!(App: ignore WlDataDeviceManager);
delegate_noop!(App: ignore WlOutput);
delegate_noop!(App: ignore WlShm);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: ignore WpFractionalScaleManagerV1);
delegate_noop!(App: ignore WpViewport);
delegate_noop!(App: ignore WpViewporter);
delegate_noop!(App: ignore ZwlrLayerShellV1);
delegate_noop!(App: ignore ZwlrScreencopyManagerV1);
delegate_noop!(App: ignore ZwpLinuxBufferParamsV1);
delegate_noop!(App: ignore ZwpLinuxDmabufV1);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let connection = Connection::connect_to_env()?;
    let mut event_queue = connection.new_event_queue();
    let qhandle = event_queue.handle();

    let display = connection.display();
    let _registry = display.get_registry(&qhandle, ());

    // this is not super rust'y, but i don't care. egl lib is not going to run away from here.
    let egl_lib_ = unsafe { egl::Lib::load()? };
    let egl_lib: &'static egl::Lib = unsafe { std::mem::transmute(&egl_lib_) };
    let egl_context_ =
        unsafe { egl::Context::create(egl_lib, connection.backend().display_ptr() as _)? };
    let egl_context: &'static egl::Context = unsafe { std::mem::transmute(&egl_context_) };
    let gl_lib_ = unsafe { gl::Lib::load(egl_lib) };
    let gl_lib: &'static gl::Lib = unsafe { std::mem::transmute(&gl_lib_) };
    let xkbcommon_lib_ = unsafe { xkbcommon::Lib::load()? };
    let xkbcommon_lib: &'static xkbcommon::Lib = unsafe { std::mem::transmute(&xkbcommon_lib_) };

    let mut app = App {
        egl_lib,
        egl_context,
        gl_lib,
        xkbcommon_lib,

        compositor: None,
        fractional_scale_manager: None,
        layer_shell: None,
        linux_dmabuf: None,
        screencopy_manager: None,
        seat: None,
        viewporter: None,
        shm: None,
        data_device_manager: None,

        screens: vec![],

        keyboard: Default::default(),
        pointer: Default::default(),
        serial_tracker: Default::default(),
        keyboard_focused_surface_id: None,
        pointer_focused_surface_id: None,
        clipboard: None,

        events: VecDeque::new(),
        draw_buffer: DrawBuffer::default(),
        renderer: unsafe { Renderer::new(gl_lib)? },

        copy: false,
        quit: false,
    };

    event_queue.roundtrip(&mut app)?;

    app.capture_all_screens(&mut event_queue, &qhandle)?;
    app.overlay_all_screens(&mut event_queue, &qhandle)?;

    while !app.quit {
        event_queue.blocking_dispatch(&mut app)?;

        if app.copy {
            continue;
        }

        while let Some(event) = app.events.pop_front() {
            match event {
                Event::Keyboard(ref keyboard_event) => match keyboard_event.kind {
                    KeyboardEventKind::Press {
                        scancode: Scancode::Esc,
                    } => app.quit = true,
                    KeyboardEventKind::Press {
                        scancode: Scancode::C,
                    } if keyboard_event.mods.ctrl => {
                        app.handle_copy_request(&mut event_queue, &qhandle);
                        continue;
                    }
                    _ => {}
                },
                Event::Pointer(_) => {}
            }

            app.update_all_screens(&event);
        }
    }

    Ok(())
}
