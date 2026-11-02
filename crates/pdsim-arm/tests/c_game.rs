//! Builds `examples/c-stress` with the Playdate SDK (when it and
//! `arm-none-eabi-gcc` are available) and runs it in the emulator.

use pdsim_arm::ArmGame;
use pdsim_core::{Calibration, Category, FrameStats};
use pdsim_device::{Device, Game};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn build_example() -> Option<(PathBuf, PathBuf)> {
    static BUILT: OnceLock<Option<(PathBuf, PathBuf)>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let sdk = Device::sdk_from_env()?;
            if Command::new("arm-none-eabi-gcc")
                .arg("--version")
                .output()
                .is_err()
            {
                eprintln!("arm-none-eabi-gcc not found; skipping");
                return None;
            }
            let ex = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/c-stress");
            let out = Command::new("make")
                .arg("device_bin")
                .current_dir(&ex)
                .env("PLAYDATE_SDK_PATH", &sdk)
                .output()
                .ok()?;
            assert!(
                out.status.success(),
                "make failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            Some((ex.join("Source/pdex.elf"), sdk))
        })
        .clone()
}

fn load() -> Option<ArmGame> {
    let (elf, sdk) = build_example()?;
    let data = std::env::temp_dir().join(format!("pdsim-arm-test-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let dev = Device::new(
        Calibration::default_estimates(),
        elf.parent().unwrap(),
        &data,
        Some(sdk.clone()),
    )
    .shared();
    Some(ArmGame::load(dev, &elf, Some(&sdk), true).unwrap())
}

#[test]
fn c_example_runs_with_block_costing() {
    let Some(mut g) = load() else {
        eprintln!("PLAYDATE_SDK_PATH not set; skipping");
        return;
    };
    let frames: Vec<FrameStats> = (0..61).map(|_| g.frame(None).unwrap()).collect();
    let f = &frames[10];
    assert!(
        f.cycles.get(Category::C) > 50_000,
        "C cycles {}",
        f.cycles.get(Category::C)
    );
    assert!(f.cycles.get(Category::Gfx) > 0);
    assert!(!f.over_budget);
    // The game's 60th frame sorts a large array.
    assert!(frames[59].over_budget, "spike frame: {} ms", frames[59].work_ms);
    let d = g.device().borrow();
    assert!(d.log.iter().any(|l| l.contains("400 particles")), "{:?}", d.log);
    let top: Vec<String> = d.hotspots.top(20).into_iter().map(|h| h.name).collect();
    assert!(top.iter().any(|n| n == "qsort"), "{top:?}");
    assert!(top.iter().any(|n| n == "[graphics.fillRect]"), "{top:?}");
    // Sprites (moved by their C update callbacks) are drawn on row 200.
    let frame = d.gfx.display();
    assert!((0..400).any(|x| !frame.get(x, 200)), "sprites drawn");
    assert!(d.c_heap >= 40_000 * 4, "heap {}", d.c_heap);
}

#[test]
fn costing_is_deterministic() {
    let Some(mut a) = load() else { return };
    let Some(mut b) = load() else { return };
    for _ in 0..5 {
        let fa = a.frame(None).unwrap();
        let fb = b.frame(None).unwrap();
        assert_eq!(fa.cycles, fb.cycles);
    }
}
