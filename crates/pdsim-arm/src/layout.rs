//! The `PlaydateAPI` struct layout, read from the SDK's C headers.
//!
//! Every member of the API structs is a pointer (4 bytes on the device):
//! either a function or another API struct. The order of the members is the
//! layout, so a small parser over `pd_api.h` and `pd_api/*.h` is enough. It
//! also records each function's parameter kinds so calls can be decoded
//! according to the ARM hard-float calling convention.

use std::collections::BTreeMap;
use std::path::Path;

/// How a value is passed or returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 32-bit integer, enum, bool or pointer (core registers / stack).
    Int,
    /// `float` (VFP single registers / stack).
    Float,
    /// `double`.
    Double,
    /// `PDRect`: four floats, a homogeneous float aggregate.
    PdRect,
    /// `LCDRect`: four ints passed by value.
    LcdRect,
    /// 64-bit integer.
    Int64,
}

/// A function member.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    pub ret: Option<Kind>,
    pub params: Vec<Kind>,
    pub variadic: bool,
}

/// A member of an API struct.
#[derive(Debug, Clone, PartialEq)]
pub enum Member {
    Function {
        name: String,
        sig: Signature,
    },
    /// Pointer to another API struct (`const struct playdate_graphics* graphics`).
    Struct {
        name: String,
        ty: String,
    },
}

/// All API structs, by struct name (`PlaydateAPI`, `playdate_graphics`, ...).
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub structs: BTreeMap<String, Vec<Member>>,
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            i += 2;
            while i + 1 < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i += 2;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = vec![];
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if c == sep && depth == 0 {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Classifies a C type (with or without a parameter name).
pub fn classify(ty: &str) -> Option<Kind> {
    let t = ty.trim();
    if t.contains('*') || t.contains('(') || t.contains('[') {
        return Some(Kind::Int);
    }
    let words: Vec<&str> = t
        .split_whitespace()
        .filter(|w| *w != "const" && *w != "struct" && *w != "unsigned" && *w != "signed")
        .collect();
    let base = words.first().copied().unwrap_or("");
    match base {
        "void" if words.len() == 1 => None,
        "float" => Some(Kind::Float),
        "double" => Some(Kind::Double),
        "PDRect" => Some(Kind::PdRect),
        "LCDRect" => Some(Kind::LcdRect),
        "int64_t" | "uint64_t" => Some(Kind::Int64),
        "long" if words.get(1) == Some(&"long") => Some(Kind::Int64),
        _ => Some(Kind::Int),
    }
}

fn parse_member(m: &str) -> Option<Member> {
    let m = m.split_whitespace().collect::<Vec<_>>().join(" ");
    if m.is_empty() {
        return None;
    }
    if let Some(p) = m.find("(*") {
        // RET (*name)(PARAMS)
        let ret = &m[..p];
        let rest = &m[p + 2..];
        let close = rest.find(')')?;
        let name = rest[..close].trim().to_string();
        let params_s = rest[close + 1..].trim();
        let params_s = params_s.strip_prefix('(')?.strip_suffix(')')?;
        let mut params = vec![];
        let mut variadic = false;
        for p in split_top(params_s, ',') {
            let p = p.trim();
            if p == "..." {
                variadic = true;
            } else if p == "void" || p.is_empty() {
            } else if let Some(k) = classify(p) {
                params.push(k);
            }
        }
        return Some(Member::Function {
            name,
            sig: Signature {
                ret: classify(ret),
                params,
                variadic,
            },
        });
    }
    // const struct playdate_xxx* name
    let words: Vec<&str> = m
        .split(|c: char| c.is_whitespace() || c == '*')
        .filter(|w| !w.is_empty())
        .collect();
    if let Some(i) = words.iter().position(|w| *w == "struct") {
        let ty = words.get(i + 1)?.to_string();
        let name = words.last()?.to_string();
        return Some(Member::Struct { name, ty });
    }
    None
}

impl Layout {
    /// Parses header text (all API headers concatenated).
    pub fn parse(text: &str) -> Layout {
        let src = strip_comments(text);
        let mut structs = BTreeMap::new();
        let mut rest = src.as_str();
        while let Some(p) = rest.find("struct ") {
            let after = &rest[p + 7..];
            let name_end = after
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            let name = &after[..name_end];
            let tail = after[name_end..].trim_start();
            if tail.starts_with('{') && (name.starts_with("playdate_") || name == "PlaydateAPI") {
                let body_start = after.len() - tail.len() + 1;
                let mut depth = 1;
                let mut end = body_start;
                for (i, c) in after[body_start..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = body_start + i;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let body = &after[body_start..end];
                let members: Vec<Member> = split_top(body, ';')
                    .iter()
                    .filter_map(|m| parse_member(m))
                    .collect();
                structs.insert(name.to_string(), members);
                rest = &after[end..];
            } else {
                rest = after;
            }
        }
        Layout { structs }
    }

    /// Reads the layout from an SDK.
    pub fn from_sdk(sdk: &Path) -> std::io::Result<Layout> {
        let api = sdk.join("C_API");
        let mut text = std::fs::read_to_string(api.join("pd_api.h"))?;
        let mut files: Vec<_> = std::fs::read_dir(api.join("pd_api"))?
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        for f in files {
            if f.extension().is_some_and(|e| e == "h") {
                text.push('\n');
                text.push_str(&std::fs::read_to_string(f)?);
            }
        }
        Ok(Layout::parse(&text))
    }

    /// Flattens the tree under `PlaydateAPI` into `(path, signature)` pairs
    /// such as `("graphics.fillRect", ...)` in a stable order.
    pub fn functions(&self) -> Vec<(String, Signature)> {
        let mut out = vec![];
        self.walk("PlaydateAPI", "", &mut out, 0);
        out
    }

    fn walk(&self, ty: &str, prefix: &str, out: &mut Vec<(String, Signature)>, depth: usize) {
        if depth > 6 {
            return;
        }
        let Some(members) = self.structs.get(ty) else {
            return;
        };
        for m in members {
            match m {
                Member::Function { name, sig } => out.push((format!("{prefix}{name}"), sig.clone())),
                Member::Struct { name, ty } => self.walk(ty, &format!("{prefix}{name}."), out, depth + 1),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        struct playdate_sys {
            void* (*realloc)(void* ptr, size_t size); // comment
            int (*formatString)(char **ret, const char *fmt, ...);
            float (*getCrankAngle)(void);
            void (*setSerialMessageCallback)(void (*callback)(const char* data));
        };
        struct playdate_graphics {
            const struct playdate_video* video;
            void (*fillEllipse)(int x, int y, int width, int height, float startAngle, float endAngle, LCDColor color);
            /* block
               comment */
            int (*checkMaskCollision)(LCDBitmap* b1, int x1, LCDRect rect);
        };
        struct playdate_video { int (*getInfo)(LCDVideoPlayer* p); };
        struct PlaydateAPI {
            const struct playdate_sys* system;
            const struct playdate_graphics* graphics;
        };
        struct playdate_sprite { void (*setBounds)(LCDSprite *sprite, PDRect bounds); PDRect (*getBounds)(LCDSprite *sprite); };
    "#;

    #[test]
    fn parses_members_in_order() {
        let l = Layout::parse(SAMPLE);
        let f = l.functions();
        let names: Vec<&str> = f.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "system.realloc",
                "system.formatString",
                "system.getCrankAngle",
                "system.setSerialMessageCallback",
                "graphics.video.getInfo",
                "graphics.fillEllipse",
                "graphics.checkMaskCollision"
            ]
        );
        let fe = &f[5].1;
        use Kind::*;
        assert_eq!(fe.params, vec![Int, Int, Int, Int, Float, Float, Int]);
        assert!(f[1].1.variadic);
        assert_eq!(f[2].1.ret, Some(Float));
        assert_eq!(f[3].1.params, vec![Int]);
        assert_eq!(f[6].1.params, vec![Int, Int, LcdRect]);
        let sp = &l.structs["playdate_sprite"];
        assert!(matches!(&sp[1], Member::Function { sig, .. } if sig.ret == Some(PdRect)));
    }

    #[test]
    fn real_sdk_headers_parse() {
        let Some(sdk) = std::env::var_os("PLAYDATE_SDK_PATH").map(std::path::PathBuf::from) else {
            return;
        };
        let l = Layout::from_sdk(&sdk).unwrap();
        let f = l.functions();
        let get = |n: &str| {
            f.iter()
                .find(|(x, _)| x == n)
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| panic!("{n}"))
        };
        assert_eq!(f[0].0, "system.realloc");
        assert!(f.len() > 400, "{} functions", f.len());
        assert_eq!(
            get("graphics.drawEllipse").params,
            vec![
                Kind::Int,
                Kind::Int,
                Kind::Int,
                Kind::Int,
                Kind::Int,
                Kind::Float,
                Kind::Float,
                Kind::Int
            ]
        );
        assert_eq!(
            get("sprite.moveTo").params,
            vec![Kind::Int, Kind::Float, Kind::Float]
        );
        assert_eq!(get("sprite.getBounds").ret, Some(Kind::PdRect));
        assert!(get("system.logToConsole").variadic);
        // The graphics struct starts with the video pointer, then clear.
        let g = &l.structs["playdate_graphics"];
        assert!(matches!(&g[0], Member::Struct { name, .. } if name == "video"));
        assert!(matches!(&g[1], Member::Function { name, .. } if name == "clear"));
    }
}
