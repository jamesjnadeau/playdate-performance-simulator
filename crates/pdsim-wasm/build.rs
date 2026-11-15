//! Link arguments for the browser build (`pdsim-web.{js,wasm}`); native builds
//! are unaffected.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("emscripten") {
        return;
    }
    for arg in [
        "-sMODULARIZE=1",
        "-sEXPORT_ES6=1",
        "-sEXPORT_NAME=createPdsim",
        "-sENVIRONMENT=worker,node",
        "-sEXPORTED_RUNTIME_METHODS=FS,HEAPU8",
        "-sEXPORTED_FUNCTIONS=_pdsim_new,_pdsim_send,_pdsim_tick,_pdsim_poll,_malloc,_free",
        "-sALLOW_MEMORY_GROWTH=1",
        "-lidbfs.js",
    ] {
        println!("cargo:rustc-link-arg-bin=pdsim-web={arg}");
    }
}
