use pdsim_gfx::Graphics;

#[test]
fn mark_updated_rows_clamps_and_ignores_reversed_ranges() {
    let mut g = Graphics::new(None);
    let before = *g.dirty_rows();

    // start > end marks nothing (and must not panic).
    g.mark_updated_rows(50, 10);
    assert_eq!(*g.dirty_rows(), before);

    // Out-of-range bounds are clamped to the screen.
    g.mark_updated_rows(-5, 1000);
    assert!(g.dirty_rows().iter().all(|r| *r));
}
