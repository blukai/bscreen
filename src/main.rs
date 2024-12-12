use std::{
    collections::VecDeque,
    ffi::c_int,
    os::fd::{AsFd, BorrowedFd},
};

use anyhow::{anyhow, Context};
use egl::{Egl, EglContext, EglImageKhr, EglWindowSurface};
use gl::{Gl, GlTexture2D};
use input::{Event, KeyboardEvent, KeyboardEventKind, Scancode};
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_callback, wl_compositor,
        wl_keyboard::{self, KeyState},
        wl_output, wl_registry, wl_seat, wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_egl::WlEglSurface as WlEglWindow;
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};
use wayland_protocols_wlr::{
    layer_shell::v1::client::{
        zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
    },
    screencopy::v1::client::{
        zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
        zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    },
};
use xkbcommon::{XkbContext, Xkbcommon};

mod dynlib;
mod egl;
mod gfx;
mod gl;
mod input;
mod xkbcommon;

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
    _gl_texture: GlTexture2D,
    _egl_image_khr: EglImageKhr,
    wl_buffer: wl_buffer::WlBuffer,
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
            GlTexture2D::new(
                app.gl,
                width,
                height,
                match format {
                    DRM_FORMAT_XRGB8888 => gfx::TextureFormat::Bgra8Unorm,
                    format => unimplemented!("unhandled fourcc format {format}"),
                },
                None,
            )
        };
        let egl_image_khr = unsafe { EglImageKhr::new(app.egl, app.egl_context, &gl_texture)? };

        let mut fourcc: c_int = 0;
        let mut num_planes: c_int = 0;
        let mut modifiers: egl::sys::types::EGLuint64KHR = 0;
        if unsafe {
            app.egl.ExportDMABUFImageQueryMESA(
                app.egl_context.display,
                egl_image_khr.handle,
                &mut fourcc,
                &mut num_planes,
                &mut modifiers,
            )
        } == egl::sys::FALSE
        {
            return Err(app.egl.unwrap_err()).context("could not retrieve pixel format");
        }
        // TODO: can there me other number of planes?
        assert!(num_planes == 1);

        let mut fd: c_int = 0;
        let mut stride: egl::sys::types::EGLint = 0;
        let mut offset: egl::sys::types::EGLint = 0;
        if unsafe {
            app.egl.ExportDMABUFImageMESA(
                app.egl_context.display,
                egl_image_khr.handle,
                &mut fd,
                &mut stride,
                &mut offset,
            )
        } == egl::sys::FALSE
        {
            return Err(app.egl.unwrap_err()).context("could not retrieve dmabuf fd");
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
            _gl_texture: gl_texture,
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
    output: wl_output::WlOutput,
    screencopy: Option<Screencopy>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    layer_surface_configured: bool,
    egl_window: Option<WlEglWindow>,
    egl_window_surface: Option<EglWindowSurface>,
}

impl Screen {
    fn draw(&self, egl_context: &'static EglContext, gl: &'static Gl) -> anyhow::Result<()> {
        let Some(egl_window_surface) = self.egl_window_surface.as_ref() else {
            unreachable!()
        };

        unsafe {
            egl_context.make_current(egl_window_surface.handle)?;
            gl.ClearColor(1.0, 0.0, 0.0, 0.5);
            gl.Clear(gl::sys::COLOR_BUFFER_BIT);
            egl_context.swap_buffers(egl_window_surface.handle)?;
        }

        Ok(())
    }
}

struct App {
    egl: &'static Egl,
    egl_context: &'static EglContext,
    gl: &'static Gl,
    xkbcommon: &'static Xkbcommon,

    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<ZwlrLayerShellV1>,
    linux_dmabuf: Option<ZwpLinuxDmabufV1>,
    screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    seat: Option<wl_seat::WlSeat>,

    quit: bool,
    screens: Vec<Screen>,
    xkb_context: Option<XkbContext>,
    events: VecDeque<Event>,
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

        for (idx, screen) in self.screens.iter_mut().enumerate() {
            assert!(screen.surface.is_none());
            assert!(screen.layer_surface.is_none());
            assert!(!screen.layer_surface_configured);

            let surface = compositor.create_surface(qhandle, ());
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

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: <wl_registry::WlRegistry as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                interface,
                name,
                version,
            } => match interface.as_str() {
                "wl_output" => {
                    state.screens.push(Screen {
                        output: registry.bind(name, version, qhandle, ()),
                        screencopy: None,
                        surface: None,
                        layer_surface: None,
                        layer_surface_configured: false,
                        egl_window: None,
                        egl_window_surface: None,
                    });
                }
                "zwlr_screencopy_manager_v1" => {
                    state
                        .screencopy_manager
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                "zwp_linux_dmabuf_v1" => {
                    state
                        .linux_dmabuf
                        .replace(registry.bind(name, version, qhandle, ()));
                }
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
                "wl_seat" => {
                    state
                        .seat
                        .replace(registry.bind(name, version, qhandle, ()));
                }
                _ => {
                    log::debug!("unused interface {interface}");
                }
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
        log::debug!("zwlr_screencopy_frame_v1::Event::{event:?}");

        let idx = *data;
        let Some(mut screencopy) = state.screens[idx].screencopy.take() else {
            unreachable!();
        };

        use zwlr_screencopy_frame_v1::Event::*;
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

impl Dispatch<ZwlrLayerSurfaceV1, usize> for App {
    fn event(
        state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: <ZwlrLayerSurfaceV1 as Proxy>::Event,
        data: &usize,
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        log::debug!("zwlr_layer_surface_v1::Event::{event:?}");

        let idx = *data;
        let screen = &mut state.screens[idx];

        use zwlr_layer_surface_v1::Event::*;
        match event {
            Configure {
                serial,
                width,
                height,
            } => {
                proxy.ack_configure(serial);

                let Some(surface) = screen.surface.as_ref() else {
                    unreachable!()
                };

                let egl_window = WlEglWindow::new(surface.id(), width as _, height as _)
                    .expect("failed to create wl egl window");
                let egl_window_surface = unsafe {
                    EglWindowSurface::new(state.egl, state.egl_context, egl_window.ptr())
                        .expect("failed to create egl window surface")
                };

                screen.layer_surface_configured = true;
                screen.egl_window.replace(egl_window);
                screen.egl_window_surface.replace(egl_window_surface);

                screen
                    .draw(state.egl_context, state.gl)
                    .expect("failed to draw");

                surface.frame(qhandle, idx);
            }
            Closed => unimplemented!(),
            _ => unreachable!(),
        }
    }
}

impl Dispatch<wl_callback::WlCallback, usize> for App {
    fn event(
        state: &mut Self,
        _proxy: &wl_callback::WlCallback,
        event: <wl_callback::WlCallback as Proxy>::Event,
        data: &usize,
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        let idx = *data;
        let screen = &mut state.screens[idx];

        use wl_callback::Event::*;
        match event {
            Done { .. } => {
                let Some(surface) = screen.surface.as_ref() else {
                    unreachable!()
                };

                screen
                    .draw(state.egl_context, state.gl)
                    .expect("failed to draw");

                surface.frame(qhandle, idx);
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        state: &mut Self,
        proxy: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        log::debug!("wl_seat::Event::{event:?}");

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

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        log::debug!("wl_keyboard::Event::{event:?}");

        use wl_keyboard::Event::*;
        match event {
            Keymap { format, fd, size } => match format {
                WEnum::Value(value) => match value {
                    wl_keyboard::KeymapFormat::NoKeymap => {
                        log::warn!("non-xkb keymap");
                    }
                    wl_keyboard::KeymapFormat::XkbV1 => {
                        assert!(state.xkb_context.is_none());
                        let xkb_context = unsafe {
                            XkbContext::from_fd(state.xkbcommon, fd.as_fd(), size)
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

delegate_noop!(App: ignore ZwlrLayerShellV1);
delegate_noop!(App: ignore ZwlrScreencopyManagerV1);
delegate_noop!(App: ignore ZwpLinuxBufferParamsV1);
delegate_noop!(App: ignore ZwpLinuxDmabufV1);
delegate_noop!(App: ignore wl_buffer::WlBuffer);
delegate_noop!(App: ignore wl_output::WlOutput);
delegate_noop!(App: ignore wl_surface::WlSurface);
delegate_noop!(App: ignore wl_compositor::WlCompositor);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let connection = Connection::connect_to_env()?;
    let mut event_queue = connection.new_event_queue();
    let qhandle = event_queue.handle();

    let display = connection.display();
    let _registry = display.get_registry(&qhandle, ());

    // this is not super rust'y, but i don't care. egl lib is not going to run away from here.
    let egl_ = unsafe { Egl::load()? };
    let egl: &'static Egl = unsafe { std::mem::transmute(&egl_) };
    let egl_context_ = unsafe { EglContext::create(egl, connection.backend().display_ptr() as _)? };
    let egl_context: &'static EglContext = unsafe { std::mem::transmute(&egl_context_) };
    let gl_ = unsafe { Gl::load(egl) };
    let gl: &'static Gl = unsafe { std::mem::transmute(&gl_) };
    let xkbcommon_ = unsafe { Xkbcommon::load()? };
    let xkbcommon: &'static Xkbcommon = unsafe { std::mem::transmute(&xkbcommon_) };

    let mut app = App {
        egl,
        egl_context,
        gl,
        xkbcommon,

        compositor: None,
        layer_shell: None,
        linux_dmabuf: None,
        screencopy_manager: None,
        seat: None,

        quit: false,
        screens: vec![],
        xkb_context: None,
        events: VecDeque::new(),
    };

    event_queue.roundtrip(&mut app)?;

    app.capture_all_screens(&mut event_queue, &qhandle)?;
    app.overlay_all_screens(&mut event_queue, &qhandle)?;

    while !app.quit {
        event_queue.blocking_dispatch(&mut app)?;

        while let Some(event) = app.events.pop_front() {
            match event {
                Event::Keyboard(keyboard_event) => match keyboard_event.kind {
                    KeyboardEventKind::Press { scancode } if scancode == Scancode::Esc => {
                        app.quit = true;
                    }
                    _ => {}
                },
            }
        }
    }

    Ok(())
}
