use std::env;
use std::fs::File;
use std::path::PathBuf;

use gl_generator::{Api, Fallbacks, Profile, Registry};

fn generate_egl_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(&env::var("OUT_DIR").unwrap());
    let mut file = File::create(out_dir.join("egl_bindings.rs")).unwrap();
    Registry::new(
        Api::Egl,
        (1, 5),
        Profile::Core,
        Fallbacks::All,
        ["EGL_MESA_image_dma_buf_export", "EGL_KHR_image"],
    )
    .write_bindings(gl_generator::StructGenerator, &mut file)?;

    Ok(())
}

fn generate_gl_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(&env::var("OUT_DIR").unwrap());
    let mut file = File::create(out_dir.join("gl_bindings.rs")).unwrap();
    Registry::new(
        Api::Gles2,
        (2, 0),
        Profile::Core,
        Fallbacks::None,
        ["GL_EXT_texture_format_BGRA8888"],
    )
    .write_bindings(gl_generator::StructGenerator, &mut file)?;

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");

    generate_egl_bindings()?;
    generate_gl_bindings()?;

    Ok(())
}
