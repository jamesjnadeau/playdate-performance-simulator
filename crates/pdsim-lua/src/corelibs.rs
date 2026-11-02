//! Stand-ins for the SDK's CoreLibs, used when no SDK is available.

macro_rules! corelib {
    ($($name:literal),* $(,)?) => {
        /// `(name, source)` pairs, e.g. `("timer", "...")`.
        pub const FALLBACK: &[(&str, &str)] = &[
            $(($name, include_str!(concat!("../lua/corelibs/", $name, ".lua")))),*
        ];
    };
}

corelib!(
    "object",
    "timer",
    "frameTimer",
    "easing",
    "crank",
    "math",
    "string",
    "graphics",
    "sprites",
    "ui",
    "animation",
    "animator",
    "nineslice",
    "qrcode",
    "keyboard",
    "accelerometer",
    "logic",
    "save",
    "strict",
    "utilities/where",
    "3d",
);

/// Source of a fallback CoreLib by name (`"timer"`, ...).
pub fn fallback(name: &str) -> Option<&'static str> {
    FALLBACK.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}
