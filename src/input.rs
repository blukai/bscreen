use crate::xkbcommon::Mods;

// https://github.com/torvalds/linux/blob/231825b2e1ff6ba799c5eaf396d3ab2354e37c6b/include/uapi/linux/input-event-codes.h#L76
const KEY_ESC: u32 = 1;
const KEY_C: u32 = 46;

#[derive(PartialEq)]
pub enum Scancode {
    Esc,
    C,
    Unidentified(u32),
}

impl Scancode {
    pub fn from_int(int: u32) -> Scancode {
        match int {
            KEY_ESC => Scancode::Esc,
            KEY_C => Scancode::C,
            _ => Scancode::Unidentified(int),
        }
    }
}

pub enum KeyboardEventKind {
    Press { scancode: Scancode },
    Release { scancode: Scancode },
}

pub struct KeyboardEvent {
    pub kind: KeyboardEventKind,
    pub surface_id: usize,
    pub mods: Mods,
}

pub enum Event {
    Keyboard(KeyboardEvent),
}
