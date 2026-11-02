//! Buttons, crank and accelerometer.

use serde::{Deserialize, Serialize};

/// Button bits, matching the SDK's `PDButtons`.
pub mod buttons {
    pub const LEFT: u8 = 1;
    pub const RIGHT: u8 = 2;
    pub const UP: u8 = 4;
    pub const DOWN: u8 = 8;
    pub const B: u8 = 16;
    pub const A: u8 = 32;

    /// Bit for a button name ("a", "b", "up", "down", "left", "right").
    pub fn from_name(s: &str) -> Option<u8> {
        Some(match s.to_ascii_lowercase().as_str() {
            "left" => LEFT,
            "right" => RIGHT,
            "up" => UP,
            "down" => DOWN,
            "b" => B,
            "a" => A,
            _ => return None,
        })
    }
}

/// Input state as the game sees it during a frame.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Input {
    /// Buttons held now.
    pub current: u8,
    /// Buttons pressed since the previous frame.
    pub pushed: u8,
    /// Buttons released since the previous frame.
    pub released: u8,
    /// Crank angle in degrees (0 = up, clockwise).
    pub crank_angle: f32,
    /// Crank change since the previous frame, in degrees.
    pub crank_change: f32,
    pub crank_docked: bool,
    pub accel: (f32, f32, f32),
    pub accel_enabled: bool,
    // Events received since the last frame began.
    #[serde(skip)]
    held: u8,
    #[serde(skip)]
    pending_pushed: u8,
    #[serde(skip)]
    pending_released: u8,
    #[serde(skip)]
    pending_crank: f32,
    #[serde(skip)]
    target_angle: f32,
}

impl Input {
    pub fn new() -> Self {
        Input {
            crank_docked: true,
            accel: (0.0, 1.0, 0.0),
            ..Default::default()
        }
    }

    /// A button went down.
    pub fn press(&mut self, b: u8) {
        if self.held & b != b {
            self.pending_pushed |= b & !self.held;
        }
        self.held |= b;
    }

    /// A button went up.
    pub fn release(&mut self, b: u8) {
        self.pending_released |= b & self.held;
        self.held &= !b;
    }

    /// Turns the crank by `delta` degrees (undocking it).
    pub fn crank_by(&mut self, delta: f32) {
        self.crank_docked = false;
        self.pending_crank += delta;
        self.target_angle = (self.target_angle + delta).rem_euclid(360.0);
    }

    /// Sets the crank to an absolute angle.
    pub fn crank_to(&mut self, angle: f32) {
        let mut d = angle - self.target_angle;
        if d > 180.0 {
            d -= 360.0;
        } else if d < -180.0 {
            d += 360.0;
        }
        self.crank_by(d);
    }

    /// Docks or undocks the crank.
    pub fn set_docked(&mut self, docked: bool) {
        self.crank_docked = docked;
    }

    /// Sets the accelerometer reading.
    pub fn set_accel(&mut self, x: f32, y: f32, z: f32) {
        self.accel = (x, y, z);
    }

    /// Applies the events received since the last frame.
    pub fn begin_frame(&mut self) {
        self.current = self.held | self.pending_pushed;
        self.pushed = self.pending_pushed;
        self.released = self.pending_released;
        self.pending_pushed = 0;
        self.pending_released = 0;
        self.crank_change = self.pending_crank;
        self.pending_crank = 0.0;
        self.crank_angle = self.target_angle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_tap_is_seen_once() {
        let mut i = Input::new();
        i.press(buttons::A);
        i.release(buttons::A);
        i.begin_frame();
        assert_eq!(i.pushed, buttons::A);
        assert_eq!(i.released, buttons::A);
        assert_eq!(i.current, buttons::A);
        i.begin_frame();
        assert_eq!(i.pushed | i.released | i.current, 0);
    }

    #[test]
    fn held_button() {
        let mut i = Input::new();
        i.press(buttons::UP);
        i.begin_frame();
        i.begin_frame();
        assert_eq!(i.current, buttons::UP);
        assert_eq!(i.pushed, 0);
    }

    #[test]
    fn crank_wraps() {
        let mut i = Input::new();
        i.crank_by(350.0);
        i.begin_frame();
        assert_eq!(i.crank_change, 350.0);
        i.crank_to(10.0);
        i.begin_frame();
        assert!((i.crank_change - 20.0).abs() < 1e-4);
        assert!((i.crank_angle - 10.0).abs() < 1e-4);
        assert!(!i.crank_docked);
    }
}
