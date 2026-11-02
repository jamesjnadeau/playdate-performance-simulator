//! Compiles the vendored, patched Lua 5.4 (see vendor/README.md) and links it
//! statically. mlua-sys is built with its `external` feature, so this is the
//! Lua that mlua uses.

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from("vendor/lua-5.4.9");
    let mut b = cc::Build::new();
    b.include(&dir).warnings(false).define("LUA_COMPAT_5_3", None);
    // A fixed string-hash seed (Lua randomizes it per state), so table
    // iteration order, and therefore cycle counts, are the same every run.
    b.define("luai_makeseed(L)", Some("((unsigned int)0x5eed)"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "linux" => {
            b.define("LUA_USE_LINUX", None);
        }
        "macos" => {
            b.define("LUA_USE_MACOSX", None);
        }
        "windows" => {
            b.define("LUA_USE_WINDOWS", None);
        }
        _ => {
            b.define("LUA_USE_POSIX", None);
        }
    }
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        // The VM is the hot loop even in debug builds.
        b.opt_level(2);
    }
    for e in std::fs::read_dir(&dir).expect("vendored lua") {
        let p = e.expect("dir entry").path();
        if p.extension().is_some_and(|x| x == "c") {
            println!("cargo:rerun-if-changed={}", p.display());
            b.file(&p);
        }
    }
    for e in std::fs::read_dir(&dir).expect("vendored lua") {
        let p = e.expect("dir entry").path();
        if p.extension().is_some_and(|x| x == "h") {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
    b.compile("pdsimlua");
    if target_os == "linux" {
        println!("cargo:rustc-link-lib=m");
    }
}
