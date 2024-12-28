use std::{
    collections::VecDeque,
    ffi::{CString, c_char, c_void},
    ptr::NonNull,
    rc::Rc,
};

use anyhow::{Context, anyhow};
use glam::Vec2;

use crate::{
    Connection,
    input::{
        CursorShape, Event, KeyboardEvent, KeyboardEventKind, PointerButton, PointerButtons,
        PointerEvent, PointerEventKind, Scancode, SerialTracker, SerialType,
    },
    wayland, wayland_cursor, xkbcommon,
};

pub fn get_surface_id(surface: NonNull<wayland::wl_surface>) -> u64 {
    surface.as_ptr() as u64
}

pub struct Input {
    conn: Rc<Connection>,

    keyboard: NonNull<wayland::wl_keyboard>,
    xkb_context: Option<xkbcommon::Context>,
    keyboard_focused_surface_id: Option<u64>,

    pointer: NonNull<wayland::wl_pointer>,
    pointer_position: Vec2,
    pointer_focused_surface_id: Option<u64>,
    pointer_buttons: PointerButtons,
    pointer_frame_events: VecDeque<PointerEvent>,
    cursor_theme: NonNull<wayland_cursor::wl_cursor_theme>,
    cursor_surface: NonNull<wayland::wl_surface>,

    pub serial_tracker: SerialTracker,
    pub events: VecDeque<Event>,
}

unsafe extern "C" fn handle_keyboard_keymap(
    data: *mut c_void,
    _wl_keyboard: *mut wayland::wl_keyboard,
    format: u32,
    fd: i32,
    size: u32,
) {
    log::debug!("wl_keyboard.keymap");

    let input = &mut *(data as *mut Input);
    match format {
        wayland::WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1 => {
            assert!(input.xkb_context.is_none());
            let xkb_context = xkbcommon::Context::from_fd(input.conn.libs.xkbcommon, fd, size)
                .expect("failed to create xkb context");
            input.xkb_context = Some(xkb_context);
            log::info!("created xkb context");
        }
        wayland::WL_KEYBOARD_KEYMAP_FORMAT_NO_KEYMAP => {
            unimplemented!("unsupported keymap format {format}")
        }
        _ => unreachable!("unknown keymap format {format}"),
    }
}

unsafe extern "C" fn handle_keyboard_enter(
    data: *mut c_void,
    _wl_keyboard: *mut wayland::wl_keyboard,
    serial: u32,
    surface: *mut wayland::wl_surface,
    _keys: *mut wayland::wl_array,
) {
    log::debug!("wl_keyboard.enter");

    let Some(surface) = NonNull::new(surface) else {
        log::warn!("recieved keyboard enter event with null surface");
        return;
    };

    let input = &mut *(data as *mut Input);
    input.keyboard_focused_surface_id = Some(get_surface_id(surface));
    input
        .serial_tracker
        .update_serial(SerialType::KeyboardEnter, serial);
}

unsafe extern "C" fn handle_keyboard_leave(
    data: *mut c_void,
    _wl_keyboard: *mut wayland::wl_keyboard,
    _serial: u32,
    _surface: *mut wayland::wl_surface,
) {
    log::debug!("wl_keyboard.leave");

    let input = &mut *(data as *mut Input);
    input.keyboard_focused_surface_id = None;
    input.serial_tracker.reset_serial(SerialType::KeyboardEnter);
}

unsafe extern "C" fn handle_keyboard_key(
    data: *mut c_void,
    _wl_keyboard: *mut wayland::wl_keyboard,
    _serial: u32,
    _time: u32,
    key: u32,
    state: u32,
) {
    log::debug!("wl_keyboard.key");

    let input = &mut *(data as *mut Input);
    assert!(input.xkb_context.is_some());
    assert!(input.keyboard_focused_surface_id.is_some());

    let scancode = Scancode::from_int(key);
    let keyboard_event = KeyboardEvent {
        kind: match state {
            wayland::WL_KEYBOARD_KEY_STATE_PRESSED => KeyboardEventKind::Press { scancode },
            wayland::WL_KEYBOARD_KEY_STATE_RELEASED => KeyboardEventKind::Release { scancode },
            _ => unreachable!("unsupported key state {state}"),
        },
        surface_id: input.keyboard_focused_surface_id.unwrap(),
        mods: input.xkb_context.as_ref().unwrap().mods.clone(),
    };
    input.events.push_back(Event::Keyboard(keyboard_event));
}

unsafe extern "C" fn handle_keyboard_modifiers(
    data: *mut c_void,
    _wl_keyboard: *mut wayland::wl_keyboard,
    _serial: u32,
    mods_depressed: u32,
    mods_latched: u32,
    mods_locked: u32,
    group: u32,
) {
    log::debug!("wl_keyboard.modifiers");

    let input = &mut *(data as *mut Input);
    assert!(input.xkb_context.is_some());

    input.xkb_context.as_mut().unwrap().update_mods(
        mods_depressed,
        mods_latched,
        mods_locked,
        0,
        0,
        group,
    );
}

const WL_KEYBOARD_LISTENER: wayland::wl_keyboard_listener = wayland::wl_keyboard_listener {
    keymap: handle_keyboard_keymap,
    enter: handle_keyboard_enter,
    leave: handle_keyboard_leave,
    key: handle_keyboard_key,
    modifiers: handle_keyboard_modifiers,
    repeat_info: wayland::noop_listener!(),
};

unsafe extern "C" fn handle_pointer_enter(
    data: *mut c_void,
    _wl_pointer: *mut wayland::wl_pointer,
    serial: u32,
    surface: *mut wayland::wl_surface,
    _surface_x: wayland::wl_fixed,
    _surface_y: wayland::wl_fixed,
) {
    log::debug!("wl_pointer.enter");

    let Some(surface) = NonNull::new(surface) else {
        log::warn!("recieved pointer enter event with null surface");
        return;
    };

    let input = &mut *(data as *mut Input);
    input.pointer_focused_surface_id = Some(get_surface_id(surface));
    input
        .serial_tracker
        .update_serial(SerialType::PointerEnter, serial);
}

unsafe extern "C" fn handle_pointer_leave(
    data: *mut c_void,
    _wl_pointer: *mut wayland::wl_pointer,
    _serial: u32,
    _surface: *mut wayland::wl_surface,
) {
    log::debug!("wl_pointer.leave");

    let input = &mut *(data as *mut Input);
    input.pointer_focused_surface_id = None;
    input.serial_tracker.reset_serial(SerialType::PointerEnter);
}

unsafe extern "C" fn handle_pointer_motion(
    data: *mut c_void,
    _wl_pointer: *mut wayland::wl_pointer,
    _time: u32,
    surface_x: wayland::wl_fixed,
    surface_y: wayland::wl_fixed,
) {
    log::trace!("wl_pointer.motion");

    let input = &mut *(data as *mut Input);
    assert!(input.pointer_focused_surface_id.is_some());

    let prev_position = input.pointer_position;
    let next_position = Vec2::new(
        wayland::wl_fixed_to_f32(surface_x),
        wayland::wl_fixed_to_f32(surface_y),
    );
    input.pointer_position = next_position;

    let frame_event = PointerEvent {
        kind: PointerEventKind::Motion {
            delta: next_position - prev_position,
        },
        surface_id: input.pointer_focused_surface_id.unwrap(),
        position: next_position,
        buttons: input.pointer_buttons.clone(),
    };
    input.pointer_frame_events.push_back(frame_event);
}

unsafe extern "C" fn handle_pointer_button(
    data: *mut c_void,
    _wl_pointer: *mut wayland::wl_pointer,
    _serial: u32,
    _time: u32,
    button: u32,
    state: u32,
) {
    log::debug!("wl_pointer.pointer_button");

    let input = &mut *(data as *mut Input);
    assert!(input.pointer_focused_surface_id.is_some());

    let button = PointerButton::from_int(button);
    let pressed = state == wayland::WL_POINTER_BUTTON_STATE_PRESSED;
    match button {
        PointerButton::Left => input.pointer_buttons.left = pressed,
        _ => {}
    }

    let frame_event = PointerEvent {
        kind: match state {
            wayland::WL_POINTER_BUTTON_STATE_PRESSED => PointerEventKind::Press { button },
            wayland::WL_POINTER_BUTTON_STATE_RELEASED => PointerEventKind::Release { button },
            _ => unreachable!("unknown pointer button state {state}"),
        },
        surface_id: input.pointer_focused_surface_id.unwrap(),
        position: input.pointer_position,
        buttons: input.pointer_buttons.clone(),
    };
    input.pointer_frame_events.push_back(frame_event);
}

unsafe extern "C" fn handle_pointer_frame(
    data: *mut c_void,
    _wl_pointer: *mut wayland::wl_pointer,
) {
    log::trace!("wl_pointer.frame");

    let input = &mut *(data as *mut Input);
    input
        .events
        .extend(input.pointer_frame_events.drain(..).map(Event::Pointer));
}

const WL_POINTER_LISTENER: wayland::wl_pointer_listener = wayland::wl_pointer_listener {
    enter: handle_pointer_enter,
    leave: handle_pointer_leave,
    motion: handle_pointer_motion,
    button: handle_pointer_button,
    axis: wayland::noop_listener!(),
    frame: handle_pointer_frame,
    axis_source: wayland::noop_listener!(),
    axis_stop: wayland::noop_listener!(),
    axis_discrete: wayland::noop_listener!(),
    axis_value120: wayland::noop_listener!(),
    axis_relative_direction: wayland::noop_listener!(),
};

impl Input {
    pub fn new_boxed(conn: &Rc<Connection>) -> anyhow::Result<Box<Self>> {
        let mut uninit = Box::<Self>::new_uninit();

        let seat = conn.globals.seat.context("seat is not available")?;

        let keyboard =
            NonNull::new(unsafe { wayland::wl_seat_get_keyboard(conn.libs.wayland, seat) })
                .context("could not get keyboard")?;
        unsafe {
            (conn.libs.wayland.wl_proxy_add_listener)(
                keyboard.as_ptr() as *mut wayland::wl_proxy,
                &WL_KEYBOARD_LISTENER as *const wayland::wl_keyboard_listener as _,
                uninit.as_mut_ptr() as *mut c_void,
            );
        }

        let pointer =
            NonNull::new(unsafe { wayland::wl_seat_get_pointer(conn.libs.wayland, seat) })
                .context("could not get pointer")?;
        unsafe {
            (conn.libs.wayland.wl_proxy_add_listener)(
                pointer.as_ptr() as *mut wayland::wl_proxy,
                &WL_POINTER_LISTENER as *const wayland::wl_pointer_listener as _,
                uninit.as_mut_ptr() as *mut c_void,
            );
        }

        // NOTE: it seems like people on the internet default to 24.
        //
        // TODO: do i need to take scale (/fractional scaling) into account?
        let cursor_theme = NonNull::new(unsafe {
            (conn.libs.wayland_cursor.wl_cursor_theme_load)(
                "default\0".as_ptr() as *const c_char,
                24,
                conn.globals.shm.context("shm is not available")?,
            )
        })
        .context("could not get cursor theme")?;
        let cursor_surface = NonNull::new(unsafe {
            wayland::wl_compositor_create_surface(
                conn.libs.wayland,
                conn.globals
                    .compositor
                    .context("compositor is not available")?,
            )
        })
        .context("could not create cursor surface")?;

        uninit.write(Self {
            conn: Rc::clone(conn),

            keyboard,
            xkb_context: None,
            keyboard_focused_surface_id: None,

            pointer,
            pointer_position: Vec2::ZERO,
            pointer_focused_surface_id: None,
            pointer_buttons: PointerButtons::default(),
            pointer_frame_events: VecDeque::new(),
            cursor_theme,
            cursor_surface,

            serial_tracker: SerialTracker::default(),
            events: VecDeque::new(),
        });

        Ok(unsafe { uninit.assume_init() })
    }

    pub fn set_cursor_shape(&self, cursor_shape: CursorShape) -> anyhow::Result<()> {
        let Some(serial) = self.serial_tracker.get_serial(SerialType::PointerEnter) else {
            log::warn!("no pointer enter serial found");
            return Ok(());
        };

        let cursor_name = CString::new(cursor_shape.name())?;
        let cursor = unsafe {
            (self.conn.libs.wayland_cursor.wl_cursor_theme_get_cursor)(
                self.cursor_theme.as_ptr(),
                cursor_name.as_ptr(),
            )
        };
        if cursor.is_null() {
            log::warn!("could not find {} cursor", cursor_shape.name());
            return Ok(());
        };
        let cursor = unsafe { &*cursor };

        let cursor_images =
            unsafe { std::slice::from_raw_parts(cursor.images, cursor.image_count as usize) };
        let cursor_image_ptr = cursor_images[0];
        let cursor_image = unsafe { &*cursor_image_ptr };

        let cursor_image_buffer =
            unsafe { (self.conn.libs.wayland_cursor.wl_cursor_image_get_buffer)(cursor_image_ptr) };
        if cursor_image_buffer.is_null() {
            return Err(anyhow!("could not get cursor image buffer"));
        }

        unsafe {
            wayland::wl_surface_attach(
                self.conn.libs.wayland,
                self.cursor_surface.as_ptr(),
                cursor_image_buffer,
                0,
                0,
            );

            // NOTE: pre version 4 wl_surface::damage must be used instead.
            let wl_surface_version = (self.conn.libs.wayland.wl_proxy_get_version)(
                self.cursor_surface.as_ptr() as *mut wayland::wl_proxy,
            );
            assert!(wl_surface_version >= 4);
            wayland::wl_surface_damage_buffer(
                self.conn.libs.wayland,
                self.cursor_surface.as_ptr(),
                0,
                0,
                cursor_image.width as i32,
                cursor_image.height as i32,
            );

            wayland::wl_surface_commit(self.conn.libs.wayland, self.cursor_surface.as_ptr());

            wayland::wl_pointer_set_cursor(
                self.conn.libs.wayland,
                self.pointer.as_ptr(),
                serial,
                self.cursor_surface.as_ptr(),
                cursor_image.hotspot_x as i32,
                cursor_image.hotspot_y as i32,
            );
        }

        Ok(())
    }
}
