extern crate core_common;

fn main() {
    println!("{:?}", core_common::config::PeerConfig::load("455058072"));
}
