//! The Playdate C API, implemented on the simulated device.
//!
//! Each call arrives at a trampoline; [`dispatch`] decodes the arguments from
//! the function's signature (read from the SDK headers), runs the Rust
//! implementation, charges its calibrated cost, and sets the return value.
//! Functions with no implementation return 0 and are reported once.

use crate::abi::{self, Arg};
use crate::emu::*;
use pdsim_core::{Category, Work};
use pdsim_gfx::sprite::{CollisionResponse, DrawItem, RectF};
use pdsim_gfx::*;
use unicorn_engine::Unicorn;

type Uc<'a> = Unicorn<'a, ArmState>;

/// An open file.
#[derive(Debug, Clone, Default)]
pub struct OpenFile {
    pub path: String,
    pub data: Vec<u8>,
    pub pos: usize,
    pub write: bool,
    pub dirty: bool,
}

fn bh(id: u32) -> u32 {
    BITMAP_BASE + id * 16
}
fn bid(h: u32) -> Option<u32> {
    (BITMAP_BASE..FONT_BASE)
        .contains(&h)
        .then(|| (h - BITMAP_BASE) / 16)
}
fn fh(id: u32) -> u32 {
    FONT_BASE + id * 16
}
fn fid(h: u32) -> Option<u32> {
    (FONT_BASE..SPRITE_BASE)
        .contains(&h)
        .then(|| (h - FONT_BASE) / 16)
}
fn sh(id: u32) -> u32 {
    SPRITE_BASE + id * 16
}
fn sid(h: u32) -> Option<u32> {
    (SPRITE_BASE..FILE_BASE)
        .contains(&h)
        .then(|| (h - SPRITE_BASE) / 16)
}

fn color(uc: &Uc, v: u32) -> Color {
    if v <= 3 {
        return Color::from_index(v as i64);
    }
    let b = uc.bytes(v, 16);
    if b.len() < 16 {
        return Color::Black;
    }
    let mut bits = [0u8; 8];
    let mut mask = [0u8; 8];
    bits.copy_from_slice(&b[..8]);
    mask.copy_from_slice(&b[8..16]);
    Color::Pattern(Pattern { bits, mask })
}

fn rf(a: &[Arg], i: usize) -> f32 {
    a.get(i).map_or(0.0, |x| x.f())
}
fn ri(a: &[Arg], i: usize) -> i32 {
    a.get(i).map_or(0, |x| x.i())
}
fn ru(a: &[Arg], i: usize) -> u32 {
    a.get(i).map_or(0, |x| x.u())
}

/// Runs a graphics call with the frame mirror kept in sync and charges it.
fn gfx<F: FnOnce(&mut Uc, &mut Graphics) -> Work>(uc: &mut Uc, name: &str, f: F) {
    ArmGame::before_gfx(uc);
    let dev = uc.get_data().dev.clone();
    let w = {
        let mut d = dev.borrow_mut();
        f(uc, &mut d.gfx)
    };
    ArmGame::after_gfx(uc);
    uc.get_data_mut().charge_api(Category::Gfx, name, w);
}

fn charge(uc: &mut Uc, cat: Category, name: &str, w: Work) {
    uc.get_data_mut().charge_api(cat, name, w);
}

/// Text passed as (pointer, length, encoding).
fn text(uc: &Uc, ptr: u32, len: u32, enc: u32) -> String {
    let len = len.min(1 << 16) as usize;
    let b = uc.bytes(ptr, len);
    match enc {
        // kUTF16Encoding
        2 => {
            let u: Vec<u16> = b
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|c| *c != 0)
                .collect();
            String::from_utf16_lossy(&u)
        }
        _ => {
            let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            String::from_utf8_lossy(&b[..end]).into_owned()
        }
    }
}

/// Writes a string into scratch memory and returns its address.
fn put_str(uc: &mut Uc, s: &str) -> u32 {
    let a = uc.get_data_mut().scratch(s.len() as u32 + 1);
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    uc.write(a, &b);
    a
}

fn heap_alloc(uc: &mut Uc, n: u32) -> u32 {
    uc.get_data_mut().heap.alloc(n).unwrap_or(0)
}

/// Writes an array of u32 to the heap; returns its address and writes the
/// count to `len_ptr`.
fn put_array(uc: &mut Uc, v: &[u32], len_ptr: u32) -> u32 {
    uc.w32(len_ptr, v.len() as u32);
    if v.is_empty() {
        return 0;
    }
    let a = heap_alloc(uc, v.len() as u32 * 4);
    let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    uc.write(a, &b);
    a
}

fn resolve_font(uc: &Uc, path: &str) -> Option<std::path::PathBuf> {
    let st = uc.get_data();
    let d = st.dev.borrow();
    if let Some(name) = path.strip_prefix("/System/Fonts/") {
        let stem = name.trim_end_matches(".pft").trim_end_matches(".fnt");
        let sdk = d.sdk.as_ref()?.join("Resources/Fonts");
        fn find(dir: &std::path::Path, stem: &str) -> Option<std::path::PathBuf> {
            for e in std::fs::read_dir(dir).ok()?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if let Some(f) = find(&p, stem) {
                        return Some(f);
                    }
                } else if p.file_stem().is_some_and(|s| s == stem)
                    && p.extension().is_some_and(|e| e == "fnt")
                {
                    return Some(p);
                }
            }
            None
        }
        return find(&sdk, stem);
    }
    let p = path.trim_end_matches(".pft");
    d.fs.resolve_with_ext(p, &["fnt"])
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Seconds between the Unix epoch and the Playdate epoch (2000-01-01).
const PD_EPOCH: i64 = 946_684_800;

/// Handles one API call.
pub fn dispatch(uc: &mut Uc, idx: usize) {
    let Some((name, sig)) = uc.get_data().funcs.get(idx).cloned() else {
        return;
    };
    // Decode fixed args; format variadic strings while the cursor is alive.
    let (a, formatted) = {
        let regs = Regs(uc);
        let (args, mut cur) = abi::decode(&regs, &sig);
        let formatted = if sig.variadic {
            // The format string is the last fixed argument.
            let fmt = args.last().map(|x| uc.cstr(x.u())).unwrap_or_default();
            Some(abi::format(&fmt, &mut cur, &|p| uc.cstr(p)))
        } else {
            None
        };
        (args, formatted)
    };
    let ret = run(uc, &name, &a, formatted).or_else(|| stub(uc, idx, &name, &sig));
    let ret = match ret {
        Some(r) => r,
        None => {
            let first = uc.get_data_mut().unimplemented.insert(idx);
            if first {
                let msg = format!("[pdsim] C API {name} is not simulated (returns 0)");
                uc.get_data().dev.borrow_mut().log_line(msg);
            }
            charge(uc, Category::Other, &name, Work::default());
            Ret::I(0)
        }
    };
    set_ret(uc, sig.ret, ret);
}

/// Sound, Lua, JSON, scoreboards and networking are not simulated. Sound
/// objects still get non-null handles, and playing charges the calibrated
/// sound cost, so games keep running and their audio calls show up.
fn stub(uc: &mut Uc, idx: usize, name: &str, sig: &crate::layout::Signature) -> Option<Ret> {
    if !name.starts_with("sound.") {
        return None;
    }
    let leaf = name.rsplit('.').next().unwrap_or("");
    if uc.get_data_mut().unimplemented.insert(idx) && !uc.get_data().unimplemented.contains(&usize::MAX) {
        uc.get_data_mut().unimplemented.insert(usize::MAX);
        let msg =
            "[pdsim] sound is not simulated: sound calls succeed silently and charge a flat cost".to_string();
        uc.get_data().dev.borrow_mut().log_line(msg);
    }
    if leaf.starts_with("play") {
        charge(
            uc,
            Category::Audio,
            if leaf.contains("Note") {
                "sound.playNote"
            } else {
                "sound.play"
            },
            Work::default(),
        );
        return Some(Ret::I(1));
    }
    charge(uc, Category::Audio, "sound.update", Work::default());
    match sig.ret {
        Some(crate::layout::Kind::Int)
            if leaf.starts_with("new")
                || leaf.starts_with("load")
                || leaf.starts_with("copy")
                || leaf == "getDefaultChannel" =>
        {
            let h = SOUND_BASE + 16 * (idx as u32 & 0xffff);
            Some(Ret::I(h))
        }
        Some(crate::layout::Kind::Float) => Some(Ret::F(0.0)),
        _ => Some(Ret::I(0)),
    }
}

fn run(uc: &mut Uc, name: &str, a: &[Arg], fmt: Option<String>) -> Option<Ret> {
    let dev = uc.get_data().dev.clone();
    let none = Some(Ret::None);
    let r = match name {
        // ------------------------------------------------------------ system
        "system.realloc" => {
            let (ptr, size) = (ru(a, 0), ru(a, 1));
            charge(
                uc,
                Category::Other,
                "system.realloc",
                Work {
                    bytes: size as u64,
                    ..Default::default()
                },
            );
            let st = uc.get_data_mut();
            if size == 0 {
                if ptr != 0 {
                    st.heap.free(ptr);
                }
                return Some(Ret::I(0));
            }
            let old = if ptr != 0 { st.heap.size_of(ptr) } else { None };
            let Some(new) = st.heap.alloc(size) else {
                return Some(Ret::I(0));
            };
            if let Some(n) = old {
                let data = uc.bytes(ptr, n.min(size) as usize);
                uc.write(new, &data);
                uc.get_data_mut().heap.free(ptr);
            }
            Some(Ret::I(new))
        }
        "system.formatString" => {
            let s = fmt.unwrap_or_default();
            charge(
                uc,
                Category::Other,
                "system.formatString",
                Work {
                    bytes: s.len() as u64,
                    ..Default::default()
                },
            );
            let p = heap_alloc(uc, s.len() as u32 + 1);
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            uc.write(p, &b);
            uc.w32(ru(a, 0), p);
            Some(Ret::I(s.len() as u32))
        }
        "system.logToConsole" => {
            let s = fmt.unwrap_or_default();
            charge(
                uc,
                Category::Other,
                "system.logToConsole",
                Work {
                    bytes: s.len() as u64,
                    ..Default::default()
                },
            );
            dev.borrow_mut().log_line(s);
            none
        }
        "system.error" => {
            let s = fmt.unwrap_or_default();
            dev.borrow_mut().log_line(format!("error: {s}"));
            uc.get_data_mut().error = Some(format!("pd->system->error: {s}"));
            none
        }
        "system.getLanguage" => Some(Ret::I(0)),
        "system.getCurrentTimeMilliseconds" => {
            uc.get_data_mut().sync();
            Some(Ret::I(dev.borrow().now_ms() as u32))
        }
        "system.getSecondsSinceEpoch" => {
            uc.get_data_mut().sync();
            let ms = dev.borrow().now_ms();
            let t = (pdsim_device_epoch() - PD_EPOCH as f64) + ms / 1000.0;
            uc.w32(ru(a, 0), (t.fract() * 1000.0) as u32);
            Some(Ret::I(t as u32))
        }
        "system.drawFPS" => {
            let fps = dev.borrow().last_frame.as_ref().map_or(0.0, |f| {
                if f.frame_ms > 0.0 {
                    1000.0 / f.frame_ms
                } else {
                    0.0
                }
            });
            let (x, y) = (ri(a, 0), ri(a, 1));
            gfx(uc, "system.drawFPS", |_, g| {
                let f = g.system_font();
                let s = format!("{}", fps.round());
                let (w, h) = g.text_size(f, &s);
                g.push_context(None);
                g.fill_rect(Rect::new(x, y, w + 4, h + 2), Color::White);
                g.draw_text(f, &s, x + 2, y + 1);
                g.pop_context();
                Work::default()
            });
            none
        }
        "system.setUpdateCallback" => {
            uc.get_data_mut().update_cb = Some((ru(a, 0), ru(a, 1)));
            none
        }
        "system.getButtonState" => {
            charge(uc, Category::Other, "system.getButtonState", Work::default());
            let (c, p, r) = {
                let d = dev.borrow();
                (d.input.current, d.input.pushed, d.input.released)
            };
            uc.w32(ru(a, 0), c as u32);
            uc.w32(ru(a, 1), p as u32);
            uc.w32(ru(a, 2), r as u32);
            none
        }
        "system.setPeripheralsEnabled"
        | "system.setAutoLockDisabled"
        | "system.setMenuImage"
        | "system.clearICache"
        | "system.setSerialMessageCallback"
        | "system.removeAllMenuItems"
        | "system.removeMenuItem"
        | "system.setMenuItemValue"
        | "system.setMenuItemTitle"
        | "system.setMenuItemUserdata"
        | "system.restartGame"
        | "system.exitToLauncher"
        | "system.getServerTime" => none,
        "system.getAccelerometer" => {
            let (x, y, z) = dev.borrow().input.accel;
            uc.wf32(ru(a, 0), x);
            uc.wf32(ru(a, 1), y);
            uc.wf32(ru(a, 2), z);
            none
        }
        "system.getCrankChange" => {
            charge(uc, Category::Other, "system.getCrankChange", Work::default());
            Some(Ret::F(dev.borrow().input.crank_change))
        }
        "system.getCrankAngle" => {
            charge(uc, Category::Other, "system.getCrankAngle", Work::default());
            Some(Ret::F(dev.borrow().input.crank_angle))
        }
        "system.isCrankDocked" => Some(Ret::I(dev.borrow().input.crank_docked as u32)),
        "system.setCrankSoundsDisabled"
        | "system.getFlipped"
        | "system.getReduceFlashing"
        | "system.getTimezoneOffset"
        | "system.getMenuItemValue"
        | "system.getMenuItemUserdata"
        | "system.getLaunchArgs"
        | "system.sendMirrorData"
        | "system.getPowerStatus"
        | "system.getLocalizedText"
        | "system.parseString"
        | "system.vaFormatString" => Some(Ret::I(0)),
        "system.addMenuItem" | "system.addCheckmarkMenuItem" | "system.addOptionsMenuItem" => {
            Some(Ret::I(MENU_BASE + 16))
        }
        "system.getMenuItemTitle" => Some(Ret::I(put_str(uc, ""))),
        "system.getElapsedTime" => {
            uc.get_data_mut().sync();
            charge(uc, Category::Other, "system.getElapsedTime", Work::default());
            let t = dev.borrow_mut().elapsed_s_sample();
            Some(Ret::F(t as f32))
        }
        "system.resetElapsedTime" => {
            uc.get_data_mut().sync();
            dev.borrow_mut().reset_elapsed();
            none
        }
        "system.getBatteryPercentage" => Some(Ret::F(100.0)),
        "system.getBatteryVoltage" => Some(Ret::F(4.2)),
        "system.getVolume" => Some(Ret::F(1.0)),
        "system.shouldDisplay24HourTime" => Some(Ret::I(1)),
        "system.convertEpochToDateTime" => {
            let secs = ru(a, 0) as i64 + PD_EPOCH;
            let days = secs.div_euclid(86400);
            let rem = secs.rem_euclid(86400);
            let (y, m, d) = civil_from_days(days);
            let wd = ((days + 4).rem_euclid(7) + 1) as u8; // 1 = Sunday
            let p = ru(a, 1);
            let b = [
                (y as u16).to_le_bytes()[0],
                (y as u16).to_le_bytes()[1],
                m as u8,
                d as u8,
                wd,
                (rem / 3600) as u8,
                (rem % 3600 / 60) as u8,
                (rem % 60) as u8,
            ];
            uc.write(p, &b);
            none
        }
        "system.convertDateTimeToEpoch" => {
            let b = uc.bytes(ru(a, 0), 8);
            if b.len() < 8 {
                return Some(Ret::I(0));
            }
            let y = u16::from_le_bytes([b[0], b[1]]) as i64;
            let days = days_from_civil(y, b[2] as u32, b[3] as u32);
            let secs = days * 86400 + b[5] as i64 * 3600 + b[6] as i64 * 60 + b[7] as i64 - PD_EPOCH;
            Some(Ret::I(secs as u32))
        }
        "system.setButtonCallback" => {
            uc.get_data_mut().button_cb = Some((ru(a, 0), ru(a, 1)));
            none
        }
        "system.delay" => {
            uc.get_data_mut().sync();
            let mut d = dev.borrow_mut();
            let c = ru(a, 0) as f64 * d.cal.cpu_hz as f64 / 1000.0;
            d.charge(Category::Other, c);
            none
        }
        "system.getSystemInfo" => {
            let p = uc.get_data_mut().scratch(12);
            uc.w32(p, 30102);
            uc.w32(p + 4, 0);
            uc.w32(p + 8, 30102);
            Some(Ret::I(p))
        }

        // ------------------------------------------------------------ display
        "display.getWidth" => Some(Ret::I((400 / dev.borrow().gfx.scale.max(1)) as u32)),
        "display.getHeight" => Some(Ret::I((240 / dev.borrow().gfx.scale.max(1)) as u32)),
        "display.setRefreshRate" => {
            dev.borrow_mut().scheduler.set_refresh_rate(rf(a, 0));
            none
        }
        "display.getRefreshRate" => Some(Ret::F(dev.borrow().scheduler.refresh_rate())),
        "display.getFPS" => {
            let fps = dev.borrow().last_frame.as_ref().map_or(0.0, |f| {
                if f.frame_ms > 0.0 {
                    1000.0 / f.frame_ms
                } else {
                    0.0
                }
            });
            Some(Ret::F(fps as f32))
        }
        "display.setInverted" => {
            dev.borrow_mut().gfx.inverted = ru(a, 0) != 0;
            none
        }
        "display.setScale" => {
            dev.borrow_mut().gfx.scale = (ru(a, 0) as i32).clamp(1, 8);
            none
        }
        "display.setOffset" => {
            dev.borrow_mut().gfx.display_offset = (ri(a, 0), ri(a, 1));
            none
        }
        "display.setMosaic" | "display.setFlipped" => none,

        // ------------------------------------------------------------ graphics state
        "graphics.clear" => {
            let c = color(uc, ru(a, 0));
            gfx(uc, "graphics.clear", |_, g| g.clear(c));
            none
        }
        "graphics.setBackgroundColor" => {
            dev.borrow_mut().gfx.background_color = Color::from_index(ru(a, 0) as i64);
            none
        }
        "graphics.setStencil" => {
            dev.borrow_mut().gfx.state.stencil = bid(ru(a, 0)).map(|id| (id, false));
            none
        }
        "graphics.setStencilImage" => {
            dev.borrow_mut().gfx.state.stencil = bid(ru(a, 0)).map(|id| (id, ru(a, 1) != 0));
            none
        }
        "graphics.setDrawMode" => {
            let mut d = dev.borrow_mut();
            let prev = d.gfx.state.draw_mode.index();
            d.gfx.state.draw_mode = DrawMode::from_index(ru(a, 0) as i64);
            Some(Ret::I(prev as u32))
        }
        "graphics.setDrawOffset" => {
            dev.borrow_mut().gfx.state.draw_offset = (ri(a, 0), ri(a, 1));
            none
        }
        "graphics.setClipRect" => {
            dev.borrow_mut()
                .gfx
                .set_clip_rect(Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)));
            charge(uc, Category::Gfx, "graphics.setClipRect", Work::default());
            none
        }
        "graphics.setScreenClipRect" => {
            dev.borrow_mut()
                .gfx
                .set_screen_clip_rect(Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)));
            none
        }
        "graphics.clearClipRect" => {
            dev.borrow_mut().gfx.clear_clip_rect();
            none
        }
        "graphics.setLineCapStyle" => {
            dev.borrow_mut().gfx.state.line_cap = match ru(a, 0) {
                1 => LineCap::Square,
                2 => LineCap::Round,
                _ => LineCap::Butt,
            };
            none
        }
        "graphics.setFont" => {
            dev.borrow_mut().gfx.state.font = fid(ru(a, 0));
            none
        }
        "graphics.setTextTracking" => {
            dev.borrow_mut().gfx.state.tracking = Some(ri(a, 0));
            none
        }
        "graphics.getTextTracking" => {
            let d = dev.borrow();
            let f = d.gfx.current_font();
            Some(Ret::I(
                d.gfx
                    .state
                    .tracking
                    .unwrap_or_else(|| d.gfx.font(f).map_or(0, |f| f.tracking)) as u32,
            ))
        }
        "graphics.setTextLeading" => {
            dev.borrow_mut().gfx.state.leading = ri(a, 0);
            none
        }
        "graphics.pushContext" => {
            ArmGame::before_gfx(uc);
            dev.borrow_mut().gfx.push_context(bid(ru(a, 0)));
            charge(uc, Category::Gfx, "graphics.pushContext", Work::default());
            none
        }
        "graphics.popContext" => {
            dev.borrow_mut().gfx.pop_context();
            charge(uc, Category::Gfx, "graphics.popContext", Work::default());
            none
        }

        // ------------------------------------------------------------ drawing
        "graphics.fillRect" => {
            let c = color(uc, ru(a, 4));
            gfx(uc, "graphics.fillRect", |_, g| {
                g.fill_rect(Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)), c)
            });
            none
        }
        "graphics.drawRect" => {
            let c = color(uc, ru(a, 4));
            gfx(uc, "graphics.drawRect", |_, g| {
                let saved = (g.state.line_width, g.state.stroke);
                g.state.line_width = 1;
                g.state.stroke = StrokeLocation::Inside;
                let w = g.draw_rect(Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)), c);
                (g.state.line_width, g.state.stroke) = saved;
                w
            });
            none
        }
        "graphics.drawLine" => {
            let c = color(uc, ru(a, 5));
            gfx(uc, "graphics.drawLine", |_, g| {
                let saved = g.state.line_width;
                g.state.line_width = ri(a, 4).max(1);
                let w = g.draw_line(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3), c);
                g.state.line_width = saved;
                w
            });
            none
        }
        "graphics.setPixel" => {
            let c = color(uc, ru(a, 2));
            gfx(uc, "graphics.drawPixel", |_, g| {
                g.draw_pixel(ri(a, 0), ri(a, 1), c)
            });
            none
        }
        "graphics.fillTriangle" => {
            let c = color(uc, ru(a, 6));
            gfx(uc, "graphics.fillTriangle", |_, g| {
                g.fill_triangle(
                    [(ri(a, 0), ri(a, 1)), (ri(a, 2), ri(a, 3)), (ri(a, 4), ri(a, 5))],
                    c,
                )
            });
            none
        }
        "graphics.drawEllipse" => {
            let c = color(uc, ru(a, 7));
            gfx(uc, "graphics.drawEllipse", |_, g| {
                g.draw_ellipse(
                    Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)),
                    ri(a, 4),
                    rf(a, 5),
                    rf(a, 6),
                    c,
                )
            });
            none
        }
        "graphics.fillEllipse" => {
            let c = color(uc, ru(a, 6));
            gfx(uc, "graphics.fillEllipse", |_, g| {
                g.fill_ellipse(
                    Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)),
                    rf(a, 4),
                    rf(a, 5),
                    c,
                )
            });
            none
        }
        "graphics.drawRoundRect" => {
            let c = color(uc, ru(a, 6));
            gfx(uc, "graphics.drawRoundRect", |_, g| {
                g.draw_round_rect(
                    Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)),
                    ri(a, 4),
                    ri(a, 5),
                    c,
                )
            });
            none
        }
        "graphics.fillRoundRect" => {
            let c = color(uc, ru(a, 5));
            gfx(uc, "graphics.fillRoundRect", |_, g| {
                g.fill_round_rect(Rect::new(ri(a, 0), ri(a, 1), ri(a, 2), ri(a, 3)), ri(a, 4), c)
            });
            none
        }
        "graphics.fillPolygon" => {
            let n = ru(a, 0).min(4096);
            let b = uc.bytes(ru(a, 1), n as usize * 8);
            let pts: Vec<(i32, i32)> = b
                .chunks_exact(8)
                .map(|c| {
                    (
                        i32::from_le_bytes(c[0..4].try_into().unwrap()),
                        i32::from_le_bytes(c[4..8].try_into().unwrap()),
                    )
                })
                .collect();
            let c = color(uc, ru(a, 2));
            let rule = if ru(a, 3) == 0 {
                FillRule::NonZero
            } else {
                FillRule::EvenOdd
            };
            gfx(uc, "graphics.fillPolygon", |_, g| g.fill_polygon(&pts, rule, c));
            none
        }
        "graphics.drawBitmap" => {
            if let Some(id) = bid(ru(a, 0)) {
                gfx(uc, "graphics.drawBitmap", |_, g| {
                    g.draw_bitmap(id, ri(a, 1), ri(a, 2), Flip::from_index(ru(a, 3) as i64))
                });
            }
            none
        }
        "graphics.tileBitmap" => {
            if let Some(id) = bid(ru(a, 0)) {
                gfx(uc, "graphics.tileBitmap", |_, g| {
                    g.tile_bitmap(
                        id,
                        Rect::new(ri(a, 1), ri(a, 2), ri(a, 3), ri(a, 4)),
                        Flip::from_index(ru(a, 5) as i64),
                    )
                });
            }
            none
        }
        "graphics.drawScaledBitmap" => {
            if let Some(id) = bid(ru(a, 0)) {
                gfx(uc, "graphics.drawScaledBitmap", |_, g| {
                    g.draw_scaled_bitmap(id, ri(a, 1), ri(a, 2), rf(a, 3), rf(a, 4))
                });
            }
            none
        }
        "graphics.drawRotatedBitmap" => {
            if let Some(id) = bid(ru(a, 0)) {
                gfx(uc, "graphics.drawRotatedBitmap", |_, g| {
                    g.draw_rotated_bitmap(
                        id,
                        ri(a, 1),
                        ri(a, 2),
                        rf(a, 3),
                        rf(a, 4),
                        rf(a, 5),
                        rf(a, 6),
                        rf(a, 7),
                    )
                });
            }
            none
        }
        "graphics.drawText" => {
            let s = text(uc, ru(a, 0), ru(a, 1), ru(a, 2));
            let mut width = 0;
            gfx(uc, "graphics.drawText", |_, g| {
                let f = g.current_font();
                let (w, _, work) = g.draw_text(f, &s, ri(a, 3), ri(a, 4));
                width = w;
                work
            });
            Some(Ret::I(width as u32))
        }
        "graphics.drawTextInRect" => {
            let s = text(uc, ru(a, 0), ru(a, 1), ru(a, 2));
            let wrap = match ru(a, 7) {
                0 => WrapMode::Clip,
                1 => WrapMode::Character,
                _ => WrapMode::Word,
            };
            let align = match ru(a, 8) {
                1 => TextAlign::Right,
                2 => TextAlign::Center,
                _ => TextAlign::Left,
            };
            gfx(uc, "graphics.drawText", |_, g| {
                let f = g.current_font();
                g.draw_text_in_rect(
                    f,
                    &s,
                    Rect::new(ri(a, 3), ri(a, 4), ri(a, 5), ri(a, 6)),
                    wrap,
                    align,
                )
                .1
            });
            none
        }
        "graphics.getTextWidth" => {
            let s = text(uc, ru(a, 1), ru(a, 2), ru(a, 3));
            let d = dev.borrow();
            let f = fid(ru(a, 0)).unwrap_or_else(|| d.gfx.current_font());
            Some(Ret::I(
                d.gfx.font(f).map_or(0, |font| font.line_width(&s, ri(a, 4))) as u32,
            ))
        }
        "graphics.getTextHeightForMaxWidth" => {
            let s = text(uc, ru(a, 1), ru(a, 2), ru(a, 4));
            let d = dev.borrow();
            let f = fid(ru(a, 0)).unwrap_or_else(|| d.gfx.current_font());
            let lines = d.gfx.wrap_text(f, &s, ri(a, 3), WrapMode::Word);
            let h = d.gfx.font(f).map_or(0, |font| font.height() + ri(a, 7));
            Some(Ret::I((lines.len() as i32 * h).max(0) as u32))
        }
        "graphics.getFontHeight" => {
            let d = dev.borrow();
            let f = fid(ru(a, 0)).unwrap_or_else(|| d.gfx.current_font());
            Some(Ret::I(d.gfx.font(f).map_or(0, |f| f.height()) as u32))
        }
        "graphics.loadFont" => {
            let path = uc.cstr(ru(a, 0));
            let found = resolve_font(uc, &path);
            let r = found.map(|p| dev.borrow_mut().gfx.load_font(&p));
            match r {
                Some(Ok(id)) => {
                    charge(uc, Category::Io, "graphics.loadFont", Work::default());
                    Some(Ret::I(fh(id)))
                }
                _ => {
                    let e = put_str(uc, &format!("font not found: {path}"));
                    uc.w32(ru(a, 1), e);
                    Some(Ret::I(0))
                }
            }
        }

        // ------------------------------------------------------------ bitmaps
        "graphics.newBitmap" => {
            let c = color(uc, ru(a, 2));
            let id = dev.borrow_mut().gfx.new_bitmap(ri(a, 0), ri(a, 1), c);
            charge(uc, Category::Gfx, "graphics.newBitmap", Work::default());
            Some(Ret::I(bh(id)))
        }
        "graphics.freeBitmap" => {
            if let Some(id) = bid(ru(a, 0)) {
                dev.borrow_mut().gfx.free_bitmap(id);
            }
            none
        }
        "graphics.loadBitmap" | "graphics.loadIntoBitmap" => {
            let path = uc.cstr(ru(a, 0));
            let p = path.trim_end_matches(".pdi").to_string();
            let found = dev.borrow().fs.resolve_with_ext(&p, &["png"]);
            let loaded = found.as_ref().map(Bitmap::load_png);
            match loaded {
                Some(Ok(b)) => {
                    charge(
                        uc,
                        Category::Io,
                        "graphics.loadBitmap",
                        Work {
                            bytes: b.memory_bytes(),
                            ..Default::default()
                        },
                    );
                    if name == "graphics.loadIntoBitmap" {
                        if let Some(id) = bid(ru(a, 1)) {
                            if let Some(t) = dev.borrow_mut().gfx.bitmap_mut(id) {
                                *t = b;
                            }
                        }
                        none
                    } else {
                        Some(Ret::I(bh(dev.borrow_mut().gfx.add_bitmap(b))))
                    }
                }
                other => {
                    let msg = match other {
                        Some(Err(e)) => e.to_string(),
                        _ => format!("file not found: {path}"),
                    };
                    let e = put_str(uc, &msg);
                    let err_ptr = if name == "graphics.loadBitmap" {
                        ru(a, 1)
                    } else {
                        ru(a, 2)
                    };
                    uc.w32(err_ptr, e);
                    Some(Ret::I(0))
                }
            }
        }
        "graphics.copyBitmap" => {
            let id = bid(ru(a, 0)).and_then(|id| dev.borrow_mut().gfx.copy_bitmap(id));
            charge(uc, Category::Gfx, "graphics.copyBitmap", Work::default());
            Some(Ret::I(id.map_or(0, bh)))
        }
        "graphics.clearBitmap" => {
            let c = color(uc, ru(a, 1));
            if let Some(id) = bid(ru(a, 0)) {
                gfx(uc, "graphics.clear", |_, g| {
                    g.push_context(Some(id));
                    let w = g.clear(c);
                    g.pop_context();
                    w
                });
            }
            none
        }
        "graphics.getBitmapData" => {
            let Some(id) = bid(ru(a, 0)) else {
                return Some(Ret::None);
            };
            let b = dev.borrow().gfx.bitmap(id).cloned().unwrap_or_default();
            uc.w32(ru(a, 1), b.width as u32);
            uc.w32(ru(a, 2), b.height as u32);
            uc.w32(ru(a, 3), b.rowbytes as u32);
            // A read-only snapshot of the pixels.
            let data = heap_alloc(uc, b.data.len() as u32);
            uc.write(data, &b.data);
            uc.w32(ru(a, 5), data);
            let mask = match &b.mask {
                Some(m) => {
                    let p = heap_alloc(uc, m.len() as u32);
                    uc.write(p, m);
                    p
                }
                None => 0,
            };
            uc.w32(ru(a, 4), mask);
            none
        }
        "graphics.rotatedBitmap" => {
            let id = bid(ru(a, 0)).and_then(|id| {
                dev.borrow_mut()
                    .gfx
                    .rotated_bitmap(id, rf(a, 1), rf(a, 2), rf(a, 3))
            });
            let size = id
                .and_then(|i| dev.borrow().gfx.bitmap(i).map(|b| b.memory_bytes()))
                .unwrap_or(0);
            uc.w32(ru(a, 4), size as u32);
            charge(uc, Category::Gfx, "graphics.drawRotatedBitmap", Work::default());
            Some(Ret::I(id.map_or(0, bh)))
        }
        "graphics.setBitmapMask" => {
            let (Some(b), Some(m)) = (bid(ru(a, 0)), bid(ru(a, 1))) else {
                return Some(Ret::I(0));
            };
            let mut d = dev.borrow_mut();
            let Some(mask) = d.gfx.bitmap(m).cloned() else {
                return Some(Ret::I(0));
            };
            if let Some(t) = d.gfx.bitmap_mut(b) {
                let mut mv = vec![0u8; t.data.len()];
                for y in 0..t.height {
                    for x in 0..t.width {
                        if mask.get(x, y) {
                            mv[y as usize * t.rowbytes + (x as usize >> 3)] |= 0x80 >> (x & 7);
                        }
                    }
                }
                t.mask = Some(mv);
            }
            Some(Ret::I(1))
        }
        "graphics.getBitmapMask" => {
            let Some(b) = bid(ru(a, 0)) else {
                return Some(Ret::I(0));
            };
            let mut d = dev.borrow_mut();
            let Some(src) = d.gfx.bitmap(b).cloned() else {
                return Some(Ret::I(0));
            };
            if src.mask.is_none() {
                return Some(Ret::I(0));
            }
            let mut m = Bitmap::new(src.width, src.height, Color::Black);
            for y in 0..src.height {
                for x in 0..src.width {
                    m.set(x, y, src.opaque(x, y));
                }
            }
            Some(Ret::I(bh(d.gfx.add_bitmap(m))))
        }
        "graphics.getBitmapPixel" => {
            let d = dev.borrow();
            let v = bid(ru(a, 0)).and_then(|id| d.gfx.bitmap(id)).map_or(0, |b| {
                let (x, y) = (ri(a, 1), ri(a, 2));
                if !b.opaque(x, y) {
                    2
                } else {
                    b.get(x, y) as u32
                }
            });
            Some(Ret::I(v))
        }
        "graphics.checkMaskCollision" => {
            let d = dev.borrow();
            let (Some(b1), Some(b2)) = (
                bid(ru(a, 0)).and_then(|i| d.gfx.bitmap(i)),
                bid(ru(a, 4)).and_then(|i| d.gfx.bitmap(i)),
            ) else {
                return Some(Ret::I(0));
            };
            let r1 = Rect::new(ri(a, 1), ri(a, 2), b1.width, b1.height);
            let r2 = Rect::new(ri(a, 5), ri(a, 6), b2.width, b2.height);
            let mut area = r1.intersect(&r2);
            if let Some(Arg::LcdRect([l, r, t, b])) = a.get(8) {
                // LCDRect is left, right, top, bottom.
                area = area.intersect(&Rect::new(*l, *t, r - l, b - t));
            }
            let hit = (area.y..area.y + area.h).any(|y| {
                (area.x..area.x + area.w)
                    .any(|x| b1.opaque(x - r1.x, y - r1.y) && b2.opaque(x - r2.x, y - r2.y))
            });
            Some(Ret::I(hit as u32))
        }
        "graphics.setColorToPattern" => {
            let Some(id) = bid(ru(a, 1)) else {
                return Some(Ret::None);
            };
            let (ox, oy) = (ri(a, 2), ri(a, 3));
            let b = dev.borrow().gfx.bitmap(id).cloned().unwrap_or_default();
            let mut pat = [0u8; 16];
            for y in 0..8 {
                for x in 0..8 {
                    if b.get(ox + x, oy + y) {
                        pat[y as usize] |= 0x80 >> x;
                    }
                    if b.opaque(ox + x, oy + y) {
                        pat[8 + y as usize] |= 0x80 >> x;
                    }
                }
            }
            let p = heap_alloc(uc, 16);
            uc.write(p, &pat);
            uc.w32(ru(a, 0), p);
            none
        }
        "graphics.getFrame" => {
            uc.get_data_mut().frame_mirror = true;
            let data = dev.borrow().gfx.frame().data.clone();
            uc.write(FRAME_ADDR, &data);
            Some(Ret::I(FRAME_ADDR))
        }
        "graphics.getDisplayFrame" => {
            let data = dev.borrow().gfx.display().data.clone();
            uc.write(DISPLAY_ADDR, &data);
            Some(Ret::I(DISPLAY_ADDR))
        }
        "graphics.copyFrameBufferBitmap" | "graphics.getDisplayBufferBitmap" => {
            ArmGame::before_gfx(uc);
            let mut d = dev.borrow_mut();
            let b = if name == "graphics.copyFrameBufferBitmap" {
                d.gfx.frame().clone()
            } else {
                d.gfx.display().clone()
            };
            Some(Ret::I(bh(d.gfx.add_bitmap(b))))
        }
        "graphics.markUpdatedRows" => {
            ArmGame::before_gfx(uc);
            dev.borrow_mut().gfx.mark_updated_rows(ri(a, 0), ri(a, 1));
            charge(
                uc,
                Category::Gfx,
                "graphics.markUpdatedRows",
                Work {
                    rows: (ri(a, 1) - ri(a, 0) + 1).max(0) as u64,
                    ..Default::default()
                },
            );
            none
        }
        "graphics.display" => none,
        "graphics.getDebugBitmap"
        | "graphics.getFontPage"
        | "graphics.getPageGlyph"
        | "graphics.getGlyphKerning"
        | "graphics.getFontGlyph"
        | "graphics.makeFontFromData"
        | "graphics.makeFontFromData_deprecated" => Some(Ret::I(0)),

        // ------------------------------------------------------------ bitmap tables
        "graphics.newBitmapTable" => {
            let (n, w, h) = (ru(a, 0).min(4096), ri(a, 1), ri(a, 2));
            let ids: Vec<u32> = (0..n)
                .map(|_| dev.borrow_mut().gfx.new_bitmap(w, h, Color::Clear))
                .collect();
            let st = uc.get_data_mut();
            let t = st.next_table;
            st.next_table += 1;
            st.tables.insert(t, ids);
            Some(Ret::I(TABLE_BASE + t * 16))
        }
        "graphics.loadBitmapTable" | "graphics.loadIntoBitmapTable" => {
            let path = uc.cstr(ru(a, 0));
            let base = {
                let d = dev.borrow();
                let clean =
                    pdsim_device::FileSystem::clean(path.trim_end_matches(".pdt")).unwrap_or_default();
                d.fs.game_dir.join(clean)
            };
            match pdsim_gfx::table::load_table(&base) {
                Ok(v) => {
                    let ids: Vec<u32> = v
                        .into_iter()
                        .map(|b| dev.borrow_mut().gfx.add_bitmap(b))
                        .collect();
                    charge(uc, Category::Io, "graphics.loadBitmap", Work::default());
                    if name == "graphics.loadIntoBitmapTable" {
                        let t = (ru(a, 1).wrapping_sub(TABLE_BASE)) / 16;
                        uc.get_data_mut().tables.insert(t, ids);
                        none
                    } else {
                        let st = uc.get_data_mut();
                        let t = st.next_table;
                        st.next_table += 1;
                        st.tables.insert(t, ids);
                        Some(Ret::I(TABLE_BASE + t * 16))
                    }
                }
                Err(e) => {
                    let p = put_str(uc, &e.to_string());
                    let err_ptr = if name == "graphics.loadBitmapTable" {
                        ru(a, 1)
                    } else {
                        ru(a, 2)
                    };
                    uc.w32(err_ptr, p);
                    Some(Ret::I(0))
                }
            }
        }
        "graphics.getTableBitmap" => {
            let t = (ru(a, 0).wrapping_sub(TABLE_BASE)) / 16;
            let id = uc
                .get_data()
                .tables
                .get(&t)
                .and_then(|v| v.get(ru(a, 1) as usize).copied());
            Some(Ret::I(id.map_or(0, bh)))
        }
        "graphics.getBitmapTableInfo" => {
            let t = (ru(a, 0).wrapping_sub(TABLE_BASE)) / 16;
            let n = uc.get_data().tables.get(&t).map_or(0, |v| v.len());
            uc.w32(ru(a, 1), n as u32);
            uc.w32(ru(a, 2), n as u32);
            none
        }
        "graphics.freeBitmapTable" => {
            let t = (ru(a, 0).wrapping_sub(TABLE_BASE)) / 16;
            if let Some(ids) = uc.get_data_mut().tables.remove(&t) {
                for id in ids {
                    dev.borrow_mut().gfx.free_bitmap(id);
                }
            }
            none
        }

        // ------------------------------------------------------------ sprites
        n if n.starts_with("sprite.") => return sprite(uc, &n[7..], a),

        // ------------------------------------------------------------ files
        n if n.starts_with("file.") => return file(uc, &n[5..], a),

        _ => return None,
    };
    r
}

fn pdsim_device_epoch() -> f64 {
    // Same fixed "wall clock" start as the Lua backend: 2026-01-01T00:00:00Z.
    1_767_225_600.0
}

fn sprite_rect(uc: &Uc, id: u32) -> [f32; 4] {
    let d = uc.get_data().dev.borrow();
    d.sprites.get(id).map_or([0.0; 4], |s| {
        let b = s.bounds();
        [b.x, b.y, b.w, b.h]
    })
}

/// Writes `SpriteCollisionInfo` structs (76 bytes each) to the heap.
fn put_collisions(uc: &mut Uc, cols: &[pdsim_gfx::sprite::Collision], len_ptr: u32) -> u32 {
    uc.w32(len_ptr, cols.len() as u32);
    if cols.is_empty() {
        return 0;
    }
    let base = heap_alloc(uc, cols.len() as u32 * 76);
    for (i, c) in cols.iter().enumerate() {
        let p = base + i as u32 * 76;
        let mut b = Vec::with_capacity(76);
        b.extend(sh(c.sprite).to_le_bytes());
        b.extend(sh(c.other).to_le_bytes());
        b.extend((c.response.index() as u32).to_le_bytes());
        b.extend([c.overlaps as u8, 0, 0, 0]);
        b.extend(c.ti.to_le_bytes());
        b.extend(c.move_x.to_le_bytes());
        b.extend(c.move_y.to_le_bytes());
        b.extend(c.normal_x.to_le_bytes());
        b.extend(c.normal_y.to_le_bytes());
        b.extend(c.touch_x.to_le_bytes());
        b.extend(c.touch_y.to_le_bytes());
        for r in [c.sprite_rect, c.other_rect] {
            b.extend(r.x.to_le_bytes());
            b.extend(r.y.to_le_bytes());
            b.extend(r.w.to_le_bytes());
            b.extend(r.h.to_le_bytes());
        }
        uc.write(p, &b);
    }
    base
}

fn sprite(uc: &mut Uc, f: &str, a: &[Arg]) -> Option<Ret> {
    let dev = uc.get_data().dev.clone();
    let none = Some(Ret::None);
    let s = sid(ru(a, 0));
    let modify = |g: &dyn Fn(&mut pdsim_gfx::sprite::Sprite)| {
        if let Some(id) = s {
            dev.borrow_mut().sprites.modify(id, |sp| g(sp));
        }
    };
    let get = |g: &dyn Fn(&pdsim_gfx::sprite::Sprite) -> u32| -> u32 {
        s.and_then(|id| dev.borrow().sprites.get(id).map(g)).unwrap_or(0)
    };
    let r = match f {
        "newSprite" => Some(Ret::I(sh(dev.borrow_mut().sprites.create()))),
        "freeSprite" => {
            if let Some(id) = s {
                dev.borrow_mut().sprites.destroy(id);
                uc.get_data_mut().sprite_fns.remove(&id);
            }
            none
        }
        "copy" => {
            let Some(id) = s else { return Some(Ret::I(0)) };
            let mut d = dev.borrow_mut();
            let src = d.sprites.get(id).cloned();
            let n = d.sprites.create();
            if let Some(mut sp) = src {
                sp.added = false;
                d.sprites.modify(n, |t| *t = sp);
            }
            drop(d);
            let fns = uc.get_data().sprite_fns.get(&id).copied();
            if let Some(fns) = fns {
                uc.get_data_mut().sprite_fns.insert(n, fns);
            }
            Some(Ret::I(sh(n)))
        }
        "addSprite" => {
            if let Some(id) = s {
                dev.borrow_mut().sprites.add(id);
            }
            charge(uc, Category::Gfx, "sprite.add", Work::default());
            none
        }
        "removeSprite" => {
            if let Some(id) = s {
                dev.borrow_mut().sprites.remove(id);
            }
            charge(uc, Category::Gfx, "sprite.remove", Work::default());
            none
        }
        "removeSprites" => {
            let n = ru(a, 1).min(65536);
            for i in 0..n {
                let h = uc.r32(ru(a, 0) + i * 4);
                if let Some(id) = sid(h) {
                    dev.borrow_mut().sprites.remove(id);
                }
            }
            none
        }
        "removeAllSprites" => {
            dev.borrow_mut().sprites.remove_all();
            none
        }
        "getSpriteCount" => Some(Ret::I(dev.borrow().sprites.count() as u32)),
        "setAlwaysRedraw" => {
            dev.borrow_mut().sprites.always_redraw = ru(a, 0) != 0;
            none
        }
        "addDirtyRect" => {
            if let Some(Arg::LcdRect([l, r, t, b])) = a.first() {
                dev.borrow_mut()
                    .sprites
                    .add_dirty_rect(Rect::new(*l, *t, r - l, b - t));
            }
            charge(uc, Category::Gfx, "sprite.addDirtyRect", Work::default());
            none
        }
        "setBounds" => {
            let r = rect_arg(a.get(1).copied().unwrap_or(Arg::I(0)));
            modify(&|sp| {
                sp.width = r[2];
                sp.height = r[3];
                sp.x = r[0] + r[2] * sp.center.0;
                sp.y = r[1] + r[3] * sp.center.1;
            });
            none
        }
        "getBounds" => Some(Ret::Rect(s.map_or([0.0; 4], |id| sprite_rect(uc, id)))),
        "moveTo" => {
            let (x, y) = (rf(a, 1), rf(a, 2));
            modify(&|sp| {
                sp.x = x;
                sp.y = y;
            });
            charge(uc, Category::Gfx, "sprite.moveTo", Work::default());
            none
        }
        "moveBy" => {
            let (x, y) = (rf(a, 1), rf(a, 2));
            modify(&|sp| {
                sp.x += x;
                sp.y += y;
            });
            charge(uc, Category::Gfx, "sprite.moveTo", Work::default());
            none
        }
        "getPosition" => {
            let (x, y) = s
                .and_then(|id| dev.borrow().sprites.get(id).map(|sp| (sp.x, sp.y)))
                .unwrap_or((0.0, 0.0));
            uc.wf32(ru(a, 1), x);
            uc.wf32(ru(a, 2), y);
            none
        }
        "setImage" => {
            let img = bid(ru(a, 1));
            let size = img.and_then(|i| {
                dev.borrow()
                    .gfx
                    .bitmap(i)
                    .map(|b| (b.width as f32, b.height as f32))
            });
            let flip = Flip::from_index(ru(a, 2) as i64);
            modify(&|sp| {
                sp.image = img;
                sp.flip = flip;
                if let Some((w, h)) = size {
                    sp.width = w;
                    sp.height = h;
                }
            });
            charge(uc, Category::Gfx, "sprite.setImage", Work::default());
            none
        }
        "getImage" => Some(Ret::I(get(&|sp| sp.image.map_or(0, bh)))),
        "setSize" => {
            let (w, h) = (rf(a, 1), rf(a, 2));
            modify(&|sp| {
                sp.width = w;
                sp.height = h;
            });
            none
        }
        "setCenter" => {
            let (x, y) = (rf(a, 1), rf(a, 2));
            modify(&|sp| sp.center = (x, y));
            none
        }
        "getCenter" => {
            let c = s
                .and_then(|id| dev.borrow().sprites.get(id).map(|sp| sp.center))
                .unwrap_or((0.5, 0.5));
            uc.wf32(ru(a, 1), c.0);
            uc.wf32(ru(a, 2), c.1);
            none
        }
        "setZIndex" => {
            let z = ru(a, 1) as i16 as i32;
            modify(&|sp| sp.z = z);
            none
        }
        "getZIndex" => Some(Ret::I(get(&|sp| sp.z as i16 as u16 as u32))),
        "setDrawMode" => {
            let m = DrawMode::from_index(ru(a, 1) as i64);
            modify(&|sp| sp.draw_mode = m);
            none
        }
        "setImageFlip" => {
            let fl = Flip::from_index(ru(a, 1) as i64);
            modify(&|sp| sp.flip = fl);
            none
        }
        "getImageFlip" => Some(Ret::I(get(&|sp| sp.flip as u32))),
        "setUpdatesEnabled" => {
            let v = ru(a, 1) != 0;
            modify(&|sp| sp.updates_enabled = v);
            none
        }
        "updatesEnabled" => Some(Ret::I(get(&|sp| sp.updates_enabled as u32))),
        "setCollisionsEnabled" => {
            let v = ru(a, 1) != 0;
            modify(&|sp| sp.collisions_enabled = v);
            none
        }
        "collisionsEnabled" => Some(Ret::I(get(&|sp| sp.collisions_enabled as u32))),
        "setVisible" => {
            let v = ru(a, 1) != 0;
            modify(&|sp| sp.visible = v);
            none
        }
        "isVisible" => Some(Ret::I(get(&|sp| sp.visible as u32))),
        "setOpaque" => {
            let v = ru(a, 1) != 0;
            modify(&|sp| sp.opaque = v);
            none
        }
        "markDirty" => {
            if let Some(id) = s {
                dev.borrow_mut().sprites.mark_dirty(id);
            }
            none
        }
        "setTag" => {
            let t = ru(a, 1) as u8;
            modify(&|sp| sp.tag = t);
            none
        }
        "getTag" => Some(Ret::I(get(&|sp| sp.tag as u32))),
        "setIgnoresDrawOffset" => {
            let v = ru(a, 1) != 0;
            modify(&|sp| sp.ignores_draw_offset = v);
            none
        }
        "setClipRect" => {
            if let Some(Arg::LcdRect([l, r, t, b])) = a.get(1) {
                let rc = Rect::new(*l, *t, r - l, b - t);
                modify(&|sp| sp.clip_rect = Some(rc));
            }
            none
        }
        "clearClipRect" => {
            modify(&|sp| sp.clip_rect = None);
            none
        }
        "setClipRectsInRange"
        | "clearClipRectsInRange"
        | "setStencil"
        | "setStencilPattern"
        | "clearStencil"
        | "setStencilImage"
        | "resetCollisionWorld" => none,
        "setUpdateFunction" | "setDrawFunction" | "setCollisionResponseFunction" | "setUserdata" => {
            if let Some(id) = s {
                let v = ru(a, 1);
                let e = uc.get_data_mut().sprite_fns.entry(id).or_default();
                match f {
                    "setUpdateFunction" => e.update = v,
                    "setDrawFunction" => e.draw = v,
                    "setCollisionResponseFunction" => e.collision = v,
                    _ => e.userdata = v,
                }
            }
            none
        }
        "getUserdata" => Some(Ret::I(
            s.and_then(|id| uc.get_data().sprite_fns.get(&id).map(|f| f.userdata))
                .unwrap_or(0),
        )),
        "setCollideRect" => {
            let r = rect_arg(a.get(1).copied().unwrap_or(Arg::I(0)));
            modify(&|sp| sp.collide_rect = Some(RectF::new(r[0], r[1], r[2], r[3])));
            none
        }
        "getCollideRect" => Some(Ret::Rect(
            s.and_then(|id| dev.borrow().sprites.get(id).and_then(|sp| sp.collide_rect))
                .map_or([0.0; 4], |r| [r.x, r.y, r.w, r.h]),
        )),
        "clearCollideRect" => {
            modify(&|sp| sp.collide_rect = None);
            none
        }
        "moveWithCollisions" | "checkCollisions" => {
            let Some(id) = s else { return Some(Ret::I(0)) };
            let (gx, gy) = (rf(a, 1), rf(a, 2));
            // Ask the game for each candidate's response first.
            let candidates = dev.borrow().sprites.move_candidates(id, gx, gy);
            let resp_fn = uc.get_data().sprite_fns.get(&id).map_or(0, |f| f.collision);
            let mut responses = std::collections::HashMap::new();
            for o in candidates {
                let r = if resp_fn != 0 {
                    match call(uc, resp_fn, &[sh(id), sh(o)]) {
                        Ok(v) => v,
                        Err(e) => {
                            uc.get_data_mut().error = Some(e);
                            0
                        }
                    }
                } else {
                    0
                };
                responses.insert(o, CollisionResponse::from_index(r as i64));
            }
            let lookup = |o: SpriteId| responses.get(&o).copied().unwrap_or_default();
            let (ax, ay, cols) = if f == "moveWithCollisions" {
                dev.borrow_mut().sprites.move_with_collisions(id, gx, gy, &lookup)
            } else {
                dev.borrow().sprites.check_collisions(id, gx, gy, &lookup)
            };
            let n = dev.borrow().sprites.count() as u64;
            charge(
                uc,
                Category::Gfx,
                "sprite.moveWithCollisions",
                Work {
                    items: n,
                    ..Default::default()
                },
            );
            uc.wf32(ru(a, 3), ax);
            uc.wf32(ru(a, 4), ay);
            Some(Ret::I(put_collisions(uc, &cols, ru(a, 5))))
        }
        "querySpritesAtPoint" => {
            let v: Vec<u32> = dev
                .borrow()
                .sprites
                .query_point(rf(a, 0), rf(a, 1))
                .into_iter()
                .map(sh)
                .collect();
            Some(Ret::I(put_array(uc, &v, ru(a, 2))))
        }
        "querySpritesInRect" => {
            let r = RectF::new(rf(a, 0), rf(a, 1), rf(a, 2), rf(a, 3));
            let v: Vec<u32> = dev.borrow().sprites.query_rect(r).into_iter().map(sh).collect();
            Some(Ret::I(put_array(uc, &v, ru(a, 4))))
        }
        "querySpritesAlongLine" => {
            let v: Vec<u32> = dev
                .borrow()
                .sprites
                .query_line(rf(a, 0), rf(a, 1), rf(a, 2), rf(a, 3))
                .into_iter()
                .map(sh)
                .collect();
            Some(Ret::I(put_array(uc, &v, ru(a, 4))))
        }
        "querySpriteInfoAlongLine" => {
            uc.w32(ru(a, 4), 0);
            Some(Ret::I(0))
        }
        "overlappingSprites" => {
            let v: Vec<u32> = s
                .map(|id| dev.borrow().sprites.overlapping(id))
                .unwrap_or_default()
                .into_iter()
                .map(sh)
                .collect();
            Some(Ret::I(put_array(uc, &v, ru(a, 1))))
        }
        "allOverlappingSprites" => {
            let v: Vec<u32> = dev
                .borrow()
                .sprites
                .all_overlapping()
                .into_iter()
                .flat_map(|(x, y)| [sh(x), sh(y)])
                .collect();
            uc.w32(ru(a, 0), (v.len() / 2) as u32);
            if v.is_empty() {
                return Some(Ret::I(0));
            }
            let p = heap_alloc(uc, v.len() as u32 * 4);
            let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
            uc.write(p, &b);
            Some(Ret::I(p))
        }
        "updateAndDrawSprites" | "drawSprites" => {
            if f == "updateAndDrawSprites" {
                let ids = dev.borrow().sprites.update_list();
                for id in ids {
                    let upd = uc.get_data().sprite_fns.get(&id).map_or(0, |f| f.update);
                    if upd != 0 {
                        if let Err(e) = call(uc, upd, &[sh(id)]) {
                            uc.get_data_mut().error = Some(e);
                            return Some(Ret::None);
                        }
                    }
                }
            }
            draw_sprites(uc);
            none
        }
        _ => return None,
    };
    r
}

fn draw_sprites(uc: &mut Uc) {
    ArmGame::before_gfx(uc);
    let dev = uc.get_data().dev.clone();
    // Which sprites draw themselves.
    let list = dev.borrow().sprites.display_list();
    for id in &list {
        let has = uc.get_data().sprite_fns.get(id).is_some_and(|f| f.draw != 0);
        dev.borrow_mut()
            .sprites
            .modify(*id, |s| s.has_draw_callback = has);
    }
    let (passes, saved, count) = {
        let mut d = dev.borrow_mut();
        let off = d.gfx.state.draw_offset;
        let n = d.sprites.count() as u64;
        (d.sprites.plan_draw(off), d.gfx.state.clone(), n)
    };
    let mut work = Work::default();
    for pass in &passes {
        {
            let mut d = dev.borrow_mut();
            let font = d.gfx.state.font;
            d.gfx.state = State {
                font,
                clip: Some(pass.rect),
                ..State::default()
            };
            let bg = d.gfx.background_color;
            if pass.fill_background && bg != Color::Clear {
                work.add(d.gfx.fill_rect(pass.rect, bg));
            }
        }
        for item in &pass.items {
            match item {
                DrawItem::Image {
                    image,
                    x,
                    y,
                    flip,
                    mode,
                    clip,
                    ..
                } => {
                    let mut d = dev.borrow_mut();
                    d.gfx.state.draw_offset = (0, 0);
                    d.gfx.state.clip = Some(*clip);
                    d.gfx.state.draw_mode = *mode;
                    work.add(d.gfx.draw_bitmap(*image, *x, *y, *flip));
                }
                DrawItem::Callback {
                    sprite,
                    x,
                    y,
                    local,
                    clip,
                } => {
                    let draw = uc.get_data().sprite_fns.get(sprite).map_or(0, |f| f.draw);
                    let b = sprite_rect(uc, *sprite);
                    {
                        let mut d = dev.borrow_mut();
                        let font = d.gfx.state.font;
                        d.gfx.state = State {
                            font,
                            clip: Some(*clip),
                            ..State::default()
                        };
                    }
                    let _ = (x, y);
                    // void draw(LCDSprite*, PDRect bounds, PDRect drawrect)
                    let dr = [
                        local.x as f32 + b[0],
                        local.y as f32 + b[1],
                        local.w as f32,
                        local.h as f32,
                    ];
                    ArmGame::after_gfx(uc);
                    if let Err(e) = call_full(
                        uc,
                        draw,
                        &[sh(*sprite)],
                        &[b[0], b[1], b[2], b[3], dr[0], dr[1], dr[2], dr[3]],
                    ) {
                        uc.get_data_mut().error = Some(e);
                    }
                    ArmGame::before_gfx(uc);
                }
            }
        }
    }
    dev.borrow_mut().gfx.state = saved;
    ArmGame::after_gfx(uc);
    work.items = count;
    charge(uc, Category::Gfx, "sprite.update", work);
}

fn file(uc: &mut Uc, f: &str, a: &[Arg]) -> Option<Ret> {
    let dev = uc.get_data().dev.clone();
    let handle = |h: u32| (h.wrapping_sub(FILE_BASE)) / 16;
    let fail = |uc: &mut Uc, msg: String| {
        uc.get_data_mut().last_file_error = Some(msg);
    };
    let r = match f {
        "geterr" => {
            let e = uc.get_data_mut().last_file_error.take();
            Some(Ret::I(e.map_or(0, |e| put_str(uc, &e))))
        }
        "open" => {
            let path = uc.cstr(ru(a, 0));
            let mode = ru(a, 1);
            let write = mode & (4 | 8) != 0;
            let data = if write {
                if mode & 8 != 0 {
                    dev.borrow().fs.read(&path).unwrap_or_default()
                } else {
                    vec![]
                }
            } else {
                match dev.borrow().fs.read(&path) {
                    Some(d) => d,
                    None => {
                        fail(uc, format!("file not found: {path}"));
                        return Some(Ret::I(0));
                    }
                }
            };
            charge(uc, Category::Io, "file.open", Work::default());
            let pos = if mode & 8 != 0 { data.len() } else { 0 };
            let st = uc.get_data_mut();
            let id = st.next_file;
            st.next_file += 1;
            st.files.insert(
                id,
                OpenFile {
                    path,
                    data,
                    pos,
                    write,
                    dirty: write,
                },
            );
            Some(Ret::I(FILE_BASE + id * 16))
        }
        "close" | "flush" => {
            let id = handle(ru(a, 0));
            let fo = if f == "close" {
                uc.get_data_mut().files.remove(&id)
            } else {
                uc.get_data().files.get(&id).cloned()
            };
            if let Some(fo) = fo {
                if fo.dirty {
                    dev.borrow().fs.write(&fo.path, &fo.data);
                    charge(
                        uc,
                        Category::Io,
                        "file.write",
                        Work {
                            bytes: fo.data.len() as u64,
                            ..Default::default()
                        },
                    );
                }
                if f == "flush" {
                    if let Some(x) = uc.get_data_mut().files.get_mut(&id) {
                        x.dirty = false;
                    }
                }
            }
            charge(uc, Category::Io, "file.close", Work::default());
            Some(Ret::I(0))
        }
        "read" => {
            let id = handle(ru(a, 0));
            let (buf, len) = (ru(a, 1), ru(a, 2) as usize);
            let chunk = uc.get_data_mut().files.get_mut(&id).map(|fo| {
                let end = (fo.pos + len).min(fo.data.len());
                let c = fo.data[fo.pos.min(end)..end].to_vec();
                fo.pos = end;
                c
            });
            match chunk {
                Some(c) => {
                    uc.write(buf, &c);
                    charge(
                        uc,
                        Category::Io,
                        "file.read",
                        Work {
                            bytes: c.len() as u64,
                            ..Default::default()
                        },
                    );
                    Some(Ret::I(c.len() as u32))
                }
                None => Some(Ret::I(u32::MAX)),
            }
        }
        "write" => {
            let id = handle(ru(a, 0));
            let bytes = uc.bytes(ru(a, 1), ru(a, 2) as usize);
            let n = bytes.len();
            let ok = uc.get_data_mut().files.get_mut(&id).map(|fo| {
                let end = fo.pos + n;
                if fo.data.len() < end {
                    fo.data.resize(end, 0);
                }
                fo.data[fo.pos..end].copy_from_slice(&bytes);
                fo.pos = end;
                fo.dirty = true;
            });
            Some(Ret::I(if ok.is_some() { n as u32 } else { u32::MAX }))
        }
        "tell" => Some(Ret::I(
            uc.get_data()
                .files
                .get(&handle(ru(a, 0)))
                .map_or(u32::MAX, |fo| fo.pos as u32),
        )),
        "seek" => {
            let id = handle(ru(a, 0));
            let (pos, whence) = (ri(a, 1), ru(a, 2));
            let ok = uc.get_data_mut().files.get_mut(&id).map(|fo| {
                let base = match whence {
                    1 => fo.pos as i64,
                    2 => fo.data.len() as i64,
                    _ => 0,
                };
                fo.pos = (base + pos as i64).max(0) as usize;
            });
            Some(Ret::I(if ok.is_some() { 0 } else { u32::MAX }))
        }
        "stat" => {
            let path = uc.cstr(ru(a, 0));
            let found = dev.borrow().fs.resolve_read(&path);
            let Some(p) = found else {
                fail(uc, format!("file not found: {path}"));
                return Some(Ret::I(u32::MAX));
            };
            let md = std::fs::metadata(&p).ok();
            let size = md.as_ref().map_or(0, |m| m.len()) as u32;
            let out = ru(a, 1);
            uc.w32(out, p.is_dir() as u32);
            uc.w32(out + 4, size);
            for i in 0..6 {
                uc.w32(out + 8 + i * 4, 0);
            }
            Some(Ret::I(0))
        }
        "mkdir" => {
            let path = uc.cstr(ru(a, 0));
            let d = dev.borrow();
            let ok = pdsim_device::FileSystem::clean(&path)
                .is_some_and(|p| std::fs::create_dir_all(d.fs.data_dir.join(p)).is_ok());
            Some(Ret::I(if ok { 0 } else { u32::MAX }))
        }
        "unlink" => {
            let path = uc.cstr(ru(a, 0));
            Some(Ret::I(if dev.borrow().fs.delete(&path) {
                0
            } else {
                u32::MAX
            }))
        }
        "rename" => {
            let (from, to) = (uc.cstr(ru(a, 0)), uc.cstr(ru(a, 1)));
            let d = dev.borrow();
            let ok = match (
                pdsim_device::FileSystem::clean(&from),
                pdsim_device::FileSystem::clean(&to),
            ) {
                (Some(x), Some(y)) => std::fs::rename(d.fs.data_dir.join(x), d.fs.data_dir.join(y)).is_ok(),
                _ => false,
            };
            Some(Ret::I(if ok { 0 } else { u32::MAX }))
        }
        "listfiles" => {
            let path = uc.cstr(ru(a, 0));
            let (cb, ud) = (ru(a, 1), ru(a, 2));
            let names = dev.borrow().fs.list(&path);
            for n in names {
                let p = put_str(uc, &n);
                if let Err(e) = call(uc, cb, &[p, ud]) {
                    uc.get_data_mut().error = Some(e);
                    break;
                }
            }
            Some(Ret::I(0))
        }
        _ => return None,
    };
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        for days in [-1000i64, 0, 10957, 20000, 30000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(10957), (2000, 1, 1));
    }
}
