use std::collections::HashMap;

use glam::Vec2;

// https://github.com/torvalds/linux/blob/231825b2e1ff6ba799c5eaf396d3ab2354e37c6b/include/uapi/linux/input-event-codes.h#L76

const KEY_ESC: u32 = 1;
const KEY_C: u32 = 46;

const BTN_LEFT: u32 = 0x110;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scancode {
    Esc,
    C,
    Unidentified(u32),
}

impl Scancode {
    pub fn from_int(int: u32) -> Scancode {
        match int {
            KEY_ESC => Self::Esc,
            KEY_C => Self::C,
            _ => Self::Unidentified(int),
        }
    }
}

// TODO: in zig this would have been packed struct(u8), but rust is rust.
#[derive(Debug, Clone)]
pub struct KeyboardMods {
    pub ctrl: bool,
}

#[derive(Debug, PartialEq)]
pub enum KeyboardEventKind {
    Press { scancode: Scancode },
    Release { scancode: Scancode },
}

#[derive(Debug)]
pub struct KeyboardEvent {
    pub kind: KeyboardEventKind,
    pub surface_id: u64,
    pub mods: KeyboardMods,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerButton {
    Left,
    Unidentified(u32),
}

impl PointerButton {
    pub fn from_int(int: u32) -> Self {
        match int {
            BTN_LEFT => Self::Left,
            _ => Self::Unidentified(int),
        }
    }
}

// TODO: same as Mods.. maybe try to pack it?
#[derive(Debug, Clone, Default)]
pub struct PointerButtons {
    pub left: bool,
}

#[derive(Debug, PartialEq)]
pub enum PointerEventKind {
    Motion { delta: Vec2 },
    Press { button: PointerButton },
    Release { button: PointerButton },
}

#[derive(Debug)]
pub struct PointerEvent {
    pub kind: PointerEventKind,
    pub surface_id: u64,
    pub position: Vec2,
    pub buttons: PointerButtons,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorShape {
    Default,
    Crosshair,
    Move,
    NwResize,
    NeResize,
    SeResize,
    SwResize,
}

impl CursorShape {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Crosshair => "crosshair",
            Self::Move => "move",
            Self::NwResize => "nw-resize",
            Self::NeResize => "ne-resize",
            Self::SeResize => "se-resize",
            Self::SwResize => "sw-resize",
        }
    }
}

#[derive(Debug)]
pub enum Event {
    Keyboard(KeyboardEvent),
    Pointer(PointerEvent),
}

#[derive(PartialEq, Eq, Hash)]
pub enum SerialType {
    KeyboardEnter,
    PointerEnter,
}

#[derive(Default)]
pub struct SerialTracker {
    serial_map: HashMap<SerialType, u32>,
}

impl SerialTracker {
    pub fn update_serial(&mut self, ty: SerialType, serial: u32) {
        self.serial_map.insert(ty, serial);
    }

    pub fn reset_serial(&mut self, ty: SerialType) {
        self.serial_map.remove(&ty);
    }

    pub fn get_serial(&self, ty: SerialType) -> Option<u32> {
        self.serial_map.get(&ty).cloned()
    }
}
