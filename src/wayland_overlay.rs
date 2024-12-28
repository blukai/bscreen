use std::{
    ffi::{c_char, c_int, c_void},
    ptr::NonNull,
    rc::Rc,
};

use anyhow::{Context, anyhow};

use crate::{Connection, egl, gfx::Size, wayland, wayland_egl};

// TODO: maybe turn overlay into an enum with Configured/Unconfigured variants.

pub struct Overlay {
    conn: Rc<Connection>,
    output: NonNull<wayland::wl_output>,

    pub surface: NonNull<wayland::wl_surface>,
    pub layer_surface: Option<NonNull<wayland::zwlr_layer_surface_v1>>,
    viewport: NonNull<wayland::wp_viewport>,

    pub fractional_scale: Option<f64>,
    pub logical_size: Option<Size>,

    pub acked_first_configure: bool,
    window: Option<*mut wayland_egl::wl_egl_window>,
    pub window_surface: Option<egl::WindowSurface>,
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if let Some(window_surface) = self.window_surface.take() {
            drop(window_surface);
        }

        if let Some(window) = self.window.take() {
            unsafe {
                (self.conn.libs.wayland_egl.wl_egl_window_destroy)(window);
            }
        }

        unsafe {
            wayland::wp_viewport_destroy(self.conn.libs.wayland, self.viewport.as_ptr());
        }

        if let Some(layer_surface) = self.layer_surface.take() {
            unsafe {
                wayland::zwlr_layer_surface_v1_destroy(
                    self.conn.libs.wayland,
                    layer_surface.as_ptr(),
                );
            }
        }

        unsafe {
            wayland::wl_surface_destroy(self.conn.libs.wayland, self.surface.as_ptr());
        }
    }
}

unsafe extern "C" fn handle_preferred_scale(
    data: *mut c_void,
    _wp_fractional_scale_v1: *mut wayland::wp_fractional_scale_v1,
    scale: u32,
) {
    log::debug!("wp_fractional_scale_v1.preferred_scale");

    let overlay = &mut *(data as *mut Overlay);
    // > The sent scale is the numerator of a fraction with a denominator of 120.
    let fractional_scale = scale as f64 / 120.0;
    overlay
        .configure(Some(fractional_scale), None)
        .expect("failed to configure overlay");
}

const WP_FRACTIONAL_SCALE_V1_LISTENER: wayland::wp_fractional_scale_v1_listener =
    wayland::wp_fractional_scale_v1_listener {
        preferred_scale: handle_preferred_scale,
    };

unsafe extern "C" fn handle_configure(
    data: *mut c_void,
    zwlr_layer_surface_v1: *mut wayland::zwlr_layer_surface_v1,
    serial: u32,
    width: u32,
    height: u32,
) {
    log::debug!("zwlr_layer_surface_v1.configure");

    let overlay = &mut *(data as *mut Overlay);

    wayland::zwlr_layer_surface_v1_ack_configure(
        overlay.conn.libs.wayland,
        zwlr_layer_surface_v1,
        serial,
    );
    overlay.acked_first_configure = true;

    // TODO: this is not the actual logical size of the screen. must get it from xdg shell or
    // something instead.
    let logical_size = Size::new(width, height);

    overlay
        .configure(None, Some(logical_size))
        .expect("failed to configure overlay");
}

unsafe extern "C" fn handle_closed(
    _data: *mut c_void,
    _zwlr_layer_surface_v1: *mut wayland::zwlr_layer_surface_v1,
) {
    log::debug!("zwlr_layer_surface_v1.closed");
    unimplemented!();
}

const ZWLR_LAYER_SURFACE_V1_LISTENER: wayland::zwlr_layer_surface_v1_listener =
    wayland::zwlr_layer_surface_v1_listener {
        configure: handle_configure,
        closed: handle_closed,
    };

impl Overlay {
    pub fn new_boxed(
        conn: &Rc<Connection>,
        output: NonNull<wayland::wl_output>,
    ) -> anyhow::Result<Box<Self>> {
        let compositor = conn
            .globals
            .compositor
            .context("compositor is not available")?;
        let surface = NonNull::new(unsafe {
            wayland::wl_compositor_create_surface(conn.libs.wayland, compositor)
        })
        .context("could not create surface")?;

        let mut uninit = Box::<Self>::new_uninit();

        let fractional_scale_manager = conn
            .globals
            .fractional_scale_manager
            .context("fractional scale manager is not available")?;
        let fractional_scale = unsafe {
            wayland::wp_fractional_scale_manager_v1_get_fractional_scale(
                conn.libs.wayland,
                fractional_scale_manager,
                surface.as_ptr(),
            )
        };
        if fractional_scale.is_null() {
            return Err(anyhow!("could not get fractioanl scale"));
        }

        unsafe {
            (conn.libs.wayland.wl_proxy_add_listener)(
                fractional_scale as *mut wayland::wl_proxy,
                &WP_FRACTIONAL_SCALE_V1_LISTENER as *const wayland::wp_fractional_scale_v1_listener
                    as _,
                uninit.as_mut_ptr() as *mut c_void,
            );
        }

        let layer_shell = conn
            .globals
            .layer_shell
            .context("layer shell is not available")?;
        let layer_surface = NonNull::new(unsafe {
            wayland::zwlr_layer_shell_v1_get_layer_surface(
                conn.libs.wayland,
                layer_shell,
                surface.as_ptr(),
                output.as_ptr(),
                wayland::ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,
                b"bscreen\0".as_ptr() as *const c_char,
            )
        })
        .context("could not create layer surface")?;

        unsafe {
            (conn.libs.wayland.wl_proxy_add_listener)(
                layer_surface.as_ptr() as *mut wayland::wl_proxy,
                &ZWLR_LAYER_SURFACE_V1_LISTENER as *const wayland::zwlr_layer_surface_v1_listener
                    as _,
                uninit.as_mut_ptr() as *mut c_void,
            );

            wayland::zwlr_layer_surface_v1_set_anchor(
                conn.libs.wayland,
                layer_surface.as_ptr(),
                wayland::ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP
                    | wayland::ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT
                    | wayland::ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM
                    | wayland::ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT,
            );
            // > If set to -1, the surface indicates that it would not like to be moved to
            // accommodate for other surfaces, and the compositor should extend it all the way to
            // the edges it is anchored to.
            wayland::zwlr_layer_surface_v1_set_exclusive_zone(
                conn.libs.wayland,
                layer_surface.as_ptr(),
                -1,
            );
            wayland::zwlr_layer_surface_v1_set_keyboard_interactivity(
                conn.libs.wayland,
                layer_surface.as_ptr(),
                wayland::ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE,
            );

            // > After creating a layer_surface object and setting it up, the client
            // must perform an initial commit without any buffer attached. The
            // compositor will reply with a layer_surface.configure event.
            wayland::wl_surface_commit(conn.libs.wayland, surface.as_ptr());
        }

        let viewporter = conn
            .globals
            .viewporter
            .context("viewporter is not available")?;
        let viewport = NonNull::new(unsafe {
            wayland::wp_viewporter_get_viewport(conn.libs.wayland, viewporter, surface.as_ptr())
        })
        .context("could not get viewport")?;

        uninit.write(Self {
            conn: Rc::clone(conn),
            output,

            surface,
            layer_surface: Some(layer_surface),
            viewport,

            fractional_scale: None,
            logical_size: None,

            acked_first_configure: false,
            window: None,
            window_surface: None,
        });

        Ok(unsafe { uninit.assume_init() })
    }

    fn configure(
        &mut self,
        fractional_scale: Option<f64>,
        logical_size: Option<Size>,
    ) -> anyhow::Result<()> {
        if fractional_scale.is_some() {
            self.fractional_scale = fractional_scale;
        };
        if logical_size.is_some() {
            self.logical_size = logical_size;
        };

        if !self.acked_first_configure {
            return Ok(());
        }

        let fractional_scale = self.fractional_scale.unwrap_or(1.0);
        let logical_size = self.logical_size.context("logical size is missing?")?;
        let physical_size = logical_size.to_physical(fractional_scale);

        if self.window.is_none() {
            assert!(self.window_surface.is_none());

            let window = unsafe {
                (self.conn.libs.wayland_egl.wl_egl_window_create)(
                    self.surface.as_ptr(),
                    physical_size.width as c_int,
                    physical_size.height as c_int,
                )
            };
            if window.is_null() {
                return Err(anyhow!("could not create wl egl window"));
            }
            self.window = Some(window);

            let window_surface = unsafe {
                egl::WindowSurface::new(
                    self.conn.libs.egl,
                    &self.conn.libs.egl_context,
                    window as egl::sys::types::EGLNativeWindowType,
                )?
            };
            self.window_surface = Some(window_surface);
        }

        unsafe {
            wayland::wp_viewport_set_destination(
                self.conn.libs.wayland,
                self.viewport.as_ptr(),
                logical_size.width as i32,
                logical_size.height as i32,
            );

            wayland::wl_surface_commit(self.conn.libs.wayland, self.surface.as_ptr());
        }

        log::info!(
            "configured overlay with logcial size = {}x{} and fractional scale = {fractional_scale}",
            logical_size.width,
            logical_size.height,
        );

        Ok(())
    }
}
