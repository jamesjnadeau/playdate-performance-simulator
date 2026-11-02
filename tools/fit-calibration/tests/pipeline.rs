//! The whole calibration pipeline without hardware: a perturbed profile plays
//! the part of the device, and fitting from the defaults must recover it.

use fit_calibration::*;
use pdsim_core::Calibration;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn truth() -> Calibration {
    let mut t = Calibration::default_estimates();
    for v in t.lua.opcodes.values_mut() {
        *v *= 1.3;
    }
    t.lua.opcodes.insert("ADD".into(), 25.0);
    t.lua.opcodes.insert("GETFIELD".into(), 55.0);
    t.lua.c_function_call = 300.0;
    let mut fr = t.api("graphics.fillRect");
    fr.per_pixel *= 2.0;
    fr.base *= 1.5;
    t.api.insert("graphics.fillRect".into(), fr);
    let mut dt = t.api("graphics.drawText");
    dt.per_item *= 1.4;
    t.api.insert("graphics.drawText".into(), dt);
    t.arm.classes.insert("div".into(), 11.0);
    t.arm.classes.insert("load".into(), 3.0);
    t
}

#[test]
fn lua_bench_features_explain_measured_time() {
    // What the game measured with getElapsedTime equals the cost model
    // applied to the recorded features (nothing is charged unrecorded).
    let cal = truth();
    let bench = root().join("calibration/bench-lua");
    let dev = run_as_device(&bench, &cal, 0.05, 1, None).unwrap();
    let samples = simulate(&bench, &cal, 0.05, 1, dev.results.len(), None).unwrap();
    for (r, (s, _)) in dev.results.iter().zip(&samples) {
        let p = predict(&cal, s) / cal.cpu_hz as f64;
        assert!(
            (p / r.seconds - 1.0).abs() < 0.01,
            "{}: predicted {p} measured {}",
            r.name,
            r.seconds
        );
    }
}

#[test]
fn fitting_recovers_a_perturbed_profile() {
    let base = Calibration::default_estimates();
    let t = truth();
    let mut data = vec![];
    let lua = root().join("calibration/bench-lua");
    let dev = run_as_device(&lua, &t, 0.05, 1, None).unwrap();
    data.extend(
        pair(
            &dev,
            simulate(&lua, &base, 0.05, 1, dev.results.len(), None).unwrap(),
        )
        .unwrap(),
    );
    // The C bench needs the SDK (for the API layout) and a built pdex.elf.
    let c = root().join("calibration/bench-c");
    if let Some(sdk) = pdsim_device::Device::sdk_from_env().filter(|_| c.join("Source/pdex.elf").exists()) {
        let dev = run_as_device(&c, &t, 1.0, 1, Some(sdk.clone())).unwrap();
        data.extend(
            pair(
                &dev,
                simulate(&c, &base, 1.0, 1, dev.results.len(), Some(sdk)).unwrap(),
            )
            .unwrap(),
        );
    }
    let (fitted, report) = fit(&base, &data, FitOptions::default());
    eprintln!(
        "error before {:.1}% after {:.2}% ({} params, {} runs)",
        report.error_before,
        report.error_after,
        report.parameters,
        data.len()
    );
    assert!(
        report.error_before > 10.0,
        "the perturbation should be visible: {}",
        report.error_before
    );
    assert!(report.error_after < 3.0, "fit error {}", report.error_after);
    // Well-identified costs come back close to the truth.
    let rel = |a: f64, b: f64| (a / b - 1.0).abs();
    assert!(
        rel(
            fitted.api("graphics.fillRect").per_pixel,
            t.api("graphics.fillRect").per_pixel
        ) < 0.1
    );
    assert!(
        rel(fitted.lua_opcode("ADD"), 25.0) < 0.25,
        "ADD {}",
        fitted.lua_opcode("ADD")
    );
}
