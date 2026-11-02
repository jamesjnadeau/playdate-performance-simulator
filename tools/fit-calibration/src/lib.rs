//! Fitting a calibration profile to device benchmark timings.
//!
//! 1. The bench project runs on a device and records, for each benchmark,
//!    the seconds between `resetElapsedTime` and `getElapsedTime`.
//! 2. [`simulate`] runs the same project in the simulator with feature
//!    recording on; each `getElapsedTime` yields the exact work done
//!    (opcode counts, API calls with their pixel/row counts, ARM instruction
//!    classes, ...).
//! 3. [`fit`] solves for the per-unit costs: weighted least squares on
//!    relative error, regularized toward the starting profile (so costs the
//!    benchmarks cannot separate stay at their prior), with costs kept
//!    non-negative.

use pdsim_core::{ApiCost, Calibration};
use pdsim_device::Features;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Device results as written by the bench projects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceResults {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub bench: String,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default = "three")]
    pub repeats: u32,
    pub results: Vec<BenchTime>,
}

fn one() -> f64 {
    1.0
}
fn three() -> u32 {
    3
}

/// One timed benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchTime {
    pub name: String,
    pub seconds: f64,
}

fn temp_data_dir(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("pdsim-{tag}-{}-{n}", std::process::id()))
}

/// Feature counts for one benchmark run.
pub type Sample = BTreeMap<String, f64>;

/// Runs the bench project at `game` in the simulator and returns one sample
/// per timed benchmark (in order), plus the simulated seconds for each.
pub fn simulate(
    game: &Path,
    cal: &Calibration,
    scale: f64,
    repeats: u32,
    expected: usize,
    sdk: Option<std::path::PathBuf>,
) -> anyhow::Result<Vec<(Sample, f64)>> {
    let data = temp_data_dir("fit");
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data)?;
    std::fs::write(
        data.join("bench-config.json"),
        serde_json::json!({"scale": scale, "repeats": repeats}).to_string(),
    )?;
    let cfg = pdsim_runner::RunConfig {
        calibration: cal.clone(),
        data_dir: Some(data.clone()),
        sdk,
        profile: false,
    };
    let mut g = pdsim_runner::load(game, &cfg).map_err(|e| anyhow::anyhow!(e.0))?;
    g.device().borrow_mut().features = Some(Features::default());
    let mut times = vec![];
    let mut frames = 0;
    while g
        .device()
        .borrow()
        .features
        .as_ref()
        .map_or(0, |f| f.samples.len())
        < expected
    {
        let before = g
            .device()
            .borrow()
            .features
            .as_ref()
            .map_or(0, |f| f.samples.len());
        g.frame(None).map_err(|e| anyhow::anyhow!(e.0))?;
        let d = g.device().borrow();
        let f = d.features.as_ref().unwrap();
        // The elapsed time the game saw is the sample's work: recompute it
        // from the profile so it matches the model exactly.
        for s in &f.samples[before..] {
            times.push(predict(cal, s) / cal.cpu_hz as f64);
        }
        frames += 1;
        anyhow::ensure!(frames < expected * 4 + 100, "the bench stopped producing results");
    }
    let samples = g.device().borrow().features.as_ref().unwrap().samples.clone();
    let _ = std::fs::remove_dir_all(&data);
    Ok(samples.into_iter().take(expected).zip(times).collect())
}

/// Runs a bench project in the simulator as if it were the device, and
/// returns the `results.json` it writes. Used for dry runs and to test the
/// fitting pipeline without hardware.
pub fn run_as_device(
    game: &Path,
    cal: &Calibration,
    scale: f64,
    repeats: u32,
    sdk: Option<std::path::PathBuf>,
) -> anyhow::Result<DeviceResults> {
    let data = temp_data_dir("dry");
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data)?;
    std::fs::write(
        data.join("bench-config.json"),
        serde_json::json!({"scale": scale, "repeats": repeats}).to_string(),
    )?;
    let cfg = pdsim_runner::RunConfig {
        calibration: cal.clone(),
        data_dir: Some(data.clone()),
        sdk,
        profile: false,
    };
    let mut g = pdsim_runner::load(game, &cfg).map_err(|e| anyhow::anyhow!(e.0))?;
    for _ in 0..20_000 {
        if data.join("results.json").exists() {
            break;
        }
        g.frame(None).map_err(|e| anyhow::anyhow!(e.0))?;
    }
    let text = std::fs::read_to_string(data.join("results.json"))
        .map_err(|e| anyhow::anyhow!("no results.json: {e}"))?;
    let _ = std::fs::remove_dir_all(&data);
    Ok(serde_json::from_str(&text)?)
}

/// A tunable cost in the profile.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Param {
    LuaOp(String),
    LuaCCall,
    /// Extra cost of one C function (`math.sin`, ...).
    LuaCFunc(String),
    LuaAlloc,
    LuaFree,
    LuaGcWork,
    LuaNativeCall,
    Api(String, &'static str),
    Arm(String),
    ArmRegister,
    DisplayBase,
    DisplayRow,
}

/// The cost parameter a feature multiplies.
pub fn param_of(feature: &str) -> Option<Param> {
    if let Some(op) = feature.strip_prefix("lua.op.") {
        return Some(Param::LuaOp(op.to_string()));
    }
    if let Some(f) = feature.strip_prefix("lua.cfunc.") {
        return Some(Param::LuaCFunc(f.to_string()));
    }
    if let Some(rest) = feature.strip_prefix("api.") {
        let (name, field) = rest.rsplit_once('.')?;
        let f = match field {
            "calls" => "base",
            "pixels" => "per_pixel",
            "rows" => "per_row",
            "bytes" => "per_byte",
            "items" => "per_item",
            _ => return None,
        };
        return Some(Param::Api(name.to_string(), f));
    }
    if let Some(c) = feature.strip_prefix("arm.") {
        return Some(if c == "registers" {
            Param::ArmRegister
        } else {
            Param::Arm(c.to_string())
        });
    }
    Some(match feature {
        "lua.c_calls" => Param::LuaCCall,
        "lua.alloc_bytes" => Param::LuaAlloc,
        "lua.free_bytes" => Param::LuaFree,
        "lua.gc_work" => Param::LuaGcWork,
        "lua.native_calls" => Param::LuaNativeCall,
        "display.updates" => Param::DisplayBase,
        "display.rows" => Param::DisplayRow,
        _ => return None,
    })
}

fn api_field(c: &ApiCost, f: &str) -> f64 {
    match f {
        "base" => c.base,
        "per_pixel" => c.per_pixel,
        "per_row" => c.per_row,
        "per_byte" => c.per_byte,
        _ => c.per_item,
    }
}

/// The value of a parameter in a profile.
pub fn get(cal: &Calibration, p: &Param) -> f64 {
    match p {
        Param::LuaOp(op) => cal.lua_opcode(op),
        Param::LuaCCall => cal.lua.c_function_call,
        Param::LuaCFunc(f) => cal.lua.c_functions.get(f).copied().unwrap_or(0.0),
        Param::LuaAlloc => cal.lua.gc_per_alloc_byte,
        Param::LuaFree => cal.lua.gc_per_free_byte,
        Param::LuaGcWork => cal.lua.gc_per_work_unit,
        Param::LuaNativeCall => cal.lua.native_call_overhead,
        Param::Api(n, f) => api_field(&cal.api(n), f),
        Param::Arm(c) => cal.arm_class(c),
        Param::ArmRegister => cal.arm.per_register,
        Param::DisplayBase => cal.display.base,
        Param::DisplayRow => cal.display.per_row,
    }
}

/// Sets a parameter in a profile.
pub fn set(cal: &mut Calibration, p: &Param, v: f64) {
    match p {
        Param::LuaOp(op) => {
            cal.lua.opcodes.insert(op.clone(), v);
        }
        Param::LuaCCall => cal.lua.c_function_call = v,
        Param::LuaCFunc(f) => {
            cal.lua.c_functions.insert(f.clone(), v);
        }
        Param::LuaAlloc => cal.lua.gc_per_alloc_byte = v,
        Param::LuaFree => cal.lua.gc_per_free_byte = v,
        Param::LuaGcWork => cal.lua.gc_per_work_unit = v,
        Param::LuaNativeCall => cal.lua.native_call_overhead = v,
        Param::Api(n, f) => {
            let mut c = cal.api(n);
            match *f {
                "base" => c.base = v,
                "per_pixel" => c.per_pixel = v,
                "per_row" => c.per_row = v,
                "per_byte" => c.per_byte = v,
                _ => c.per_item = v,
            }
            cal.api.insert(n.clone(), c);
        }
        Param::Arm(c) => {
            cal.arm.classes.insert(c.clone(), v);
        }
        Param::ArmRegister => cal.arm.per_register = v,
        Param::DisplayBase => cal.display.base = v,
        Param::DisplayRow => cal.display.per_row = v,
    }
}

/// Cycles the profile predicts for a sample.
pub fn predict(cal: &Calibration, s: &Sample) -> f64 {
    s.iter()
        .filter_map(|(k, v)| param_of(k).map(|p| get(cal, &p) * v))
        .sum()
}

/// Fit options.
#[derive(Debug, Clone, Copy)]
pub struct FitOptions {
    /// Strength of the pull toward the starting profile (relative units).
    pub prior: f64,
}

impl Default for FitOptions {
    fn default() -> Self {
        FitOptions { prior: 1e-6 }
    }
}

/// One line of the fit report.
#[derive(Debug, Clone, Serialize)]
pub struct FitRow {
    pub name: String,
    pub measured_ms: f64,
    pub before_ms: f64,
    pub after_ms: f64,
    /// The work the simulator recorded for this run.
    pub features: Sample,
}

/// Fit result.
#[derive(Debug, Clone, Serialize)]
pub struct FitReport {
    pub rows: Vec<FitRow>,
    /// Mean absolute relative error before and after, in percent.
    pub error_before: f64,
    pub error_after: f64,
    pub parameters: usize,
}

/// Solves `A x = b` (A square) by Gaussian elimination with partial pivoting.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let piv = (col..n).max_by(|i, j| a[*i][col].abs().partial_cmp(&a[*j][col].abs()).unwrap())?;
        if a[piv][col].abs() < 1e-300 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..n {
            let f = a[r][col] / a[col][col];
            if f != 0.0 {
                let (top, bottom) = a.split_at_mut(r);
                for (x, y) in bottom[0][col..n].iter_mut().zip(&top[col][col..n]) {
                    *x -= f * y;
                }
                b[r] -= f * b[col];
            }
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|c| a[r][c] * x[c]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    Some(x)
}

/// Minimizes `z·A·z/2 - b·z` subject to `z >= 0` (A symmetric positive
/// definite): the Lawson-Hanson active-set method on the normal equations.
fn nnls(a: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let n = b.len();
    let tol = 1e-12;
    let mut z = vec![0.0; n];
    let mut free = vec![false; n];
    // Solves the equations restricted to the free set; the rest stay zero.
    let solve_free = |free: &[bool]| -> Vec<f64> {
        let idx: Vec<usize> = (0..n).filter(|i| free[*i]).collect();
        let sub = idx.iter().map(|&i| idx.iter().map(|&j| a[i][j]).collect()).collect();
        let rhs = idx.iter().map(|&i| b[i]).collect();
        let mut s = vec![0.0; n];
        if let Some(sol) = solve(sub, rhs) {
            for (k, &i) in idx.iter().enumerate() {
                s[i] = sol[k];
            }
        }
        s
    };
    for _ in 0..3 * n {
        // The most promising bound cost: steepest descent among those at zero.
        let grad: Vec<f64> = (0..n).map(|i| b[i] - (0..n).map(|j| a[i][j] * z[j]).sum::<f64>()).collect();
        let Some(k) = (0..n)
            .filter(|i| !free[*i] && grad[*i] > tol)
            .max_by(|i, j| grad[*i].partial_cmp(&grad[*j]).unwrap())
        else {
            break;
        };
        free[k] = true;
        for _ in 0..n {
            let s = solve_free(&free);
            if (0..n).all(|i| !free[i] || s[i] > 0.0) {
                z = s;
                break;
            }
            // Step toward s until the first free cost hits zero, then bind it.
            let alpha = (0..n)
                .filter(|i| free[*i] && s[*i] <= 0.0)
                .map(|i| z[i] / (z[i] - s[i]))
                .fold(1.0, f64::min);
            for i in 0..n {
                z[i] += alpha * (s[i] - z[i]);
                if free[i] && z[i] <= tol {
                    free[i] = false;
                    z[i] = 0.0;
                }
            }
        }
    }
    z
}

/// Fits `base` to `(sample, measured seconds, name)` triples.
pub fn fit(base: &Calibration, data: &[(Sample, f64, String)], opts: FitOptions) -> (Calibration, FitReport) {
    let hz = base.cpu_hz as f64;
    let mut params: Vec<Param> = data
        .iter()
        .flat_map(|(s, _, _)| s.keys().filter_map(|k| param_of(k)))
        .collect();
    params.sort();
    params.dedup();
    let p0: Vec<f64> = params.iter().map(|p| get(base, p)).collect();
    let rows: Vec<(Vec<f64>, f64)> = data
        .iter()
        .filter(|(_, t, _)| *t > 0.0)
        .map(|(s, t, _)| {
            let mut x = vec![0.0; params.len()];
            for (k, v) in s {
                if let Some(p) = param_of(k) {
                    if let Ok(i) = params.binary_search(&p) {
                        x[i] += v;
                    }
                }
            }
            (x, t * hz)
        })
        .collect();
    // Solve in relative units (each cost divided by its prior), so costs that
    // differ by many orders of magnitude (per pixel vs per call) keep the
    // normal equations well conditioned. Per-function extras start at zero,
    // so they take the scale of the average C call they adjust.
    let n = params.len();
    let scale: Vec<f64> = params
        .iter()
        .zip(&p0)
        .map(|(p, v)| match p {
            Param::LuaCFunc(_) => v.abs().max(get(base, &Param::LuaCCall).abs()),
            _ => v.abs(),
        })
        .map(|v| v.max(0.01))
        .collect();
    let mut a = vec![vec![0.0; n]; n];
    let mut b = vec![0.0; n];
    for (xr, y) in &rows {
        let w = 1.0 / (y * y);
        for i in (0..n).filter(|i| xr[*i] != 0.0) {
            let xi = xr[i] * scale[i];
            b[i] += w * xi * y;
            for j in (0..n).filter(|j| xr[*j] != 0.0) {
                a[i][j] += w * xi * xr[j] * scale[j];
            }
        }
    }
    for i in 0..n {
        a[i][i] += opts.prior;
        b[i] += opts.prior * p0[i] / scale[i];
    }
    let x: Vec<f64> = nnls(&a, &b).iter().zip(&scale).map(|(z, s)| z * s).collect();
    let mut out = base.clone();
    for (p, v) in params.iter().zip(&x) {
        set(&mut out, p, *v);
    }
    out.estimated = false;
    let report_rows: Vec<FitRow> = data
        .iter()
        .map(|(s, t, name)| FitRow {
            name: name.clone(),
            measured_ms: t * 1000.0,
            before_ms: predict(base, s) / hz * 1000.0,
            after_ms: predict(&out, s) / hz * 1000.0,
            features: s.clone(),
        })
        .collect();
    let err = |f: &dyn Fn(&FitRow) -> f64| {
        let v: Vec<f64> = report_rows
            .iter()
            .filter(|r| r.measured_ms > 0.0)
            .map(|r| (f(r) / r.measured_ms - 1.0).abs())
            .collect();
        100.0 * v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    let report = FitReport {
        error_before: err(&|r| r.before_ms),
        error_after: err(&|r| r.after_ms),
        rows: report_rows,
        parameters: params.len(),
    };
    (out, report)
}

/// Pairs device timings with simulated samples (by order; names must match).
pub fn pair(
    device: &DeviceResults,
    samples: Vec<(Sample, f64)>,
) -> anyhow::Result<Vec<(Sample, f64, String)>> {
    anyhow::ensure!(
        samples.len() >= device.results.len(),
        "the simulator produced {} samples for {} device results",
        samples.len(),
        device.results.len()
    );
    Ok(device
        .results
        .iter()
        .zip(samples)
        .map(|(r, (s, _))| (s, r.seconds, r.name.clone()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_linear_systems() {
        let x = solve(vec![vec![2.0, 1.0], vec![1.0, 3.0]], vec![5.0, 10.0]).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-12 && (x[1] - 3.0).abs() < 1e-12);
    }

    #[test]
    fn recovers_known_costs_from_synthetic_samples() {
        let base = Calibration::default_estimates();
        let mut truth = base.clone();
        set(&mut truth, &Param::LuaOp("ADD".into()), 30.0);
        set(
            &mut truth,
            &Param::Api("graphics.fillRect".into(), "per_pixel"),
            0.1,
        );
        let mk = |adds: f64, loops: f64, calls: f64, px: f64| -> Sample {
            [
                ("lua.op.ADD".to_string(), adds),
                ("lua.op.FORLOOP".to_string(), loops),
                ("api.graphics.fillRect.calls".to_string(), calls),
                ("api.graphics.fillRect.pixels".to_string(), px),
            ]
            .into_iter()
            .filter(|(_, v)| *v > 0.0)
            .collect()
        };
        let samples = [
            mk(0.0, 1e5, 0.0, 0.0),
            mk(2e5, 1e5, 0.0, 0.0),
            mk(0.0, 1e3, 1e3, 1e4),
            mk(0.0, 1e3, 1e3, 4e6),
        ];
        let data: Vec<(Sample, f64, String)> = samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    s.clone(),
                    predict(&truth, s) / truth.cpu_hz as f64,
                    format!("b{i}"),
                )
            })
            .collect();
        let (fitted, report) = fit(&base, &data, FitOptions::default());
        assert!(
            (fitted.lua_opcode("ADD") - 30.0).abs() < 0.5,
            "{}",
            fitted.lua_opcode("ADD")
        );
        assert!((fitted.api("graphics.fillRect").per_pixel - 0.1).abs() < 0.005);
        assert!(report.error_after < 1.0, "{}", report.error_after);
        assert!(report.error_before > report.error_after);
        assert!(!fitted.estimated);
    }
}
