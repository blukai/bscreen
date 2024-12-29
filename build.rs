use std::fs::File;
use std::path::PathBuf;
use std::{env, fs};

use gl_generator::{Api, Fallbacks, Profile, Registry};

fn generate_egl_bindings() -> anyhow::Result<()> {
    let out_dir = PathBuf::from(&env::var("OUT_DIR")?);
    let mut out_file = File::create(out_dir.join("egl_bindings.rs"))?;
    Registry::new(Api::Egl, (1, 5), Profile::Core, Fallbacks::All, [
        "EGL_MESA_image_dma_buf_export",
        "EGL_KHR_image",
    ])
    .write_bindings(gl_generator::StructGenerator, &mut out_file)?;

    Ok(())
}

fn generate_gl_bindings() -> anyhow::Result<()> {
    let out_dir = PathBuf::from(&env::var("OUT_DIR")?);
    let mut out_file = File::create(out_dir.join("gl_bindings.rs"))?;
    Registry::new(Api::Gles2, (2, 0), Profile::Core, Fallbacks::None, [
        "GL_EXT_texture_format_BGRA8888",
    ])
    .write_bindings(gl_generator::StructGenerator, &mut out_file)?;

    Ok(())
}

fn generate_wayland_bindings() -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed=wayland-scanner");
    println!("cargo:rerun-if-changed=wayland-protocols");

    let out_dir = PathBuf::from(&env::var("OUT_DIR")?);
    let mut out_file = File::create(out_dir.join("wayland_bindings.rs"))?;

    let dir_entries = fs::read_dir("wayland-protocols")?;
    for dir_entry_result in dir_entries {
        let file = std::fs::File::open(dir_entry_result?.path())?;
        let protocol = wayland_scanner::parse::parse_protocol(std::io::BufReader::new(file))?;
        for interface in protocol.interfaces.iter() {
            wayland_scanner::generate::emit_interface(&mut out_file, interface)?;
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");

    generate_egl_bindings()?;
    generate_gl_bindings()?;
    generate_wayland_bindings()?;

    Ok(())
}
