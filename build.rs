fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=PROTO_PATH=protos");
}
