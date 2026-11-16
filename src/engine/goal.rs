//! Goal trigger geometry for mpl_arena_a, as the game itself tests it.
//!
//! Source: the level's two `CR15FlagBaseCR` entries
//! (`CR15FlagBaseCRWin10/576ed3f8428ebc4b` in package 48037dc70b0ecab2),
//! field +0x80: position, rotation (x, y, z, w) and box half extents, read by
//! echovr.exe at 0x14090fb50. The goal test (0x14098db04..0x14098dd96) takes
//! the disc's move since the last sample as a segment, rotates it into the
//! box's frame and slab-tests it against the box; a goal is the frame the
//! segment starts touching the box. The hit point is where the segment
//! enters the box, which is also where native pins the disc.
//!
//! Values are the exact f32 bits from the resource. The boxes are 2.2 m
//! squares turned 45 degrees about the arena's long axis (a diamond: edges
//! 1.10 m from the centre, corners 1.56 m out) and 8.4 cm deep.

/// One goal's trigger box (metres).
pub struct GoalBox {
    pub center: [f32; 3],
    /// Rotation quaternion (x, y, z, w).
    pub rot: [f32; 4],
    pub half: [f32; 3],
}

const fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// The goal blue defends (flag base team 0, at -z): orange scores here.
pub const BLUE_GOAL: GoalBox = GoalBox {
    center: [f(0x0000_0000), f(0x3d32_81aa), f(0xc210_4924)],
    rot: [f(0x3ec3_ef15), f(0x3f6c_835e), f(0x2482_71cd), f(0x23d8_20a7)],
    half: [f(0x3f8d_c9f4), f(0x3f8d_11ca), f(0x3d2c_a58f)],
};

/// The goal orange defends (flag base team 1, at +z): blue scores here.
pub const ORANGE_GOAL: GoalBox = GoalBox {
    center: [f(0x0000_0000), f(0x0000_0000), f(0x4210_2aab)],
    rot: [f(0xa4b8_79f9), f(0xa4b8_79f9), f(0x3ec3_ef16), f(0x3f6c_835e)],
    half: [f(0x3f8d_c9f3), f(0x3f8d_11ca), f(0x3d2c_a58c)],
};

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `v` rotated by the inverse of `q` (world into the box's frame).
fn unrotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let (u, w) = ([-q[0] / n, -q[1] / n, -q[2] / n], q[3] / n);
    let t = cross(u, v).map(|c| 2.0 * c);
    let ut = cross(u, t);
    [0, 1, 2].map(|i| v[i] + w * t[i] + ut[i])
}

impl GoalBox {
    /// Where the segment `a`→`b` (metres) enters the box, if it touches it.
    pub fn hit(&self, a: [f32; 3], b: [f32; 3]) -> Option<[f32; 3]> {
        let d = [0, 1, 2].map(|i| b[i] - a[i]);
        let p = unrotate(self.rot, [0, 1, 2].map(|i| a[i] - self.center[i]));
        let ld = unrotate(self.rot, d);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for i in 0..3 {
            let (lo, hi) = (-self.half[i], self.half[i]);
            if ld[i].abs() <= 1e-6 {
                if p[i] < lo || p[i] > hi {
                    return None;
                }
            } else {
                let (ta, tb) = ((lo - p[i]) / ld[i], (hi - p[i]) / ld[i]);
                t0 = t0.max(ta.min(tb));
                t1 = t1.min(ta.max(tb));
                if t0 > t1 {
                    return None;
                }
            }
        }
        Some([0, 1, 2].map(|i| a[i] + d[i] * t0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(v: [i32; 3]) -> [f32; 3] {
        v.map(|c| c as f32 / 1000.0)
    }

    #[test]
    fn native_goal_pins_where_native_did() {
        // native-20261111 t=113.166..113.181: the owner's disc crossing into
        // the orange goal; native pinned it on the box face, depth 36000.
        let h = ORANGE_GOAL.hit(m([-346, 465, 35995]), m([-345, 459, 36014])).expect("goal");
        let mm = h.map(|c| (c * 1000.0).round() as i32);
        assert_eq!(mm[2], 36000);
        assert!((mm[0] + 346).abs() <= 1 && (mm[1] - 464).abs() <= 1, "{mm:?}");
        // A move that stays in front of the box does not touch it.
        assert!(ORANGE_GOAL.hit(m([-347, 471, 35975]), m([-346, 465, 35995])).is_none());
    }

    #[test]
    fn diamond_not_circle() {
        let through = |g: &GoalBox, x: f32, y: f32| {
            let z = g.center[2];
            g.hit([x, y + g.center[1], z - 1.0], [x, y + g.center[1], z + 1.0]).is_some()
        };
        for g in [&BLUE_GOAL, &ORANGE_GOAL] {
            assert!(through(g, 0.0, 0.0));
            // On an axis the corner is 1.56 m out.
            assert!(through(g, 1.5, 0.0) && through(g, 0.0, -1.5));
            // On a diagonal the edge is 1.10 m out: 1.0 m in x and y misses.
            assert!(through(g, 0.75, 0.75));
            assert!(!through(g, 1.0, 1.0) && !through(g, -1.0, 0.9));
            assert!(!through(g, 1.6, 0.0));
        }
    }

    #[test]
    fn short_of_the_box_is_no_goal() {
        assert!(ORANGE_GOAL.hit([0.0, 0.0, 35.0], [0.0, 0.0, 35.99]).is_none());
        assert!(BLUE_GOAL.hit([0.0, 0.0, -35.0], [0.0, 0.0, -36.04]).is_some());
        assert!(BLUE_GOAL.hit([0.0, 0.0, -35.0], [0.0, 0.0, -36.02]).is_none());
    }
}
