use aqp_core::function::run_aqpbmv2_function_demo;

fn main() -> anyhow::Result<()> {
    let report = run_aqpbmv2_function_demo();
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
