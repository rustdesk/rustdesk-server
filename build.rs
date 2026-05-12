fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=PROTO_PATH=protos");
    
    // Generate version.rs file
    core_common::gen_version();
}
