//! The plan's headless end-to-end check: run the example, double its
//! workload, and check the report.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run_example(work: u32, tag: &str) -> serde_json::Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tmp = std::env::temp_dir().join(format!("pdsim-cli-{tag}-{}", std::process::id()));
    let data = tmp.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("config.json"),
        format!("{{\"work\": {work}, \"spikeEvery\": 60}}"),
    )
    .unwrap();
    let report: PathBuf = tmp.join("r.json");
    let out = Command::new(env!("CARGO_BIN_EXE_pdsim"))
        .args([
            "run",
            "examples/lua-stress",
            "--frames",
            "130",
            "-q",
            "--no-profile",
        ])
        .arg("--data-dir")
        .arg(&data)
        .arg("--report")
        .arg(&report)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);
    v
}

#[test]
fn doubling_the_workload_and_flagging_spikes() {
    let one = run_example(1, "w1");
    let two = run_example(2, "w2");
    let p50 = |v: &serde_json::Value| v["p50_work_ms"].as_f64().unwrap();
    let ratio = p50(&two) / p50(&one);
    assert!(
        ratio > 1.7 && ratio < 2.3,
        "typical frame should roughly double: {ratio}"
    );
    let spikes: Vec<u64> = one["over_budget_frame_indices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert!(spikes.contains(&59) && spikes.contains(&119), "{spikes:?}");
    assert!(one["dropped_frames"].as_u64().unwrap() >= 2);
}

#[test]
fn ci_limits_fail_the_run() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_pdsim"))
        .args([
            "run",
            "examples/lua-stress",
            "--frames",
            "70",
            "-q",
            "--no-profile",
            "--max-over-budget",
            "0",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("over budget"));
}

/// Runs `examples/c-stress` for 60 frames with `args` and returns the report
/// file, or `None` when there is no built ELF or no SDK (the SDK supplies
/// `pd_api.h`, which C games need).
#[cfg(all(feature = "c-games", feature = "thumb"))]
fn run_c_stress(args: &[&str]) -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("examples/c-stress/Source/pdex.elf").exists()
        || pdsim_device::Device::sdk_from_env().is_none()
    {
        eprintln!("examples/c-stress/Source/pdex.elf or PLAYDATE_SDK_PATH missing; skipping");
        return None;
    }
    let tag = args.join("_");
    let tmp = std::env::temp_dir().join(format!("pdsim-cli-c-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let report = tmp.join("r.json");
    let out = Command::new(env!("CARGO_BIN_EXE_pdsim"))
        .args(["run", "examples/c-stress", "--frames", "60", "-q"])
        .args(args)
        .arg("--data-dir")
        .arg(tmp.join("data"))
        .arg("--report")
        .arg(&report)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some(report)
}

#[cfg(all(feature = "c-games", feature = "thumb"))]
#[test]
fn thumb_and_unicorn_reports_are_identical() {
    let (Some(a), Some(b)) = (
        run_c_stress(&["--cpu", "unicorn"]),
        run_c_stress(&["--cpu", "thumb"]),
    ) else {
        return;
    };
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
}

/// Asks for a CPU backend this build doesn't have: the message and exit code
/// are part of the CLI's contract.
#[cfg(any(not(feature = "thumb"), not(feature = "c-games")))]
fn missing_backend(name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_pdsim"))
        .args(["run", "examples/lua-stress", "--frames", "1", "-q", "--cpu", name])
        .current_dir(&root)
        .output()
        .unwrap();
    // In a workspace build, another crate (pdsim-wasm) can switch the backend
    // on in pdsim-runner even though pdsim-cli's own feature is off.
    if out.status.success() {
        return;
    }
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        err.trim(),
        format!("pdsim: this build has no {name} CPU (rebuild with --features {name})")
    );
}

#[cfg(not(feature = "thumb"))]
#[test]
fn a_backend_that_is_not_built_in_exits_2() {
    missing_backend("thumb");
}

#[cfg(not(feature = "c-games"))]
#[test]
fn unicorn_not_built_in_exits_2() {
    missing_backend("unicorn");
}

/// A Lua run with no `--cpu` works in every build, even one without C games.
#[test]
fn a_lua_run_needs_no_cpu_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_pdsim"))
        .args(["run", "examples/lua-stress", "--frames", "5", "-q"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}
