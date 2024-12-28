use std::{
    ffi::{c_int, c_void},
    ptr::NonNull,
    rc::Rc,
};

use anyhow::{Context as _, anyhow};

use crate::{Connection, egl, gfx, gl, wayland};

const DRM_FORMAT_XRGB8888: u32 = 0x34325258;

pub enum ScreencopyState {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, PartialEq)]
pub struct ScreencopyDmabufDescriptor {
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

pub struct ScreencopyDmabuf {
    pub gl_texture: gl::Texture2D,
    _egl_image_khr: egl::ImageKhr,
    wl_buffer: NonNull<wayland::wl_buffer>,
}

impl ScreencopyDmabuf {
    fn new(conn: &Connection, descriptor: &ScreencopyDmabufDescriptor) -> anyhow::Result<Self> {
        let gl_texture = unsafe {
            gl::Texture2D::new(
                conn.libs.gl,
                descriptor.width,
                descriptor.height,
                match descriptor.format {
                    DRM_FORMAT_XRGB8888 => gfx::TextureFormat::Bgra8Unorm,
                    format => unimplemented!("unhandled fourcc format {format}"),
                },
                None,
            )
        };
        let egl_image_khr =
            unsafe { egl::ImageKhr::new(conn.libs.egl, &conn.libs.egl_context, &gl_texture)? };

        let mut fourcc: c_int = 0;
        let mut num_planes: c_int = 0;
        let mut modifiers: egl::sys::types::EGLuint64KHR = 0;
        if unsafe {
            conn.libs.egl.ExportDMABUFImageQueryMESA(
                conn.libs.egl_context.display,
                egl_image_khr.handle,
                &mut fourcc,
                &mut num_planes,
                &mut modifiers,
            )
        } == egl::sys::FALSE
        {
            return Err(conn.libs.egl.unwrap_err()).context("could not retrieve pixel format");
        }
        // TODO: can there me other number of planes?
        assert!(num_planes == 1);

        let mut fd: c_int = 0;
        let mut stride: egl::sys::types::EGLint = 0;
        let mut offset: egl::sys::types::EGLint = 0;
        if unsafe {
            conn.libs.egl.ExportDMABUFImageMESA(
                conn.libs.egl_context.display,
                egl_image_khr.handle,
                &mut fd,
                &mut stride,
                &mut offset,
            )
        } == egl::sys::FALSE
        {
            return Err(conn.libs.egl.unwrap_err()).context("could not retrieve dmabuf fd");
        }

        let linux_dmabuf = conn
            .globals
            .linux_dmabuf
            .context("linux dmabuf is not available")?;
        let params =
            unsafe { wayland::zwp_linux_dmabuf_v1_create_params(conn.libs.wayland, linux_dmabuf) };
        if params.is_null() {
            return Err(anyhow!("could not create linux dmabuf params"));
        }
        unsafe {
            wayland::zwp_linux_buffer_params_v1_add(
                conn.libs.wayland,
                params,
                fd,
                0,
                offset as u32,
                stride as u32,
                (modifiers >> 32) as u32,
                (modifiers & (u32::MAX as u64)) as u32,
            );
        }
        let wl_buffer = NonNull::new(unsafe {
            wayland::zwp_linux_buffer_params_v1_create_immed(
                conn.libs.wayland,
                params,
                descriptor.width as i32,
                descriptor.height as i32,
                descriptor.format,
                0,
            )
        })
        .context("could not create linux dmabuf buffer")?;

        Ok(Self {
            gl_texture,
            _egl_image_khr: egl_image_khr,
            wl_buffer,
        })
    }
}

pub struct Screencopy {
    conn: Rc<Connection>,
    output: NonNull<wayland::wl_output>,

    pub state: ScreencopyState,
    pub dmabuf_desc: Option<ScreencopyDmabufDescriptor>,
    pub dmabuf: Option<ScreencopyDmabuf>,
}

unsafe extern "C" fn handle_ready(
    data: *mut c_void,
    _zwlr_screencopy_frame_v1: *mut wayland::zwlr_screencopy_frame_v1,
    _tv_sec_hi: u32,
    _tv_sec_lo: u32,
    _tv_nsec: u32,
) {
    log::debug!("zwlr_screencopy_frame_v1_listener.ready");

    let screencopy = &mut *(data as *mut Screencopy);
    screencopy.state = ScreencopyState::Ready;
}

unsafe extern "C" fn handle_failed(
    data: *mut c_void,
    _zwlr_screencopy_frame_v1: *mut wayland::zwlr_screencopy_frame_v1,
) {
    log::debug!("zwlr_screencopy_frame_v1_listener.failed");

    let screencopy = &mut *(data as *mut Screencopy);
    screencopy.state = ScreencopyState::Failed;
}

unsafe extern "C" fn handle_linux_dmabuf(
    data: *mut c_void,
    _zwlr_screencopy_frame_v1: *mut wayland::zwlr_screencopy_frame_v1,
    format: u32,
    width: u32,
    height: u32,
) {
    log::debug!("zwlr_screencopy_frame_v1_listener.linux_dmabuf");

    let screencopy = &mut *(data as *mut Screencopy);

    let next_desc = ScreencopyDmabufDescriptor {
        format,
        width,
        height,
    };
    if screencopy
        .dmabuf_desc
        .as_ref()
        .is_some_and(|prev_desc| prev_desc.eq(&next_desc))
    {
        return;
    }
    screencopy.dmabuf_desc = Some(next_desc);
    _ = screencopy.dmabuf.take();
}

unsafe extern "C" fn handle_buffer_done(
    data: *mut c_void,
    zwlr_screencopy_frame_v1: *mut wayland::zwlr_screencopy_frame_v1,
) {
    log::debug!("zwlr_screencopy_frame_v1_listener.buffer_done");

    let screencopy = &mut *(data as *mut Screencopy);

    let dmabuf = screencopy.dmabuf.get_or_insert_with(|| {
        ScreencopyDmabuf::new(&screencopy.conn, screencopy.dmabuf_desc.as_ref().unwrap())
            .expect("could not create screencopy dmabuf")
    });
    wayland::zwlr_screencopy_frame_v1_copy(
        screencopy.conn.libs.wayland,
        zwlr_screencopy_frame_v1,
        dmabuf.wl_buffer.as_ptr(),
    );
}

const ZWLR_SCREENCOPY_FRAME_V1_LISTENER: wayland::zwlr_screencopy_frame_v1_listener =
    wayland::zwlr_screencopy_frame_v1_listener {
        buffer: wayland::noop_listener!(),
        flags: wayland::noop_listener!(),
        ready: handle_ready,
        failed: handle_failed,
        damage: wayland::noop_listener!(),
        linux_dmabuf: handle_linux_dmabuf,
        buffer_done: handle_buffer_done,
    };

impl Screencopy {
    pub fn new_boxed(conn: &Rc<Connection>, output: NonNull<wayland::wl_output>) -> Box<Self> {
        Box::new(Self {
            conn: Rc::clone(conn),
            output,

            state: ScreencopyState::Pending,
            dmabuf_desc: None,
            dmabuf: None,
        })
    }

    pub unsafe fn capture(&mut self) -> anyhow::Result<()> {
        let screencopy_manager = self
            .conn
            .globals
            .screencopy_manager
            .context("screencopy manager is not available")?;
        let screencopy_frame = wayland::zwlr_screencopy_manager_v1_capture_output(
            self.conn.libs.wayland,
            screencopy_manager,
            // TODO: configurable cursor capture
            1,
            self.output.as_ptr(),
        );
        (self.conn.libs.wayland.wl_proxy_add_listener)(
            screencopy_frame as *mut wayland::wl_proxy,
            &ZWLR_SCREENCOPY_FRAME_V1_LISTENER as *const wayland::zwlr_screencopy_frame_v1_listener
                as _,
            self as *mut Self as *mut c_void,
        );

        Ok(())
    }
}
