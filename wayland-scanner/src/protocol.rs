// https://gitlab.freedesktop.org/wayland/wayland/-/blob/9cb3d7aa9dc995ffafdbdef7ab86a949d0fb0e7d/protocol/wayland.dtd

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum ArgType {
    #[default]
    NewId,
    Int,
    Uint,
    Fixed,
    String,
    Object,
    Array,
    Fd,
}

#[derive(Debug, Default)]
pub struct Arg {
    pub name: String,
    pub r#type: ArgType,
    pub interface: Option<String>,
    pub allow_null: bool,
    pub r#enum: Option<String>,
}

#[derive(Debug, Default)]
pub struct Message {
    pub name: String,
    pub r#type: Option<String>,
    pub since: Option<u32>,
    pub deprecated_since: Option<u32>,
    pub args: Vec<Arg>,
}

#[derive(Debug, Default)]
pub struct Entry {
    pub name: String,
    pub value: u32,
    pub since: Option<u32>,
    pub deprecated_since: Option<u32>,
}

#[derive(Debug, Default)]
pub struct Enum {
    pub name: String,
    pub since: Option<u32>,
    pub bitfield: bool,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Default)]
pub struct Interface {
    pub name: String,
    pub version: u32,
    pub requests: Vec<Message>,
    pub events: Vec<Message>,
    pub enums: Vec<Enum>,
}

#[derive(Debug, Default)]
pub struct Protocol {
    pub name: String,
    pub interfaces: Vec<Interface>,
}
