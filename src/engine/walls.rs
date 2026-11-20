//! Each Combat map's solid level collision (`tools/gen_combat_collision.py`,
//! from the level's `CPhysicsResource`): what stands between an explosion and
//! a player. Native's blasts stop at walls; the server needs the same to keep
//! a DETONATOR from hurting players on the other side of one.
//!
//! The triangles go in a grid of `CELL`-metre cells (each triangle in every
//! cell its bounds touch); a segment is tested against the triangles of the
//! cells its own bounds touch. Blasts reach a few metres, so that is a handful
//! of cells. A map is loaded the first time one of its matches asks.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::combat::{LEVEL_COMBUSTION, LEVEL_DYSON, LEVEL_FISSION, LEVEL_GAUSS};

const CELL: f32 = 2.0;

type Tri = [[f32; 3]; 3];

pub struct Walls {
    tris: Vec<Tri>,
    grid: HashMap<(i32, i32, i32), Vec<u32>>,
}

/// `level`'s walls, if it is a Combat map or the arena (every surface there,
/// for telling a disc's bounce from a turn in the air).
pub fn of(level: u64) -> Option<&'static Walls> {
    static DYSON: OnceLock<Walls> = OnceLock::new();
    static COMBUSTION: OnceLock<Walls> = OnceLock::new();
    static FISSION: OnceLock<Walls> = OnceLock::new();
    static GAUSS: OnceLock<Walls> = OnceLock::new();
    static ARENA: OnceLock<Walls> = OnceLock::new();
    let (cell, bytes): (&OnceLock<Walls>, &[u8]) = match level {
        LEVEL_DYSON => (&DYSON, include_bytes!("data/combat_dyson_collision.bin")),
        LEVEL_COMBUSTION => (&COMBUSTION, include_bytes!("data/combat_combustion_collision.bin")),
        LEVEL_FISSION => (&FISSION, include_bytes!("data/combat_fission_collision.bin")),
        LEVEL_GAUSS => (&GAUSS, include_bytes!("data/combat_gauss_collision.bin")),
        super::msg::LEVEL_ARENA => (&ARENA, include_bytes!("data/arena_collision.bin")),
        _ => return None,
    };
    Some(cell.get_or_init(|| Walls::parse(bytes)))
}

fn key(p: [f32; 3]) -> (i32, i32, i32) {
    ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32)
}

impl Walls {
    /// `u32 bodies`, then per body `u32 vertices, u32 triangles`, the vertices
    /// (3 x i16 centimetres) and the triangles (3 x u16 indices).
    fn parse(b: &[u8]) -> Self {
        let u32_at = |o: usize| b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().expect("4"))) as usize;
        let i16_at = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
        let u16_at = |o: usize| usize::from(u16::from_le_bytes([b[o], b[o + 1]]));
        let mut tris = Vec::new();
        let mut o = 4;
        for _ in 0..u32_at(0) {
            let (nv, nt) = (u32_at(o), u32_at(o + 4));
            o += 8;
            if o + 6 * (nv + nt) > b.len() {
                break;
            }
            let verts: Vec<[f32; 3]> = (0..nv).map(|i| std::array::from_fn(|k| f32::from(i16_at(o + 6 * i + 2 * k)) / 100.0)).collect();
            o += 6 * nv;
            for i in 0..nt {
                let ix: [usize; 3] = std::array::from_fn(|k| u16_at(o + 6 * i + 2 * k));
                if ix.iter().all(|&v| v < verts.len()) {
                    tris.push(ix.map(|v| verts[v]));
                }
            }
            o += 6 * nt;
        }
        let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            let lo = key(std::array::from_fn(|k| t[0][k].min(t[1][k]).min(t[2][k])));
            let hi = key(std::array::from_fn(|k| t[0][k].max(t[1][k]).max(t[2][k])));
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        grid.entry((x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        Self { tris, grid }
    }

    pub fn triangles(&self) -> usize {
        self.tris.len()
    }

    /// Triangle `i`'s centre and unit normal (for tests).
    #[cfg(test)]
    pub fn face(&self, i: usize) -> ([f32; 3], [f32; 3]) {
        let t = self.tris[i];
        let c = std::array::from_fn(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.0);
        let (e1, e2) = ([t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]]);
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
        (c, n.map(|x| x / l))
    }

    /// Whether a wall crosses the segment from `a` to `b`, leaving out `skip`
    /// metres at each end (a blast on a surface, or a mine on one, touches
    /// that surface itself).
    pub fn blocked(&self, a: [f32; 3], b: [f32; 3], skip: f32) -> bool {
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len <= skip + skip {
            return false;
        }
        let from = skip / len;
        let to = 1.0 - from;
        let lo = key(std::array::from_fn(|k| a[k].min(b[k])));
        let hi = key(std::array::from_fn(|k| a[k].max(b[k])));
        let mut seen: Vec<u32> = Vec::new();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    let Some(cell) = self.grid.get(&(x, y, z)) else { continue };
                    for &i in cell {
                        if seen.contains(&i) {
                            continue;
                        }
                        seen.push(i);
                        if hit(&self.tris[i as usize], a, d).is_some_and(|t| t > from && t < to) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Möller–Trumbore: where along `a + t d` the ray meets the triangle.
fn hit(t: &Tri, a: [f32; 3], d: [f32; 3]) -> Option<f32> {
    let sub = |p: [f32; 3], q: [f32; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let cross = |p: [f32; 3], q: [f32; 3]| [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]];
    let dot = |p: [f32; 3], q: [f32; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let h = cross(d, e2);
    let det = dot(e1, h);
    if det.abs() < 1e-9 {
        return None;
    }
    let f = 1.0 / det;
    let s = sub(a, t[0]);
    let u = f * dot(s, h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = f * dot(d, q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(f * dot(e2, q))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_map_has_walls() {
        for l in [LEVEL_DYSON, LEVEL_COMBUSTION, LEVEL_FISSION, LEVEL_GAUSS] {
            assert!(of(l).expect("a combat map").triangles() > 50_000);
        }
        assert!(of(0).is_none());
    }

    #[test]
    fn a_triangle_between_blocks_and_one_behind_does_not() {
        let tri = [[0.0, -1.0, -1.0], [0.0, 1.0, -1.0], [0.0, 0.0, 2.0]];
        let mut w = Walls { tris: vec![tri], grid: HashMap::new() };
        w.grid.insert((0, 0, 0), vec![0]);
        w.grid.insert((-1, 0, 0), vec![0]);
        w.grid.insert((-1, -1, -1), vec![0]);
        w.grid.insert((0, -1, -1), vec![0]);
        w.grid.insert((0, 0, -1), vec![0]);
        w.grid.insert((-1, 0, -1), vec![0]);
        assert!(w.blocked([-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.1));
        assert!(!w.blocked([-1.0, 0.0, 0.0], [-0.5, 0.0, 0.0], 0.1), "stops short");
        assert!(!w.blocked([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.1), "on the surface itself");
    }

    /// Players can't pass through solid walls: their recorded paths (the user's
    /// Combustion match, 2026-11-19) should almost never cross the mesh. A
    /// mirrored or shifted mesh crosses all the time. Needs the capture.
    #[test]
    #[ignore]
    fn recorded_paths_stay_out_of_the_walls() {
        let Ok(csv) = std::fs::read_to_string("captures/netrec-20261119_200601_5640/msgs.csv") else { return };
        let w = of(LEVEL_COMBUSTION).expect("walls");
        let field = |d: &[u8], bit: usize| {
            let mut v = 0u32;
            for k in 0..19 {
                let i = bit + k;
                v |= u32::from((d[i >> 3] >> (i & 7)) & 1) << k;
            }
            (v as i32 - 250_000) as f32 / 1000.0
        };
        let (mut last, mut moves, mut crossed): (Option<[f32; 3]>, u32, u32) = (None, 0, 0);
        for line in csv.lines() {
            let c: Vec<&str> = line.split(',').collect();
            if c.len() < 8 || c[0] != "c2s" || c[6] != "64" || !c[4].contains("Physics") {
                continue;
            }
            let t: f32 = c[1].parse().unwrap_or(0.0);
            if !(380.0..1190.0).contains(&t) {
                continue;
            }
            let d: Vec<u8> = (0..c[7].len() / 2).map(|i| u8::from_str_radix(&c[7][2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
            if d.len() != 64 || d[4..8] != [0x8e, 0xc2, 0x1d, 0x00] {
                continue;
            }
            let p = [field(&d, 267), field(&d, 286), field(&d, 305)];
            if let Some(l) = last {
                let step = ((p[0] - l[0]).powi(2) + (p[1] - l[1]).powi(2) + (p[2] - l[2]).powi(2)).sqrt();
                if step > 0.05 && step < 1.0 {
                    moves += 1;
                    if w.blocked(l, p, 0.0) {
                        crossed += 1;
                    }
                }
            }
            last = Some(p);
        }
        println!("{moves} moves, {crossed} through a wall");
        assert!(moves > 1000 && crossed * 100 < moves, "{crossed} of {moves}");
    }
}
