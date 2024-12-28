use serde::{Deserialize, Deserializer};

// https://gitlab.freedesktop.org/wayland/wayland/-/blob/9cb3d7aa9dc995ffafdbdef7ab86a949d0fb0e7d/protocol/wayland.dtd

fn deserialize_u32_or_hex<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    <std::borrow::Cow<'de, str> as Deserialize<'de>>::deserialize(deserializer).and_then(|s| {
        if s.len() < 2 || !s.starts_with("0x") {
            s.parse()
                .map_err(|_| serde::de::Error::custom(format!("{s} is not hex nor u32")))
        } else {
            u32::from_str_radix(&s[2..], 16)
                .map_err(|_| serde::de::Error::custom("could not parse as u8"))
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgType {
    NewId,
    Int,
    Uint,
    Fixed,
    String,
    Object,
    Array,
    Fd,
}

#[derive(Debug, Deserialize)]
pub struct Arg {
    pub description: Option<String>,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type")]
    pub r#type: ArgType,
    #[serde(rename = "@summary")]
    pub summary: Option<String>,
    #[serde(rename = "@interface")]
    pub interface: Option<String>,
    #[serde(rename = "@allow-null", default)]
    pub allow_null: bool,
    #[serde(rename = "@enum")]
    pub r#enum: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub description: Option<String>,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@type")]
    pub r#type: Option<String>,
    #[serde(rename = "@since")]
    pub since: Option<u32>,
    #[serde(rename = "@deprecated-since")]
    pub deprecated_since: Option<u32>,
    #[serde(rename = "arg", default)]
    pub args: Vec<Arg>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub description: Option<String>,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@value", deserialize_with = "deserialize_u32_or_hex")]
    pub value: u32,
    #[serde(rename = "@summary")]
    pub summary: Option<String>,
    #[serde(rename = "@since")]
    pub since: Option<u32>,
    #[serde(rename = "@deprecated-since")]
    pub deprecated_since: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct Enum {
    pub description: Option<String>,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@since")]
    pub since: Option<u32>,
    #[serde(rename = "@bitfield", default)]
    pub bitfield: bool,
    #[serde(rename = "entry", default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Interface {
    pub description: Option<String>,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "@version")]
    pub version: u32,
    #[serde(rename = "request", default)]
    pub requests: Vec<Message>,
    #[serde(rename = "event", default)]
    pub events: Vec<Message>,
    #[serde(rename = "enum", default)]
    pub enums: Vec<Enum>,
}

#[derive(Debug, Deserialize)]
pub struct Protocol {
    pub description: Option<String>,
    #[serde(rename = "@name")]
    pub name: String,
    #[serde(rename = "interface")]
    pub interfaces: Vec<Interface>,
}

pub fn deserialize_protocol<R>(reader: R) -> Result<Protocol, quick_xml::DeError>
where
    R: std::io::BufRead,
{
    quick_xml::de::from_reader(std::io::BufReader::new(reader))
}
