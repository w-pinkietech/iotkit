#[test]
fn version_exits_without_starting_the_service() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_iotkit-edge-node"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("iotkit-edge-node {}\n", env!("CARGO_PKG_VERSION"))
    );
}
