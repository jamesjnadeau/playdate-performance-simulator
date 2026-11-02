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
