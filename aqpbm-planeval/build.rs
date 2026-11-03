fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest = std::fs::read_to_string("Cargo.toml").expect("reading Cargo.toml");
    let rev = manifest
        .lines()
        .find(|line| line.starts_with("asap-planner "))
        .and_then(|line| line.split("rev = \"").nth(1))
        .and_then(|rest| rest.split('"').next())
        .expect("asap-planner is pinned to a rev in Cargo.toml");
    println!("cargo:rustc-env=ASAP_PLANNER_REV={rev}");
}
