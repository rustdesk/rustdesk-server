fn main() {
    hbb_common::gen_version();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64")
    {
        // libsodium-sys ships /MT libraries. Resolve their CRT references against
        // the same shared, serviceable runtime as Rust, rather than embedding a
        // second static CRT through those libraries' default-link directives.
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:LIBCMT");
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:LIBCMTD");
    }
}
