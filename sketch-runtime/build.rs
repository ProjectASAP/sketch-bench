//! Compile `proto/feedback.proto` into generated Rust code via
//! `tonic-build` when the `grpc` feature is active. The
//! generated module is included by `src/exporter/grpc.rs` with
//! `tonic::include_proto!`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("CARGO_FEATURE_GRPC").is_none() {
        return Ok(());
    }
    // Server code is only used by integration tests + future
    // controller crate port; cheap to generate here since we're
    // already compiling protoc output.
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/feedback.proto"], &["proto"])?;
    println!("cargo:rerun-if-changed=proto/feedback.proto");
    Ok(())
}
