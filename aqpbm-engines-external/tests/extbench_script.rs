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

#[test]
fn a_data_dir_the_script_did_not_make_is_never_wiped() {
    let dir = std::env::temp_dir().join(format!("extbench-guard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("keep"), "data").unwrap();
    let binary = "/bin/true";
    let sha256 = Command::new("sha256sum").arg(binary).output().unwrap();
    let sha256 = String::from_utf8(sha256.stdout).unwrap();
    let out = Command::new("bash")
        .arg(script())
        .args(["clickhouse", "up"])
        .arg(&dir)
        .args(["2,22", "0", "18123", "19101"])
        .env("LISTEN_HOST", "127.0.0.1")
        .env("CLICKHOUSE_BINARY", binary)
        .env(
            "CLICKHOUSE_SHA256",
            sha256.split_whitespace().next().unwrap(),
        )
        .env("BASE_IMAGE", "unused")
        .env("APPROXBENCH", "unused")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("was not made by this script"));
    assert!(dir.join("keep").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
