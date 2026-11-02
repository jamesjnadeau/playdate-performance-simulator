//! Golden-image tests for the graphics primitives.
//!
//! Each test renders a scene and compares the framebuffer with
//! `tests/golden/<name>.png`. Run with `UPDATE_GOLDEN=1` to (re)create the
//! reference images after checking the output by eye.

use pdsim_gfx::*;
use std::path::PathBuf;

fn check(name: &str, g: &Graphics) {
    let png = g.frame().encode_png().unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() || !path.exists() {
        std::fs::write(&path, &png).unwrap();
        if std::env::var_os("UPDATE_GOLDEN").is_none() {
            panic!("golden image {name} was missing and has been created; check it and rerun");
        }
        return;
    }
    let want = Bitmap::load_png(&path).unwrap();
    let got = g.frame();
    let mut diff = 0;
    for y in 0..got.height {
        for x in 0..got.width {
            if got.get(x, y) != want.get(x, y) {
                diff += 1;
            }
        }
    }
    if diff > 0 {
        let out = path.with_extension("actual.png");
        std::fs::write(&out, &png).unwrap();
        panic!(
            "{name}: {diff} pixels differ; actual written to {}",
            out.display()
        );
    }
}

#[test]
fn rects_and_lines() {
    let mut g = Graphics::new(None);
    g.fill_rect(Rect::new(10, 10, 60, 40), Color::Black);
    g.draw_rect(Rect::new(80, 10, 60, 40), Color::Black);
    g.state.line_width = 4;
    g.draw_rect(Rect::new(150, 10, 60, 40), Color::Black);
    g.state.line_width = 1;
    for i in 0..8 {
        g.draw_line(10, 70, 10 + i * 25, 130, Color::Black);
    }
    g.state.line_width = 6;
    g.draw_line(230, 20, 380, 120, Color::Black);
    g.state.line_cap = LineCap::Round;
    g.draw_line(230, 140, 380, 140, Color::Black);
    g.state.line_cap = LineCap::Square;
    g.draw_line(230, 170, 380, 220, Color::Black);
    g.fill_rect(Rect::new(10, 150, 100, 60), Color::Black);
    g.fill_rect(Rect::new(30, 160, 60, 40), Color::Xor);
    check("rects_and_lines", &g);
}

#[test]
fn ellipses_polygons_round_rects() {
    let mut g = Graphics::new(None);
    g.fill_ellipse(Rect::new(10, 10, 80, 50), 0.0, 0.0, Color::Black);
    g.draw_ellipse(Rect::new(100, 10, 80, 50), 1, 0.0, 0.0, Color::Black);
    g.draw_ellipse(Rect::new(190, 10, 50, 50), 5, 0.0, 0.0, Color::Black);
    g.fill_ellipse(Rect::new(250, 10, 50, 50), 45.0, 270.0, Color::Black);
    g.fill_triangle([(10, 200), (60, 80), (110, 200)], Color::Black);
    g.fill_polygon(
        &[(130, 80), (230, 120), (130, 200), (180, 120)],
        FillRule::EvenOdd,
        Color::Black,
    );
    g.draw_polygon(
        &[(250, 80), (300, 130), (250, 180), (240, 130)],
        true,
        Color::Black,
    );
    g.fill_round_rect(Rect::new(310, 80, 80, 50), 12, Color::Black);
    g.draw_round_rect(Rect::new(310, 150, 80, 50), 12, 3, Color::Black);
    check("ellipses_polygons_round_rects", &g);
}

#[test]
fn patterns_and_dither() {
    let mut g = Graphics::new(None);
    for i in 0..9 {
        g.fill_rect(
            Rect::new(i * 44, 0, 44, 120),
            Color::Pattern(Pattern::dither(i as f32 / 8.0)),
        );
    }
    let checker = Pattern::opaque([0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55, 0xaa, 0x55]);
    g.fill_ellipse(Rect::new(20, 130, 150, 100), 0.0, 0.0, Color::Pattern(checker));
    let half = Pattern {
        bits: [0; 8],
        mask: [0xf0; 8],
    };
    g.fill_rect(Rect::new(200, 130, 180, 100), Color::Pattern(half));
    check("patterns_and_dither", &g);
}

fn arrow(g: &mut Graphics) -> BitmapId {
    let id = g.new_bitmap(24, 16, Color::Clear);
    g.push_context(Some(id));
    g.fill_rect(Rect::new(0, 5, 14, 6), Color::Black);
    g.fill_triangle([(14, 0), (23, 8), (14, 15)], Color::Black);
    g.fill_rect(Rect::new(2, 7, 8, 2), Color::White);
    g.pop_context();
    id
}

#[test]
fn bitmaps_and_draw_modes() {
    let mut g = Graphics::new(None);
    g.fill_rect(Rect::new(0, 120, 400, 120), Color::Black);
    let a = arrow(&mut g);
    let modes = [
        DrawMode::Copy,
        DrawMode::WhiteTransparent,
        DrawMode::BlackTransparent,
        DrawMode::FillWhite,
        DrawMode::FillBlack,
        DrawMode::Xor,
        DrawMode::Nxor,
        DrawMode::Inverted,
    ];
    for (i, m) in modes.iter().enumerate() {
        g.state.draw_mode = *m;
        g.draw_bitmap(a, 10 + i as i32 * 48, 20, Flip::Unflipped);
        g.draw_bitmap(a, 10 + i as i32 * 48, 140, Flip::FlippedX);
    }
    g.state.draw_mode = DrawMode::Copy;
    g.draw_scaled_bitmap(a, 10, 50, 3.0, 3.0);
    g.draw_rotated_bitmap(a, 200, 80, 45.0, 0.5, 0.5, 2.0, 2.0);
    g.tile_bitmap(a, Rect::new(260, 40, 130, 70), Flip::FlippedY);
    g.draw_scaled_bitmap(a, 10, 170, -2.0, 2.0);
    check("bitmaps_and_draw_modes", &g);
}

#[test]
fn text_clip_offset_stencil() {
    let mut g = Graphics::new(None);
    let f = g.system_font();
    g.draw_text(f, "Hello, Playdate!\nSecond line 0123456789", 10, 10);
    g.draw_text_in_rect(
        f,
        "The quick brown fox jumps over the lazy dog again and again.",
        Rect::new(10, 50, 150, 60),
        WrapMode::Word,
        TextAlign::Center,
    );
    g.draw_rect(Rect::new(10, 50, 150, 60), Color::Black);
    g.state.draw_offset = (200, 0);
    g.set_clip_rect(Rect::new(0, 10, 100, 50));
    g.fill_ellipse(Rect::new(-20, 0, 140, 100), 0.0, 0.0, Color::Black);
    g.clear_clip_rect();
    g.state.draw_offset = (0, 0);
    let st = g.new_bitmap(16, 16, Color::Black);
    g.push_context(Some(st));
    g.fill_ellipse(Rect::new(0, 0, 16, 16), 0.0, 0.0, Color::White);
    g.pop_context();
    g.state.stencil = Some((st, true));
    g.fill_rect(Rect::new(10, 130, 380, 100), Color::Black);
    g.state.stencil = None;
    g.fill_rect(Rect::new(250, 70, 140, 40), Color::Black);
    g.state.draw_mode = DrawMode::FillWhite;
    g.draw_text(f, "inverted", 260, 82);
    check("text_clip_offset_stencil", &g);
}

#[test]
fn work_counts() {
    let mut g = Graphics::new(None);
    let w = g.fill_rect(Rect::new(10, 10, 10, 10), Color::Black);
    assert_eq!((w.pixels, w.rows), (100, 10));
    let w = g.fill_rect(Rect::new(-5, -5, 10, 10), Color::Black);
    assert_eq!((w.pixels, w.rows), (25, 5), "clipped to the screen");
    let w = g.clear(Color::White);
    assert_eq!((w.pixels, w.rows), (96_000, 240));
    g.flush_display();
    assert_eq!(g.dirty_row_count(), 0);
    g.draw_line(0, 5, 399, 5, Color::Black);
    assert_eq!(g.dirty_row_count(), 1);
    let (_, _, w) = g.draw_text(g.system_font(), "abc", 0, 50);
    assert_eq!(w.items, 3);
    let a = g.new_bitmap(8, 8, Color::Black);
    let w = g.draw_bitmap(a, 0, 100, Flip::Unflipped);
    assert_eq!(w.pixels, 64);
}
