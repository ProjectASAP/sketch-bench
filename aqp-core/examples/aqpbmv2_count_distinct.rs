use aqp_core::function::run_count_distinct_demo;

fn main() -> anyhow::Result<()> {
    let report = run_count_distinct_demo();
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
