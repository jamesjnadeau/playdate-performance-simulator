//! The sprite system: display list, dirty rects and collisions.
//!
//! This module holds only sprite state and the algorithms. The host (Lua or
//! C bindings) calls game callbacks (update, draw, collision response)
//! itself, so no callback runs while sprite state is borrowed.

use crate::graphics::BitmapId;
use crate::types::{DrawMode, Flip, Rect};

/// Handle to a sprite.
pub type SpriteId = u32;

/// A float rectangle (collide rects and bounds use floats).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        RectF { x, y, w, h }
    }

    /// Strictly overlapping (touching edges do not count).
    pub fn overlaps(&self, o: &RectF) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// Integer pixel bounds covering this rect.
    pub fn to_rect(&self) -> Rect {
        let x0 = self.x.floor() as i32;
        let y0 = self.y.floor() as i32;
        let x1 = (self.x + self.w).ceil() as i32;
        let y1 = (self.y + self.h).ceil() as i32;
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// How a moving sprite reacts to hitting another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollisionResponse {
    #[default]
    Slide,
    Freeze,
    Overlap,
    Bounce,
}

impl CollisionResponse {
    pub fn from_index(i: i64) -> Self {
        match i {
            1 => CollisionResponse::Freeze,
            2 => CollisionResponse::Overlap,
            3 => CollisionResponse::Bounce,
            _ => CollisionResponse::Slide,
        }
    }

    pub fn from_name(s: &str) -> Self {
        match s {
            "freeze" => CollisionResponse::Freeze,
            "overlap" => CollisionResponse::Overlap,
            "bounce" => CollisionResponse::Bounce,
            _ => CollisionResponse::Slide,
        }
    }

    pub fn index(self) -> i64 {
        self as i64
    }
}

/// One collision found by a move.
#[derive(Debug, Clone, PartialEq)]
pub struct Collision {
    pub sprite: SpriteId,
    pub other: SpriteId,
    pub response: CollisionResponse,
    /// Whether the sprites overlapped before the move.
    pub overlaps: bool,
    /// Fraction of the move at which the contact happened.
    pub ti: f32,
    pub move_x: f32,
    pub move_y: f32,
    pub normal_x: i32,
    pub normal_y: i32,
    pub touch_x: f32,
    pub touch_y: f32,
    pub sprite_rect: RectF,
    pub other_rect: RectF,
    pub slide: Option<(f32, f32)>,
    pub bounce: Option<(f32, f32)>,
}

/// A sprite.
#[derive(Debug, Clone)]
pub struct Sprite {
    pub image: Option<BitmapId>,
    pub flip: Flip,
    pub x: f32,
    pub y: f32,
    pub center: (f32, f32),
    pub width: f32,
    pub height: f32,
    pub z: i32,
    pub visible: bool,
    pub updates_enabled: bool,
    pub opaque: bool,
    pub ignores_draw_offset: bool,
    pub collide_rect: Option<RectF>,
    pub collisions_enabled: bool,
    pub groups: u32,
    pub collides_with: u32,
    pub tag: u8,
    pub draw_mode: DrawMode,
    pub clip_rect: Option<Rect>,
    pub has_draw_callback: bool,
    pub added: bool,
    pub order: u64,
    pub redraws_on_image_change: bool,
    pub always_redraw: bool,
    pub stencil: Option<(BitmapId, bool)>,
    dirty: bool,
    last_bounds: Option<Rect>,
}

impl Default for Sprite {
    fn default() -> Self {
        Sprite {
            image: None,
            flip: Flip::Unflipped,
            x: 0.0,
            y: 0.0,
            center: (0.5, 0.5),
            width: 0.0,
            height: 0.0,
            z: 0,
            visible: true,
            updates_enabled: true,
            opaque: false,
            ignores_draw_offset: false,
            collide_rect: None,
            collisions_enabled: true,
            groups: 0,
            collides_with: 0,
            tag: 0,
            draw_mode: DrawMode::Copy,
            clip_rect: None,
            has_draw_callback: false,
            added: false,
            order: 0,
            redraws_on_image_change: true,
            always_redraw: false,
            stencil: None,
            dirty: true,
            last_bounds: None,
        }
    }
}

impl Sprite {
    /// Bounds in world coordinates.
    pub fn bounds(&self) -> RectF {
        RectF::new(
            self.x - self.width * self.center.0,
            self.y - self.height * self.center.1,
            self.width,
            self.height,
        )
    }

    /// Collide rect in world coordinates.
    pub fn world_collide_rect(&self) -> Option<RectF> {
        let c = self.collide_rect?;
        let b = self.bounds();
        Some(RectF::new(b.x + c.x, b.y + c.y, c.w, c.h))
    }
}

/// What to draw for one dirty rect.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawItem {
    /// Draw the sprite's image at `(x, y)` (screen coordinates).
    Image {
        sprite: SpriteId,
        image: BitmapId,
        x: i32,
        y: i32,
        flip: Flip,
        mode: DrawMode,
    },
    /// Call the sprite's draw callback with the sprite at `(x, y)` and the
    /// dirty area in sprite-local coordinates.
    Callback {
        sprite: SpriteId,
        x: i32,
        y: i32,
        local: Rect,
    },
}

/// One dirty rect: fill the background (unless a sprite covers it), then
/// draw the items, clipped to `rect`.
#[derive(Debug, Clone, PartialEq)]
pub struct DirtyPass {
    pub rect: Rect,
    pub fill_background: bool,
    pub items: Vec<DrawItem>,
}

/// All sprites.
#[derive(Debug, Default)]
pub struct SpriteSystem {
    sprites: Vec<Option<Sprite>>,
    free: Vec<SpriteId>,
    next_order: u64,
    dirty_rects: Vec<Rect>,
    pub always_redraw: bool,
    screen: Rect,
}

impl SpriteSystem {
    pub fn new() -> Self {
        SpriteSystem {
            screen: Rect::new(0, 0, 400, 240),
            ..Default::default()
        }
    }

    /// Creates a sprite (not yet in the display list).
    pub fn create(&mut self) -> SpriteId {
        let s = Sprite::default();
        if let Some(id) = self.free.pop() {
            self.sprites[id as usize] = Some(s);
            id
        } else {
            self.sprites.push(Some(s));
            (self.sprites.len() - 1) as SpriteId
        }
    }

    /// Destroys a sprite.
    pub fn destroy(&mut self, id: SpriteId) {
        self.remove(id);
        if let Some(slot) = self.sprites.get_mut(id as usize) {
            if slot.take().is_some() {
                self.free.push(id);
            }
        }
    }

    pub fn get(&self, id: SpriteId) -> Option<&Sprite> {
        self.sprites.get(id as usize).and_then(|s| s.as_ref())
    }

    pub fn get_mut(&mut self, id: SpriteId) -> Option<&mut Sprite> {
        self.sprites.get_mut(id as usize).and_then(|s| s.as_mut())
    }

    /// Changes a sprite and marks it for redraw.
    pub fn modify<F: FnOnce(&mut Sprite)>(&mut self, id: SpriteId, f: F) {
        if let Some(s) = self.get_mut(id) {
            f(s);
            s.dirty = true;
        }
    }

    /// Adds a sprite to the display list.
    pub fn add(&mut self, id: SpriteId) {
        let order = self.next_order;
        if let Some(s) = self.get_mut(id) {
            if !s.added {
                s.added = true;
                s.order = order;
                s.dirty = true;
                s.last_bounds = None;
            }
        }
        self.next_order += 1;
    }

    /// Removes a sprite from the display list.
    pub fn remove(&mut self, id: SpriteId) {
        let mut old = None;
        if let Some(s) = self.get_mut(id) {
            if s.added {
                s.added = false;
                old = s.last_bounds.take();
            }
        }
        if let Some(r) = old {
            self.dirty_rects.push(r);
        }
    }

    /// Removes every sprite from the display list.
    pub fn remove_all(&mut self) {
        for id in self.display_list() {
            self.remove(id);
        }
    }

    /// Sprites in the display list, in draw order (z, then insertion).
    pub fn display_list(&self) -> Vec<SpriteId> {
        let mut v: Vec<(i32, u64, SpriteId)> = self
            .sprites
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.as_ref()
                    .filter(|s| s.added)
                    .map(|s| (s.z, s.order, i as SpriteId))
            })
            .collect();
        v.sort();
        v.into_iter().map(|t| t.2).collect()
    }

    /// Number of sprites in the display list.
    pub fn count(&self) -> usize {
        self.sprites.iter().flatten().filter(|s| s.added).count()
    }

    /// Sprites whose update function should run this frame, in order.
    pub fn update_list(&self) -> Vec<SpriteId> {
        self.display_list()
            .into_iter()
            .filter(|id| self.get(*id).is_some_and(|s| s.updates_enabled))
            .collect()
    }

    /// Marks a screen area for redraw.
    pub fn add_dirty_rect(&mut self, r: Rect) {
        if !r.is_empty() {
            self.dirty_rects.push(r);
        }
    }

    /// Marks a sprite for redraw.
    pub fn mark_dirty(&mut self, id: SpriteId) {
        if let Some(s) = self.get_mut(id) {
            s.dirty = true;
        }
    }

    /// Works out what to redraw. `draw_offset` is the global draw offset,
    /// applied to sprites that do not ignore it.
    pub fn plan_draw(&mut self, draw_offset: (i32, i32)) -> Vec<DirtyPass> {
        let list = self.display_list();
        let screen = self.screen;
        let mut rects = std::mem::take(&mut self.dirty_rects);
        let screen_bounds = |s: &Sprite| {
            let mut r = s.bounds().to_rect();
            if !s.ignores_draw_offset {
                r.x += draw_offset.0;
                r.y += draw_offset.1;
            }
            r
        };
        for id in &list {
            let s = self.get(*id).expect("listed sprite exists");
            let b = screen_bounds(s);
            let changed = s.dirty || s.always_redraw || s.last_bounds != Some(b);
            if changed && s.visible {
                rects.push(b);
            }
            if changed {
                if let Some(old) = s.last_bounds {
                    rects.push(old);
                }
            }
        }
        for id in &list {
            let s = self.get_mut(*id).expect("listed sprite exists");
            let b = screen_bounds(s);
            s.dirty = false;
            s.last_bounds = Some(b);
        }
        if self.always_redraw {
            rects = vec![screen];
        }
        let rects = merge_rects(
            rects
                .into_iter()
                .map(|r| r.intersect(&screen))
                .filter(|r| !r.is_empty())
                .collect(),
        );
        rects
            .into_iter()
            .map(|rect| {
                let mut items = vec![];
                let mut covered = false;
                for id in &list {
                    let s = self.get(*id).expect("listed sprite exists");
                    if !s.visible {
                        continue;
                    }
                    let b = screen_bounds(s);
                    if b.intersect(&rect).is_empty() {
                        continue;
                    }
                    if s.opaque && b.intersect(&rect) == rect {
                        // An opaque sprite covering the whole rect hides
                        // everything below it.
                        items.clear();
                        covered = true;
                    }
                    if s.has_draw_callback {
                        let local = Rect::new(rect.x - b.x, rect.y - b.y, rect.w, rect.h)
                            .intersect(&Rect::new(0, 0, b.w, b.h));
                        items.push(DrawItem::Callback {
                            sprite: *id,
                            x: b.x,
                            y: b.y,
                            local,
                        });
                    } else if let Some(img) = s.image {
                        items.push(DrawItem::Image {
                            sprite: *id,
                            image: img,
                            x: b.x,
                            y: b.y,
                            flip: s.flip,
                            mode: s.draw_mode,
                        });
                    }
                }
                DirtyPass {
                    rect,
                    fill_background: !covered,
                    items,
                }
            })
            .collect()
    }

    fn candidates(&self, id: SpriteId) -> Vec<(SpriteId, RectF)> {
        let Some(me) = self.get(id) else { return vec![] };
        self.display_list()
            .into_iter()
            .filter(|o| *o != id)
            .filter_map(|o| {
                let s = self.get(o)?;
                if !s.collisions_enabled {
                    return None;
                }
                if me.collides_with != 0 && s.groups != 0 && me.collides_with & s.groups == 0 {
                    return None;
                }
                Some((o, s.world_collide_rect()?))
            })
            .collect()
    }

    /// Sprites whose collide rects overlap `id`'s.
    pub fn overlapping(&self, id: SpriteId) -> Vec<SpriteId> {
        let Some(r) = self.get(id).and_then(|s| s.world_collide_rect()) else {
            return vec![];
        };
        self.candidates(id)
            .into_iter()
            .filter(|(_, o)| o.overlaps(&r))
            .map(|(o, _)| o)
            .collect()
    }

    /// All pairs of overlapping sprites.
    pub fn all_overlapping(&self) -> Vec<(SpriteId, SpriteId)> {
        let list = self.display_list();
        let mut out = vec![];
        for (i, a) in list.iter().enumerate() {
            let Some(ra) = self
                .get(*a)
                .filter(|s| s.collisions_enabled)
                .and_then(|s| s.world_collide_rect())
            else {
                continue;
            };
            for b in &list[i + 1..] {
                if let Some(rb) = self
                    .get(*b)
                    .filter(|s| s.collisions_enabled)
                    .and_then(|s| s.world_collide_rect())
                {
                    if ra.overlaps(&rb) {
                        out.push((*a, *b));
                    }
                }
            }
        }
        out
    }

    /// Sprites whose bounds contain a point.
    pub fn query_point(&self, x: f32, y: f32) -> Vec<SpriteId> {
        self.display_list()
            .into_iter()
            .filter(|id| self.get(*id).is_some_and(|s| s.bounds().contains(x, y)))
            .collect()
    }

    /// Sprites whose bounds overlap a rect.
    pub fn query_rect(&self, r: RectF) -> Vec<SpriteId> {
        self.display_list()
            .into_iter()
            .filter(|id| self.get(*id).is_some_and(|s| s.bounds().overlaps(&r)))
            .collect()
    }

    /// Sprites whose bounds a line segment crosses.
    pub fn query_line(&self, x1: f32, y1: f32, x2: f32, y2: f32) -> Vec<SpriteId> {
        self.display_list()
            .into_iter()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|s| segment_hits(&s.bounds(), x1, y1, x2, y2).is_some())
            })
            .collect()
    }

    /// Other sprites a move of `id` to `goal` might touch (for asking the
    /// game for collision responses up front).
    pub fn move_candidates(&self, id: SpriteId, goal_x: f32, goal_y: f32) -> Vec<SpriteId> {
        let Some(s) = self.get(id) else { return vec![] };
        let Some(r) = s.world_collide_rect() else {
            return vec![];
        };
        let (dx, dy) = (goal_x - s.x, goal_y - s.y);
        let swept = RectF::new(
            r.x.min(r.x + dx),
            r.y.min(r.y + dy),
            r.w + dx.abs(),
            r.h + dy.abs(),
        );
        let grown = RectF::new(swept.x - 0.01, swept.y - 0.01, swept.w + 0.02, swept.h + 0.02);
        self.candidates(id)
            .into_iter()
            .filter(|(_, o)| o.overlaps(&grown))
            .map(|(o, _)| o)
            .collect()
    }

    /// Moves `id` towards `goal`, resolving collisions with `response(other)`.
    /// Returns the final position and the collisions, in order.
    pub fn move_with_collisions(
        &mut self,
        id: SpriteId,
        goal_x: f32,
        goal_y: f32,
        response: &dyn Fn(SpriteId) -> CollisionResponse,
    ) -> (f32, f32, Vec<Collision>) {
        let (cols, gx, gy) = self.resolve(id, goal_x, goal_y, response);
        self.modify(id, |s| {
            s.x = gx;
            s.y = gy;
        });
        (gx, gy, cols)
    }

    /// Like `move_with_collisions` but does not move the sprite.
    pub fn check_collisions(
        &self,
        id: SpriteId,
        goal_x: f32,
        goal_y: f32,
        response: &dyn Fn(SpriteId) -> CollisionResponse,
    ) -> (f32, f32, Vec<Collision>) {
        let (c, x, y) = self.resolve(id, goal_x, goal_y, response);
        (x, y, c)
    }

    fn resolve(
        &self,
        id: SpriteId,
        goal_x: f32,
        goal_y: f32,
        response: &dyn Fn(SpriteId) -> CollisionResponse,
    ) -> (Vec<Collision>, f32, f32) {
        let Some(s) = self.get(id) else {
            return (vec![], goal_x, goal_y);
        };
        let Some(start_rect) = s.world_collide_rect() else {
            return (vec![], goal_x, goal_y);
        };
        // Work with the collide rect's top-left; convert back at the end.
        let off = (start_rect.x - s.x, start_rect.y - s.y);
        let (mut x, mut y) = (start_rect.x, start_rect.y);
        let (mut gx, mut gy) = (goal_x + off.0, goal_y + off.1);
        let (w, h) = (start_rect.w, start_rect.h);
        let others = self.candidates(id);
        let mut visited: Vec<SpriteId> = vec![];
        let mut cols = vec![];
        for _ in 0..16 {
            let mut best: Option<(f32, SpriteId, RectF, i32, i32, bool)> = None;
            for (o, orect) in &others {
                if visited.contains(o) {
                    continue;
                }
                let me = RectF::new(x, y, w, h);
                if let Some((ti, nx, ny, ov)) = sweep(&me, gx - x, gy - y, orect) {
                    if best.is_none_or(|b| ti < b.0) {
                        best = Some((ti, *o, *orect, nx, ny, ov));
                    }
                }
            }
            let Some((ti, o, orect, nx, ny, overlaps)) = best else {
                break;
            };
            visited.push(o);
            let resp = response(o);
            let (dx, dy) = (gx - x, gy - y);
            let (tx, ty) = if overlaps {
                push_out(&RectF::new(x, y, w, h), &orect, nx, ny)
            } else {
                (x + dx * ti, y + dy * ti)
            };
            let mut c = Collision {
                sprite: id,
                other: o,
                response: resp,
                overlaps,
                ti,
                move_x: dx,
                move_y: dy,
                normal_x: nx,
                normal_y: ny,
                touch_x: tx - off.0,
                touch_y: ty - off.1,
                sprite_rect: RectF::new(x, y, w, h),
                other_rect: orect,
                slide: None,
                bounce: None,
            };
            match resp {
                CollisionResponse::Overlap => {}
                CollisionResponse::Freeze => {
                    x = tx;
                    y = ty;
                    gx = tx;
                    gy = ty;
                }
                CollisionResponse::Slide => {
                    if nx != 0 {
                        gx = tx;
                    }
                    if ny != 0 {
                        gy = ty;
                    }
                    x = tx;
                    y = ty;
                    c.slide = Some((gx - off.0, gy - off.1));
                }
                CollisionResponse::Bounce => {
                    let (rx, ry) = (gx - tx, gy - ty);
                    if nx != 0 {
                        gx = tx - rx;
                    }
                    if ny != 0 {
                        gy = ty - ry;
                    }
                    x = tx;
                    y = ty;
                    c.bounce = Some((gx - off.0, gy - off.1));
                }
            }
            cols.push(c);
            if resp == CollisionResponse::Freeze {
                break;
            }
        }
        (cols, gx - off.0, gy - off.1)
    }
}

/// Swept AABB test of `a` moving by `(dx, dy)` against static `b`. Returns
/// `(time of impact 0..=1, normal x, normal y, overlapping at start)`.
fn sweep(a: &RectF, dx: f32, dy: f32, b: &RectF) -> Option<(f32, i32, i32, bool)> {
    if a.overlaps(b) {
        // Normal along the axis of least penetration.
        let left = a.x + a.w - b.x;
        let right = b.x + b.w - a.x;
        let up = a.y + a.h - b.y;
        let down = b.y + b.h - a.y;
        let m = left.min(right).min(up).min(down);
        let (nx, ny) = if m == left {
            (-1, 0)
        } else if m == right {
            (1, 0)
        } else if m == up {
            (0, -1)
        } else {
            (0, 1)
        };
        return Some((0.0, nx, ny, true));
    }
    let (mut tmin, mut tmax) = (f32::NEG_INFINITY, f32::INFINITY);
    let mut normal = (0, 0);
    for axis in 0..2 {
        let (p, d, lo, hi, size) = if axis == 0 {
            (a.x, dx, b.x, b.x + b.w, a.w)
        } else {
            (a.y, dy, b.y, b.y + b.h, a.h)
        };
        // Moving interval [p, p+size] must overlap (lo, hi).
        if d == 0.0 {
            if p + size <= lo || p >= hi {
                return None;
            }
            continue;
        }
        let t1 = (lo - (p + size)) / d;
        let t2 = (hi - p) / d;
        let (enter, exit) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
        if enter > tmin {
            tmin = enter;
            normal = if axis == 0 {
                (if d > 0.0 { -1 } else { 1 }, 0)
            } else {
                (0, if d > 0.0 { -1 } else { 1 })
            };
        }
        tmax = tmax.min(exit);
    }
    if tmin < tmax && (0.0..1.0).contains(&tmin) {
        Some((tmin, normal.0, normal.1, false))
    } else {
        None
    }
}

fn push_out(a: &RectF, b: &RectF, nx: i32, ny: i32) -> (f32, f32) {
    match (nx, ny) {
        (-1, _) => (b.x - a.w, a.y),
        (1, _) => (b.x + b.w, a.y),
        (_, -1) => (a.x, b.y - a.h),
        _ => (a.x, b.y + b.h),
    }
}

fn segment_hits(r: &RectF, x1: f32, y1: f32, x2: f32, y2: f32) -> Option<f32> {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-dx, x1 - r.x),
        (dx, r.x + r.w - x1),
        (-dy, y1 - r.y),
        (dy, r.y + r.h - y1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 <= t1).then_some(t0)
}

/// Merges overlapping rects until none overlap.
fn merge_rects(mut rects: Vec<Rect>) -> Vec<Rect> {
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                if !rects[i].intersect(&rects[j]).is_empty() {
                    let u = rects[i].union(&rects[j]);
                    rects[i] = u;
                    rects.swap_remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(sys: &mut SpriteSystem, x: f32, y: f32, w: f32, h: f32) -> SpriteId {
        let id = sys.create();
        sys.modify(id, |s| {
            s.x = x;
            s.y = y;
            s.width = w;
            s.height = h;
            s.collide_rect = Some(RectF::new(0.0, 0.0, w, h));
        });
        sys.add(id);
        id
    }

    #[test]
    fn display_list_orders_by_z_then_insertion() {
        let mut sys = SpriteSystem::new();
        let a = boxed(&mut sys, 0.0, 0.0, 1.0, 1.0);
        let b = boxed(&mut sys, 0.0, 0.0, 1.0, 1.0);
        let c = boxed(&mut sys, 0.0, 0.0, 1.0, 1.0);
        sys.modify(a, |s| s.z = 5);
        assert_eq!(sys.display_list(), vec![b, c, a]);
    }

    #[test]
    fn slide_stops_at_wall_and_keeps_other_axis() {
        let mut sys = SpriteSystem::new();
        let player = boxed(&mut sys, 10.0, 10.0, 10.0, 10.0); // bounds 5..15
        let _wall = boxed(&mut sys, 40.0, 10.0, 10.0, 100.0); // x 35..45
        let (x, y, cols) = sys.move_with_collisions(player, 50.0, 20.0, &|_| CollisionResponse::Slide);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].normal_x, -1);
        assert!((x - 30.0).abs() < 1e-4, "x = {x}");
        assert!((y - 20.0).abs() < 1e-4);
    }

    #[test]
    fn freeze_and_overlap() {
        let mut sys = SpriteSystem::new();
        let p = boxed(&mut sys, 10.0, 10.0, 10.0, 10.0);
        let _w = boxed(&mut sys, 40.0, 10.0, 10.0, 10.0);
        let (x, _, _) = sys.check_collisions(p, 50.0, 10.0, &|_| CollisionResponse::Overlap);
        assert_eq!(x, 50.0);
        let (x, _, _) = sys.check_collisions(p, 50.0, 10.0, &|_| CollisionResponse::Freeze);
        assert!((x - 30.0).abs() < 1e-4);
    }

    #[test]
    fn group_masks_filter_collisions() {
        let mut sys = SpriteSystem::new();
        let p = boxed(&mut sys, 10.0, 10.0, 10.0, 10.0);
        let w = boxed(&mut sys, 40.0, 10.0, 10.0, 10.0);
        sys.modify(p, |s| s.collides_with = 0b10);
        sys.modify(w, |s| s.groups = 0b01);
        let (x, _, cols) = sys.check_collisions(p, 50.0, 10.0, &|_| CollisionResponse::Slide);
        assert!(cols.is_empty());
        assert_eq!(x, 50.0);
    }

    #[test]
    fn plan_draw_only_redraws_changes() {
        let mut sys = SpriteSystem::new();
        let a = boxed(&mut sys, 20.0, 20.0, 10.0, 10.0);
        sys.modify(a, |s| s.image = Some(7));
        let first = sys.plan_draw((0, 0));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].rect, Rect::new(15, 15, 10, 10));
        assert!(sys.plan_draw((0, 0)).is_empty(), "nothing moved");
        sys.modify(a, |s| s.x = 100.0);
        let moved = sys.plan_draw((0, 0));
        assert_eq!(moved.len(), 2, "old and new bounds");
        let total: usize = moved.iter().map(|p| p.items.len()).sum();
        assert_eq!(total, 1);
    }

    #[test]
    fn overlapping_and_queries() {
        let mut sys = SpriteSystem::new();
        let a = boxed(&mut sys, 10.0, 10.0, 10.0, 10.0);
        let b = boxed(&mut sys, 15.0, 15.0, 10.0, 10.0);
        let _c = boxed(&mut sys, 100.0, 100.0, 10.0, 10.0);
        assert_eq!(sys.overlapping(a), vec![b]);
        assert_eq!(sys.all_overlapping(), vec![(a, b)]);
        assert_eq!(sys.query_point(6.0, 6.0), vec![a]);
        assert_eq!(sys.query_line(0.0, 0.0, 200.0, 200.0).len(), 3);
    }
}
