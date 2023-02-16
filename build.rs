fn main() {
    hbb_common::gen_version();
    if cfg!(target_os = "windows") {
        static_vcruntime::metabuild();
    }
}
