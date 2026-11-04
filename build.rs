use std::{env, fs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let version = env::var("CARGO_PKG_VERSION")?;
    fs::write(
        "./src/version.rs",
        format!("pub const VERSION: &str = \"{version}\";\n"),
    )?;
    Ok(())
}
