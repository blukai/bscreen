use anyhow::{Context as _, anyhow};
use quick_xml::{events::Event, reader::Reader};

use crate::protocol::*;

fn parse_num(bytes: &[u8]) -> anyhow::Result<u32> {
    let s = std::str::from_utf8(bytes)?;
    if s.len() < 2 || !s.starts_with("0x") {
        s.parse()
    } else {
        u32::from_str_radix(&s[2..], 16)
    }
    .with_context(|| {
        format!(
            "could not parse num {}",
            // boo
            unsafe { std::str::from_utf8_unchecked(bytes.as_ref()) },
        )
    })
}

pub fn parse_protocol<R>(reader: R) -> anyhow::Result<Protocol>
where
    R: std::io::BufRead,
{
    let mut reader = Reader::from_reader(reader);

    let mut buf = Vec::new();

    let mut protocol = Protocol::default();

    let mut cur_interface: Option<Interface> = None;
    let mut cur_message: Option<Message> = None;
    let mut cur_enum: Option<Enum> = None;

    loop {
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.name().as_ref().eq(b"entry") => {
                let mut entry = Entry::default();
                for attr in e.attributes() {
                    let attr = attr?;
                    match attr.key.as_ref() {
                        b"name" => entry.name = String::from_utf8(attr.value.to_vec())?,
                        b"value" => entry.value = parse_num(&attr.value)?,
                        b"since" => entry.since = Some(parse_num(&attr.value)?),
                        b"deprecated-since" => {
                            entry.deprecated_since = Some(parse_num(&attr.value)?)
                        }
                        _ => {}
                    }
                }
                let r#enum = cur_enum.as_mut().unwrap();
                r#enum.entries.push(entry);
            }
            Event::Empty(e) if e.name().as_ref().eq(b"arg") => {
                let mut arg = Arg::default();
                for attr in e.attributes() {
                    let attr = attr?;
                    match attr.key.as_ref() {
                        b"name" => arg.name = String::from_utf8(attr.value.to_vec())?,
                        b"type" => {
                            arg.r#type = match attr.value.as_ref() {
                                b"new_id" => ArgType::NewId,
                                b"int" => ArgType::Int,
                                b"uint" => ArgType::Uint,
                                b"fixed" => ArgType::Fixed,
                                b"string" => ArgType::String,
                                b"object" => ArgType::Object,
                                b"array" => ArgType::Array,
                                b"fd" => ArgType::Fd,
                                value => {
                                    return Err(anyhow!(
                                        "unexpected value of arg type: {}",
                                        std::str::from_utf8(value)?
                                    ));
                                }
                            }
                        }
                        b"interface" => {
                            arg.interface = Some(String::from_utf8(attr.value.to_vec())?)
                        }
                        b"allow_null" => arg.allow_null = attr.value.as_ref().eq(b"true"),
                        b"enum" => arg.r#enum = Some(String::from_utf8(attr.value.to_vec())?),
                        _ => {}
                    }
                }
                let message = cur_message.as_mut().unwrap();
                message.args.push(arg);
            }
            Event::Start(e) => match e.name().as_ref() {
                b"protocol" => {
                    for attr in e.attributes() {
                        let attr = attr?;
                        match attr.key.as_ref() {
                            b"name" => protocol.name = String::from_utf8(attr.value.to_vec())?,
                            _ => {}
                        }
                    }
                }
                b"interface" => {
                    assert!(cur_interface.is_none());
                    let interface = cur_interface.insert(Interface::default());
                    for attr in e.attributes() {
                        let attr = attr?;
                        match attr.key.as_ref() {
                            b"name" => interface.name = String::from_utf8(attr.value.to_vec())?,
                            b"version" => interface.version = parse_num(&attr.value)?,
                            _ => {}
                        }
                    }
                }
                b"request" | b"event" => {
                    assert!(cur_message.is_none());
                    let message = cur_message.insert(Message::default());
                    for attr in e.attributes() {
                        let attr = attr?;
                        match attr.key.as_ref() {
                            b"name" => message.name = String::from_utf8(attr.value.to_vec())?,
                            b"type" => {
                                message.r#type = Some(String::from_utf8(attr.value.to_vec())?)
                            }
                            b"since" => message.since = Some(parse_num(&attr.value)?),
                            b"deprecated-since" => {
                                message.deprecated_since = Some(parse_num(&attr.value)?)
                            }
                            _ => {}
                        }
                    }
                }
                b"enum" => {
                    assert!(cur_enum.is_none());
                    let r#enum = cur_enum.insert(Enum::default());
                    for attr in e.attributes() {
                        let attr = attr?;
                        match attr.key.as_ref() {
                            b"name" => r#enum.name = String::from_utf8(attr.value.to_vec())?,
                            b"since" => r#enum.since = Some(parse_num(&attr.value)?),
                            b"bitfield" => r#enum.bitfield = attr.value.as_ref().eq(b"true"),
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Event::End(e) => match e.name().as_ref() {
                b"protocol" => {
                    assert!(cur_enum.is_none());
                    assert!(cur_message.is_none());
                    assert!(cur_interface.is_none());
                }
                b"interface" => {
                    let interface = cur_interface.take().unwrap();
                    protocol.interfaces.push(interface);
                }
                b"request" => {
                    let interface = cur_interface.as_mut().unwrap();
                    interface.requests.push(cur_message.take().unwrap());
                }
                b"event" => {
                    let interface = cur_interface.as_mut().unwrap();
                    interface.events.push(cur_message.take().unwrap());
                }
                b"enum" => {
                    let interface = cur_interface.as_mut().unwrap();
                    interface.enums.push(cur_enum.take().unwrap());
                }
                _ => {}
            },
            _ => {}
        }
    }

    Ok(protocol)
}
