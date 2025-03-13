use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr::NonNull,
    rc::Rc,
};

use anyhow::{Context as _, anyhow};

use crate::{Connection, wayland};

struct ClipboardDataOffer {
    mime_type: CString,
    data: Vec<u8>,
}

pub struct Clipboard {
    conn: Rc<Connection>,

    data_device: Option<NonNull<wayland::wl_data_device>>,
    data_source: Option<NonNull<wayland::wl_data_source>>,
    data_offer: Option<ClipboardDataOffer>,

    pub cancelled: bool,
}

unsafe fn write_all(fd: c_int, buf: *const c_void, count: libc::size_t) -> anyhow::Result<()> {
    let mut pos = 0;
    while pos < count {
        let n = libc::write(fd, buf.byte_add(pos) as _, count - pos);
        if n < 0 {
            let errno = *libc::__errno_location();
            if errno == libc::EAGAIN {
                continue;
            }
            return Err(anyhow!("could not write, errno {}", errno));
        }
        pos += n as usize;
    }

    Ok(())
}

unsafe extern "C" fn handle_send(
    data: *mut c_void,
    _wl_data_source: *mut wayland::wl_data_source,
    mime_type: *const c_char,
    fd: i32,
) {
    log::debug!("wl_data_source.send");

    let clipboard = &mut *(data as *mut Clipboard);
    let data_offer = clipboard
        .data_offer
        .as_ref()
        .expect("data offer is missing huh?");

    // TODO: can we receive request for other mime, not the one that was
    // offered? probably not?
    let mime_type = CStr::from_ptr(mime_type);
    assert!(data_offer.mime_type.as_ref().eq(mime_type));

    if let Err(err) = write_all(fd, data_offer.data.as_ptr() as _, data_offer.data.len()) {
        log::error!("write_all failed: {err:?}");
        // do not do early return, fd must be closed.
    }
    libc::close(fd);
}

unsafe extern "C" fn handle_cancelled(
    data: *mut c_void,
    _wl_data_source: *mut wayland::wl_data_source,
) {
    log::debug!("wl_data_source.cancelled");

    let clipboard = &mut *(data as *mut Clipboard);
    if let Some(data_device) = clipboard.data_device.take() {
        wayland::wl_data_device_release(clipboard.conn.libs.wayland, data_device.as_ptr());
    }
    if let Some(data_source) = clipboard.data_source.take() {
        wayland::wl_data_source_destroy(clipboard.conn.libs.wayland, data_source.as_ptr());
    }
    _ = clipboard.data_offer.take();

    clipboard.cancelled = true;
}

const WL_DATA_SOURCE_LISTENER: wayland::wl_data_source_listener =
    wayland::wl_data_source_listener {
        target: wayland::noop_listener!(),
        send: handle_send,
        cancelled: handle_cancelled,
        dnd_drop_performed: wayland::noop_listener!(),
        dnd_finished: wayland::noop_listener!(),
        action: wayland::noop_listener!(),
    };

impl Clipboard {
    pub fn new_boxed(conn: &Rc<Connection>) -> Box<Self> {
        Box::new(Self {
            conn: Rc::clone(conn),

            data_device: None,
            data_source: None,
            data_offer: None,

            cancelled: false,
        })
    }

    pub fn offer_data(
        &mut self,
        serial: u32,
        mime_type: String,
        data: Vec<u8>,
    ) -> anyhow::Result<()> {
        let mime_type = CString::new(mime_type)?;

        let data_device_manager = self
            .conn
            .globals
            .data_device_manager
            .context("data device manager is not available")?;

        if self.data_device.is_none() {
            self.data_device = Some(
                NonNull::new(unsafe {
                    wayland::wl_data_device_manager_get_data_device(
                        self.conn.libs.wayland,
                        data_device_manager,
                        self.conn.globals.seat.context("seat is not available")?,
                    )
                })
                .context("could not get data device")?,
            );
        }
        let data_device = self.data_device.unwrap();

        let data_source = NonNull::new(unsafe {
            wayland::wl_data_device_manager_create_data_source(
                self.conn.libs.wayland,
                data_device_manager,
            )
        })
        .context("could not create data source")?;

        unsafe {
            wayland::wl_data_source_offer(
                self.conn.libs.wayland,
                data_source.as_ptr(),
                mime_type.as_ptr(),
            );
            (self.conn.libs.wayland.wl_proxy_add_listener)(
                data_source.as_ptr() as *mut wayland::wl_proxy,
                &WL_DATA_SOURCE_LISTENER as *const wayland::wl_data_source_listener as _,
                self as *mut Self as *mut c_void,
            );
        }

        unsafe {
            wayland::wl_data_device_set_selection(
                self.conn.libs.wayland,
                data_device.as_ptr(),
                data_source.as_ptr(),
                serial,
            );
            (self.conn.libs.wayland.wl_display_flush)(self.conn.libs.wayland_display.as_ptr());
        }

        self.data_offer = Some(ClipboardDataOffer { mime_type, data });

        Ok(())
    }
}
