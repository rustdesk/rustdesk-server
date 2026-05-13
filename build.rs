fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/proto/rendezvous.capnp");
    println!("cargo:rustc-env=PROTO_PATH=protos");

    // Generate version.rs file
    core_common::gen_version();

    // Compile Cap'n Proto schema → Rust code
    capnpc::CompilerCommand::new()
        .src_prefix("src/proto")
        .file("src/proto/rendezvous.capnp")
        .run()
        .expect("Cap'n Proto schema compilation failed");
}
