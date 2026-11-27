use anyhow::Result;

use crate::system::preflight;

pub fn run() -> Result<()> {
    println!("Running host diagnostics...\n");
    let report = preflight::run_diagnostics(false, true);
    report.print();

    if report.has_failures() {
        println!("\nSome required checks failed. Resolve the [FAIL] items above before installing a runner.");
    } else {
        println!("\nHost is ready to run GitHub Actions self-hosted runners!");
    }
    Ok(())
}
