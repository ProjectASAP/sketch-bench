//! `scripts/extbench-up.sh`: it parses, and it refuses to listen on an
//! address that is not private (the server has no password).

use std::process::Command;

fn script() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/extbench-up.sh")
}

#[test]
fn the_script_parses() {
    let out = Command::new("bash")
        .arg("-n")
        .arg(script())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn only_private_listen_addresses_are_accepted() {
    for (address, accepted) in [
        ("10.0.0.2", true),
        ("172.16.4.1", true),
        ("192.168.1.9", true),
        ("127.0.0.1", true),
        ("0.0.0.0", false),
        ("8.8.8.8", false),
        ("172.32.0.1", false),
        ("10.0.0", false),
        ("10.0.0.256", false),
    ] {
        let out = Command::new("bash")
            .arg(script())
            .args(["check-listen-host", address])
            .output()
            .unwrap();
        assert_eq!(out.status.success(), accepted, "{address}");
    }
}
