mod dynlib;
mod egl;
mod gfx;
mod gl;
mod input;
mod renderer;
mod xkbcommon;

use std::collections::VecDeque;
use std::ffi::c_int;
use std::os::fd::{AsFd, BorrowedFd};

use anyhow::{anyhow, Context};
use gfx::{DrawBuffer, Rect, RectFill, Size};
use glam::Vec2;
use input::{Event, KeyboardEvent, KeyboardEventKind, Scancode};
use renderer::Renderer;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::{self, KeyState, WlKeyboard};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
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
}

impl Screen {
    fn draw(
        &self,
        egl_context: &'static egl::Context,
        gl_lib: &'static gl::Lib,
        draw_buffer: &mut DrawBuffer,
        renderer: &Renderer,
    ) -> anyhow::Result<()> {
        let egl_window_surface = self.egl_window_surface.as_ref().unwrap();
        let screencopy = self.screencopy.as_ref().unwrap();
        let screencopy_dmabuf = screencopy.dmabuf.as_ref().unwrap();
        let logical_size = self.logical_size.unwrap();
        let fractional_scale = self.fractional_scale.unwrap_or(1.0);

        unsafe {
            egl_context.make_current(egl_window_surface.handle)?;

            gl_lib.ClearColor(0.0, 0.0, 0.0, 0.0);
            gl_lib.Clear(gl::sys::COLOR_BUFFER_BIT);
        }

        draw_buffer.clear();

        draw_buffer.push_rect_filled(
            Rect::new(Vec2::splat(0.0), logical_size.as_uvec2().as_vec2()),
            RectFill::TextureHandle(screencopy_dmabuf.gl_texture.handle),
        );

        unsafe {
            renderer.draw(logical_size, fractional_scale, draw_buffer);

            egl_context.swap_buffers(egl_window_surface.handle)?;
        }

        Ok(())
    }
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

    screens: Vec<Screen>,
    xkb_context: Option<xkbcommon::Context>,

    events: VecDeque<Event>,
    draw_buffer: DrawBuffer,
    renderer: Renderer,

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
        let screen = &mut state.screens[idx];

        match event {
            Configure {
                serial,
                width,
                height,
            } => {
                log::debug!("recv layer surface configre {width}x{height} for output {idx}");
                proxy.ack_configure(serial);

                let logical_size = Size::new(width, height);
                let physical_size =
                    logical_size.to_physical(screen.fractional_scale.unwrap_or(1.0));

                let surface = screen.surface.as_ref().unwrap();

                let egl_window = WlEglWindow::new(
                    surface.id(),
                    physical_size.width as i32,
                    physical_size.height as i32,
                )
                .expect("failed to create wl egl window");
                let egl_window_surface = unsafe {
                    egl::WindowSurface::new(state.egl_lib, state.egl_context, egl_window.ptr())
                        .expect("failed to create egl window surface")
                };

                let Some(viewport) = screen.viewport.as_ref() else {
                    unreachable!()
                };
                viewport.set_destination(logical_size.width as i32, logical_size.height as i32);
                surface.commit();

                screen.layer_surface_configured = true;
                screen.logical_size.replace(logical_size);
                screen.egl_window.replace(egl_window);
                screen.egl_window_surface.replace(egl_window_surface);

                screen
                    .draw(
                        state.egl_context,
                        state.gl_lib,
                        &mut state.draw_buffer,
                        &state.renderer,
                    )
                    .expect("failed to draw");

                surface.frame(qhandle, idx);
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
        let screen = &mut state.screens[idx];

        match event {
            Done { .. } => {
                let Some(surface) = screen.surface.as_ref() else {
                    unreachable!()
                };

                screen
                    .draw(
                        state.egl_context,
                        state.gl_lib,
                        &mut state.draw_buffer,
                        &state.renderer,
                    )
                    .expect("failed to draw");

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
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        log::trace!("wl_seat::Event::{event:?}");

        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        let capabilities = wl_seat::Capability::from_bits_truncate(capabilities.into());
        if capabilities.contains(wl_seat::Capability::Keyboard) {
            proxy.get_keyboard(qhandle, ());
        } else {
            state.xkb_context.take();
        }
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

        match event {
            Keymap { format, fd, size } => match format {
                WEnum::Value(value) => match value {
                    wl_keyboard::KeymapFormat::NoKeymap => {
                        log::warn!("non-xkb keymap");
                    }
                    wl_keyboard::KeymapFormat::XkbV1 => {
                        assert!(state.xkb_context.is_none());
                        let xkb_context = unsafe {
                            xkbcommon::Context::from_fd(state.xkbcommon_lib, fd.as_fd(), size)
                                .expect("failed to create xkb context")
                        };
                        state.xkb_context.replace(xkb_context);
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
                let Some(xkb_context) = state.xkb_context.as_mut() else {
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
                let Some(xkb_context) = state.xkb_context.as_mut() else {
                    return;
                };
                let scancode = Scancode::from_int(key);
                state.events.push_back(Event::Keyboard(KeyboardEvent {
                    kind: match key_state {
                        WEnum::Value(value) => {
                            if value == KeyState::Pressed {
                                KeyboardEventKind::Press { scancode }
                            } else {
                                KeyboardEventKind::Release { scancode }
                            }
                        }
                        _ => unreachable!(),
                    },
                    // TODO: surface id (focus tracker)
                    surface_id: 0,
                    mods: xkb_context.mods.clone(),
                }))
            }
            _ => {}
        }
    }
}

delegate_noop!(App: ignore WpFractionalScaleManagerV1);
delegate_noop!(App: ignore WpViewport);
delegate_noop!(App: ignore WpViewporter);
delegate_noop!(App: ignore ZwlrLayerShellV1);
delegate_noop!(App: ignore ZwlrScreencopyManagerV1);
delegate_noop!(App: ignore ZwpLinuxBufferParamsV1);
delegate_noop!(App: ignore ZwpLinuxDmabufV1);
delegate_noop!(App: ignore WlBuffer);
delegate_noop!(App: ignore WlCompositor);
delegate_noop!(App: ignore WlOutput);
delegate_noop!(App: ignore WlSurface);

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

        screens: vec![],
        xkb_context: None,

        events: VecDeque::new(),
        draw_buffer: DrawBuffer::default(),
        renderer: unsafe { Renderer::new(gl_lib)? },

        quit: false,
    };

    event_queue.roundtrip(&mut app)?;

    app.capture_all_screens(&mut event_queue, &qhandle)?;
    app.overlay_all_screens(&mut event_queue, &qhandle)?;

    while !app.quit {
        event_queue.blocking_dispatch(&mut app)?;

        while let Some(event) = app.events.pop_front() {
            match event {
                Event::Keyboard(keyboard_event) => match keyboard_event.kind {
                    KeyboardEventKind::Press {
                        scancode: Scancode::Esc,
                    } => app.quit = true,
                    _ => {}
                },
            }
        }
    }

    Ok(())
}
