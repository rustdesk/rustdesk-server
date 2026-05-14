fn main() {
    println!("cargo:rerun-if-changed=protos/rendezvous.capnp");

    let out_dir = format!("{}/protos", std::env::var("OUT_DIR").unwrap());

    std::fs::create_dir_all(&out_dir).unwrap();

    protobuf_codegen::Codegen::new()
        .pure()
        .out_dir(out_dir)
        .inputs(["protos/rendezvous.proto", "protos/message.proto"])
        .include("protos")
        .customize(protobuf_codegen::Customize::default().tokio_bytes(true))
        .run()
        .expect("Codegen failed.");

    capnpc::CompilerCommand::new()
        .src_prefix("protos")
        .file("protos/rendezvous.capnp")
        .run()
        .expect("Cap'n Proto rendezvous.capnp compilation failed");
}
