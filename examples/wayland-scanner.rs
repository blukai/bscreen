use core::str;

fn main() -> anyhow::Result<()> {
    // TODO: git submodule?
    let filepath = "wayland-protocols/wlr-layer-shell-unstable-v1.xml";
    let file = std::fs::File::open(filepath)?;
    let protocol = wayland_scanner::parse::parse_protocol(std::io::BufReader::new(file))?;

    let mut buf: Vec<u8> = Vec::new();
    for interface in protocol.interfaces.iter().take(5) {
        // dbg!(interface);
        // wayland_scanner::generate::emit_interface(&mut buf, interface)?;
    }
    eprint!("{}", str::from_utf8(&buf)?);

    Ok(())
}
