use hbb_common::{protobuf::Message, rendezvous_proto::RendezvousMessage};
use std::process::Command;

const CHILD_ENV: &str = "RUSTDESK_PROTOBUF_RECURSION_CHILD";
const TEST_NAME: &str = "deeply_nested_unknown_groups_are_rejected_without_aborting";

#[test]
fn deeply_nested_unknown_groups_are_rejected_without_aborting() {
    if std::env::var_os(CHILD_ENV).is_some() {
        const DEPTH: usize = 300_000;
        let mut input = vec![0x0b; DEPTH];
        input.extend(std::iter::repeat(0x0c).take(DEPTH));

        assert!(RendezvousMessage::parse_from_bytes(&input).is_err());
        return;
    }

    let status = Command::new(std::env::current_exe().expect("locate the test binary"))
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_ENV, "1")
        .status()
        .expect("run the recursive-input check in an isolated process");

    assert!(
        status.success(),
        "parsing deeply nested unknown protobuf groups aborted the process: {status}"
    );
}
