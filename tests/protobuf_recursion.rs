use hbb_common::{protobuf::Message, rendezvous_proto::RendezvousMessage};
use std::process::Command;

const CHILD_ENV: &str = "RUSTDESK_PROTOBUF_RECURSION_CHILD";
const CHILD_COMPLETION_SENTINEL: &str = "protobuf_recursion_child_completed";
const TEST_NAME: &str = "deeply_nested_unknown_groups_are_rejected_without_aborting";

#[test]
fn deeply_nested_unknown_groups_are_rejected_without_aborting() {
    if std::env::var_os(CHILD_ENV).is_some() {
        const DEPTH: usize = 300_000;
        let mut input = vec![0x0b; DEPTH];
        input.extend(std::iter::repeat(0x0c).take(DEPTH));

        assert!(RendezvousMessage::parse_from_bytes(&input).is_err());
        println!("{CHILD_COMPLETION_SENTINEL}");
        return;
    }

    let output = Command::new(std::env::current_exe().expect("locate the test binary"))
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(CHILD_ENV, "1")
        .output()
        .expect("run the recursive-input check in an isolated process");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.lines().any(|line| line == CHILD_COMPLETION_SENTINEL),
        "child parser check did not complete successfully (status: {}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
}
