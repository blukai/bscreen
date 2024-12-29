use std::io;

use crate::protocol::*;

// is_nullable_type
fn arg_type_nullable(arg_type: ArgType) -> bool {
    arg_type == ArgType::String || arg_type == ArgType::Object
}

// from emit_types_forward_declarations
fn arg_null(arg: &Arg) -> bool {
    !(arg.interface.is_some() && (arg.r#type == ArgType::Object || arg.r#type == ArgType::NewId))
}

// from emit_messages
fn emit_message_signature<W: io::Write>(w: &mut W, msg: &Message) -> io::Result<()> {
    if msg.since.is_some_and(|since| since > 1) {
        write!(w, "{}", msg.since.unwrap())?;
    }

    for arg in msg.args.iter() {
        if arg_type_nullable(arg.r#type) && arg.allow_null {
            write!(w, "?")?;
        }

        match arg.r#type {
            ArgType::Int => write!(w, "i")?,
            ArgType::Uint => write!(w, "u")?,
            ArgType::Fixed => write!(w, "f")?,
            ArgType::String => write!(w, "s")?,
            ArgType::Object => write!(w, "o")?,
            ArgType::Array => write!(w, "a")?,
            ArgType::NewId => {
                if arg.interface.is_none() {
                    write!(w, "su")?;
                }
                write!(w, "n")?;
            }
            ArgType::Fd => write!(w, "h")?,
        }
    }

    Ok(())
}

// emit_type
fn emit_arg_type<W: io::Write>(w: &mut W, arg: &Arg) -> io::Result<()> {
    match arg.r#type {
        ArgType::Int | ArgType::Fd => write!(w, "i32"),
        ArgType::NewId | ArgType::Uint => write!(w, "u32"),
        ArgType::Fixed => write!(w, "super::wl_fixed"),
        ArgType::String => write!(w, "*const std::ffi::c_char"),
        ArgType::Object => write!(
            w,
            "*mut {}",
            arg.interface.as_ref().expect("interface name")
        ),
        ArgType::Array => write!(w, "*mut super::wl_array"),
    }
}

pub enum MessageKind {
    Request,
    Event,
}

fn emit_structs<W: io::Write>(
    w: &mut W,
    interface: &Interface,
    kind: MessageKind,
) -> io::Result<()> {
    write!(w, "#[repr(C)]\n")?;
    write!(w, "pub struct {} {{\n", interface.name)?;
    write!(w, "    _data: [u8; 0],\n")?;
    write!(
        w,
        "    _marker: std::marker::PhantomData<(*mut u8, std::marker::PhantomPinned)>,\n"
    )?;
    write!(w, "}}\n\n")?;

    let messages = match kind {
        MessageKind::Request => &interface.requests,
        MessageKind::Event => &interface.events,
    };

    if messages.is_empty() {
        return Ok(());
    }

    write!(w, "pub struct {}_listener {{\n", interface.name)?;
    for msg in messages.iter() {
        write!(w, "    pub {}: unsafe extern \"C\" fn(\n", msg.name)?;
        write!(w, "        data: *mut std::ffi::c_void,\n")?;
        write!(w, "        {}: *mut {},\n", interface.name, interface.name)?;
        for arg in msg.args.iter() {
            write!(w, "        {}: ", arg.name)?;
            match arg.r#type {
                ArgType::Object if arg.interface.is_none() => write!(w, "*mut std::ffi::c_void")?,
                ArgType::NewId => write!(
                    w,
                    "*mut {}",
                    arg.interface.as_ref().expect("interface name")
                )?,
                _ => emit_arg_type(w, arg)?,
            }
            write!(w, ",\n")?;
        }
        write!(w, "    ),\n")?;
    }
    write!(w, "}}\n\n")?;

    Ok(())
}

fn emit_messages<W: io::Write>(
    w: &mut W,
    interface: &Interface,
    kind: MessageKind,
) -> io::Result<()> {
    let (kind_name, messages) = match kind {
        MessageKind::Request => ("requests", &interface.requests),
        MessageKind::Event => ("events", &interface.events),
    };

    if messages.is_empty() {
        return Ok(());
    }

    write!(
        w,
        "static {}_{kind_name}: [super::wl_message; {}] = [\n",
        interface.name,
        messages.len()
    )?;
    for msg in messages.iter() {
        write!(w, "    super::wl_message {{\n")?;

        write!(
            w,
            "        name: \"{}\\0\".as_ptr() as *const std::ffi::c_char,\n",
            msg.name
        )?;

        write!(w, "        signature: \"")?;
        emit_message_signature(w, msg)?;
        write!(w, "\\0\".as_ptr() as *const std::ffi::c_char,\n",)?;

        if msg.args.iter().all(arg_null) {
            write!(w, "        types: std::ptr::null(),\n")?;
        } else {
            write!(w, "        types: [\n")?;
            for arg in msg.args.iter() {
                match arg.r#type {
                    ArgType::NewId | ArgType::Object if arg.interface.is_some() => {
                        write!(
                            w,
                            "            &{}_interface as *const super::wl_interface,\n",
                            arg.interface.as_ref().unwrap()
                        )?;
                    }
                    _ => {
                        write!(w, "            std::ptr::null(),\n")?;
                    }
                }
            }
            write!(w, "        ].as_ptr(),\n",)?;
        }

        write!(w, "    }},\n")?;
    }
    write!(w, "];\n\n")?;

    Ok(())
}

fn emit_enums<W: io::Write>(w: &mut W, interface: &Interface) -> io::Result<()> {
    for e in interface.enums.iter() {
        for entry in e.entries.iter() {
            write!(
                w,
                "pub const {}_{}_{}: u32 = {};\n",
                interface.name.to_uppercase(),
                e.name.to_uppercase(),
                entry.name.to_uppercase(),
                entry.value,
            )?;
        }

        write!(w, "\n")?;
    }

    Ok(())
}

fn emit_opcodes<W: io::Write>(
    w: &mut W,
    interface: &Interface,
    kind: MessageKind,
) -> io::Result<()> {
    let messages = match kind {
        MessageKind::Request => &interface.requests,
        MessageKind::Event => &interface.events,
    };

    if messages.is_empty() {
        return Ok(());
    }

    for (opcode, msg) in messages.iter().enumerate() {
        write!(
            w,
            "const {}_{}: u32 = {};\n",
            interface.name.to_uppercase(),
            msg.name.to_uppercase(),
            opcode
        )?;
    }

    write!(w, "\n")
}

fn emit_stubs<W: io::Write>(w: &mut W, interface: &Interface) -> io::Result<()> {
    for msg in interface.requests.iter() {
        let new_id_count = msg
            .args
            .iter()
            .fold(0, |acc, arg| acc + (arg.r#type == ArgType::NewId) as usize);
        assert!(new_id_count <= 1);

        let ret = msg.args.iter().find(|arg| arg.r#type == ArgType::NewId);

        write!(w, "#[inline]\n",)?;
        write!(w, "pub unsafe fn {}_{}(\n", interface.name, msg.name,)?;

        // args

        write!(w, "    lib: &super::Lib,\n",)?;
        write!(w, "    {}: *mut {},\n", interface.name, interface.name,)?;

        for arg in msg.args.iter() {
            if arg.r#type == ArgType::NewId {
                if arg.interface.is_none() {
                    write!(w, "    interface: *const super::wl_interface,\n")?;
                    write!(w, "    version: u32,\n")?;
                }
                continue;
            }

            write!(w, "    {}: ", arg.name)?;
            emit_arg_type(w, arg)?;
            write!(w, ",\n")?;
        }

        // return

        if let Some(ret) = ret {
            if let Some(interface_name) = ret.interface.as_ref() {
                write!(w, ") -> *mut {} {{\n", interface_name)?;
            } else {
                write!(w, ") -> *mut std::ffi::c_void {{\n")?;
            }
        } else {
            write!(w, ") {{ \n")?;
        }

        // body

        write!(w, "    (lib.wl_proxy_marshal_flags)(\n",)?;

        // proxy: *mut super::wl_proxy,
        write!(w, "        {} as *mut super::wl_proxy,\n", interface.name)?;
        // opcode: u32,
        write!(
            w,
            "        {}_{},\n",
            interface.name.to_uppercase(),
            msg.name.to_uppercase()
        )?;
        // interface: *const wl_interface,
        if let Some(ret) = ret {
            if let Some(interface_name) = ret.interface.as_ref() {
                write!(w, "        &{}_interface,\n", interface_name)?;
            } else {
                write!(w, "        interface,\n")?;
            }
        } else {
            write!(w, "        std::ptr::null(),\n")?;
        }
        // version: u32,
        if ret.is_some_and(|arg| arg.interface.is_none()) {
            write!(w, "        version,\n")?;
        } else {
            write!(
                w,
                "        (lib.wl_proxy_get_version)({} as *mut super::wl_proxy),\n",
                interface.name
            )?;
        }
        // flags: u32,
        if msg.r#type.as_ref().is_some_and(|ty| ty.eq("destructor")) {
            write!(w, "        super::WL_MARSHAL_FLAG_DESTROY,\n")?;
        } else {
            write!(w, "        0,\n")?;
        }
        // ...
        for arg in msg.args.iter() {
            if arg.r#type == ArgType::NewId {
                if arg.interface.is_none() {
                    write!(w, "        (*interface).name,\n")?;
                    write!(w, "        version,\n")?;
                }
                write!(w, "        std::ptr::null::<std::ffi::c_void>(),\n")?;
            } else {
                write!(w, "        {},\n", arg.name)?;
            }
        }

        if ret.is_some() {
            write!(w, "    ) as _\n")?;
        } else {
            write!(w, "    );\n")?;
        }

        write!(w, "}}\n\n")?;
    }

    Ok(())
}

pub fn emit_interface<W: io::Write>(w: &mut W, interface: &Interface) -> io::Result<()> {
    emit_structs(w, interface, MessageKind::Event)?;
    emit_messages(w, interface, MessageKind::Request)?;
    emit_messages(w, interface, MessageKind::Event)?;

    // TODO: consider wrapping this into whatever proxy struct as consts. and do the same with
    // _events and _requests.
    write!(
        w,
        "pub static {}_interface: super::wl_interface = super::wl_interface {{\n",
        interface.name,
    )?;
    write!(
        w,
        "    name: \"{}\\0\".as_ptr() as *const std::ffi::c_char,\n",
        interface.name
    )?;
    write!(w, "    version: {},\n", interface.version)?;
    write!(w, "    method_count: {},\n", interface.requests.len())?;
    if !interface.requests.is_empty() {
        write!(w, "    methods: {}_requests.as_ptr(),\n", interface.name)?;
    } else {
        write!(w, "    methods: std::ptr::null(),\n")?;
    }
    write!(w, "    event_count: {},\n", interface.events.len())?;
    if !interface.events.is_empty() {
        write!(w, "    events: {}_events.as_ptr(),\n", interface.name)?;
    } else {
        write!(w, "    events: std::ptr::null(),\n")?;
    }
    write!(w, "}};\n\n")?;

    emit_enums(w, interface)?;
    emit_opcodes(w, interface, MessageKind::Request)?;
    emit_stubs(w, interface)?;

    Ok(())
}
