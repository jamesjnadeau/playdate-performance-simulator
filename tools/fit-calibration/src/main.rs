//! `fit-calibration`: fit the simulator's cost model to device timings.
//!
//! ```text
//! fit-calibration --lua-results results.json [--c-results c-results.json] \
//!     --out calibration/rev-a.json --device rev-a
//! ```

use anyhow::{Context, Result};
use clap::Parser;
use fit_calibration::*;
use pdsim_core::Calibration;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Fit a calibration profile to benchmark results from a Playdate")]
struct Args {
    /// results.json from calibration/bench-lua run on the device.
    #[arg(long)]
    lua_results: Option<PathBuf>,
    /// results.json from calibration/bench-c run on the device.
    #[arg(long)]
    c_results: Option<PathBuf>,
    /// The Lua bench project.
    #[arg(long, default_value = "calibration/bench-lua")]
    lua_bench: PathBuf,
    /// The C bench project (built: Source/pdex.elf).
    #[arg(long, default_value = "calibration/bench-c")]
    c_bench: PathBuf,
    /// Starting profile (default: built-in estimates).
    #[arg(long)]
    base: Option<PathBuf>,
    /// Where to write the fitted profile.
    #[arg(long)]
    out: PathBuf,
    /// Device revision ("rev-a", "rev-b").
    #[arg(long, default_value = "rev-a")]
    device: String,
    /// Strength of the pull toward the starting profile.
    #[arg(long, default_value_t = 1e-3)]
    prior: f64,
    /// Write the per-benchmark report here (JSON).
    #[arg(long)]
    report: Option<PathBuf>,
    /// Playdate SDK (defaults to $PLAYDATE_SDK_PATH; needed for the C bench).
    #[arg(long)]
    sdk: Option<PathBuf>,
}

fn main() -> Result<()> {
    let a = Args::parse();
    let base = match &a.base {
        Some(p) => Calibration::load(p)?,
        None => Calibration::default_estimates(),
    };
    let sdk = a.sdk.clone().or_else(pdsim_device::Device::sdk_from_env);
    let mut data = vec![];
    for (results, bench) in [(&a.lua_results, &a.lua_bench), (&a.c_results, &a.c_bench)] {
        let Some(path) = results else { continue };
        let dev: DeviceResults =
            serde_json::from_str(&std::fs::read_to_string(path).with_context(|| path.display().to_string())?)
                .with_context(|| format!("parsing {}", path.display()))?;
        eprintln!(
            "{}: {} timings; simulating {} ...",
            path.display(),
            dev.results.len(),
            bench.display()
        );
        let samples = simulate(
            bench,
            &base,
            dev.scale,
            dev.repeats,
            dev.results.len(),
            sdk.clone(),
        )?;
        data.extend(pair(&dev, samples)?);
    }
    anyhow::ensure!(!data.is_empty(), "give --lua-results and/or --c-results");
    let (mut fitted, report) = fit(&base, &data, FitOptions { prior: a.prior });
    fitted.device = a.device.clone();
    fitted.name = format!("Fitted from {} device benchmark runs ({})", data.len(), a.device);
    println!(
        "{:<28} {:>12} {:>12} {:>12}",
        "benchmark", "device ms", "before ms", "after ms"
    );
    for r in &report.rows {
        println!(
            "{:<28} {:>12.3} {:>12.3} {:>12.3}",
            r.name, r.measured_ms, r.before_ms, r.after_ms
        );
    }
    println!(
        "mean |error|: {:.1}% before, {:.1}% after ({} parameters)",
        report.error_before, report.error_after, report.parameters
    );
    std::fs::write(&a.out, fitted.to_json() + "\n")?;
    eprintln!("wrote {}", a.out.display());
    if let Some(p) = &a.report {
        std::fs::write(p, serde_json::to_string_pretty(&report)?)?;
    }
    Ok(())
}
