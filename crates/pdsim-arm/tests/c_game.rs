//! Builds `examples/c-stress` with the Playdate SDK (when it and
//! `arm-none-eabi-gcc` are available) and runs it in the emulator.

use pdsim_arm::{ArmGame, Backend};
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
    Some(load_elf(&elf, &sdk, Backend::default()))
}

/// Loads the already-built `examples/c-stress` ELF without rebuilding it.
/// Needs the SDK (for the API layout) and the ELF; `None` when either is
/// missing.
fn load_c_stress(backend: Backend) -> Option<ArmGame> {
    let sdk = Device::sdk_from_env()?;
    let elf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/c-stress/Source/pdex.elf");
    if !elf.exists() {
        return None;
    }
    Some(load_elf(&elf, &sdk, backend))
}

fn load_elf(elf: &Path, sdk: &Path, backend: Backend) -> ArmGame {
    let data = std::env::temp_dir().join(format!("pdsim-arm-test-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let dev = Device::new(
        Calibration::default_estimates(),
        elf.parent().unwrap(),
        &data,
        Some(sdk.to_path_buf()),
    )
    .shared();
    ArmGame::load(dev, elf, Some(sdk), true, backend).unwrap()
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

/// Per-frame cycle totals of the first five frames of `examples/c-stress`,
/// recorded on Unicorn before the `Cpu` trait existed (at ea3c424).
const C_STRESS_FIRST_5_FRAMES: [u64; 5] = [500087, 500020, 500090, 500070, 500084];

/// Checks nesting indirectly, through the cycle totals: a nested call from a
/// trampoline that failed to restore the outer call would change them. The
/// direct check (nested `emu_start` returns to the outer run) is the
/// `nested_emu_start_from_a_hook_returns_to_the_outer_run` unit test in
/// `unicorn_cpu.rs`.
#[test]
fn nested_call_from_trampoline_restores_the_outer_call() {
    // examples/c-stress registers a sprite update function; run 5 frames through the trait and
    // compare the per-frame cycle totals with the values recorded before this change.
    let Some(mut g) = load_c_stress(Backend::default()) else {
        eprintln!("PLAYDATE_SDK_PATH not set or examples/c-stress not built; skipping");
        return;
    };
    let cycles: Vec<u64> = (0..5).map(|_| g.frame(None).unwrap().cycles.total()).collect();
    assert_eq!(cycles, C_STRESS_FIRST_5_FRAMES);
}
