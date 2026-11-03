//! Pure geometry for pre-generation. No Pumpkin types in here, so it is easy to test.
//!
//! The area is walked as square rings growing outward from the centre chunk
//! (ring 0 = the centre, ring 1 = the 8 chunks around it, ring 2 = the next 16 ...).
//! That means the middle is always finished first, like Chunky. A circle is the same
//! walk with the cells outside the circle skipped.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Square,
    Circle,
}

impl Shape {
    pub fn parse(s: &str) -> Shape {
        if s.eq_ignore_ascii_case("circle") {
            Shape::Circle
        } else {
            Shape::Square
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Shape::Square => "square",
            Shape::Circle => "circle",
        }
    }
}

/// Where we are in the walk. Small and `Copy` so it can be saved to disk and resumed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Cursor {
    pub ring: i32,
    pub pos: i32,
}

/// A world coordinate in blocks to the chunk that contains it (floor division,
/// so -1 is chunk -1, not chunk 0).
pub fn block_to_chunk(block: i32) -> i32 {
    block.div_euclid(16)
}

/// A radius in blocks to a radius in chunks, rounded up so the area is never smaller
/// than what was asked for.
pub fn blocks_to_chunk_radius(blocks: i32) -> i32 {
    (blocks.max(0) + 15) / 16
}

/// How many chunks are on ring `r`.
pub fn ring_len(r: i32) -> i32 {
    if r == 0 { 1 } else { 8 * r }
}

/// The `p`-th chunk (offset from the centre) on ring `r`.
/// Walks the four sides of the square; every chunk on the ring appears exactly once.
pub fn ring_cell(r: i32, p: i32) -> (i32, i32) {
    if r == 0 {
        return (0, 0);
    }
    let side_len = 2 * r;
    let side = p / side_len;
    let t = p % side_len;
    match side {
        0 => (-r + t, -r),
        1 => (r, -r + t),
        2 => (r - t, r),
        _ => (-r, r - t),
    }
}

/// Is the chunk at offset (dx, dz) part of the area?
pub fn inside(shape: Shape, radius: i32, dx: i32, dz: i32) -> bool {
    match shape {
        Shape::Square => true,
        Shape::Circle => {
            let (dx, dz, r) = (dx as i64, dz as i64, radius as i64);
            dx * dx + dz * dz <= r * r
        }
    }
}

fn isqrt(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut x = (n as f64).sqrt() as i64;
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

/// Exactly how many chunks the area contains. Needed for the percentage and ETA.
/// The circle is counted row by row, so this is O(radius), not O(radius squared).
pub fn total_chunks(shape: Shape, radius: i32) -> u64 {
    let r = radius.max(0) as i64;
    match shape {
        Shape::Square => ((2 * r + 1) * (2 * r + 1)) as u64,
        Shape::Circle => {
            let mut total: i64 = 0;
            for dz in -r..=r {
                total += 2 * isqrt(r * r - dz * dz) + 1;
            }
            total as u64
        }
    }
}

impl Cursor {
    /// Returns the next chunk offset to generate and moves forward,
    /// or `None` when the whole area is done.
    pub fn next(&mut self, shape: Shape, radius: i32) -> Option<(i32, i32)> {
        loop {
            if self.ring > radius {
                return None;
            }
            let (dx, dz) = ring_cell(self.ring, self.pos);
            self.pos += 1;
            if self.pos >= ring_len(self.ring) {
                self.ring += 1;
                self.pos = 0;
            }
            if inside(shape, radius, dx, dz) {
                return Some((dx, dz));
            }
        }
    }

    pub fn is_finished(&self, radius: i32) -> bool {
        self.ring > radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn walk(shape: Shape, r: i32) -> Vec<(i32, i32)> {
        let mut c = Cursor::default();
        let mut out = Vec::new();
        while let Some(cell) = c.next(shape, r) {
            out.push(cell);
        }
        out
    }

    #[test]
    fn every_cell_visited_exactly_once() {
        for shape in [Shape::Square, Shape::Circle] {
            for r in 0..40 {
                let cells = walk(shape, r);
                let unique: HashSet<_> = cells.iter().copied().collect();
                let mut expected = HashSet::new();
                for dx in -r..=r {
                    for dz in -r..=r {
                        if inside(shape, r, dx, dz) {
                            expected.insert((dx, dz));
                        }
                    }
                }
                assert_eq!(cells.len(), unique.len(), "duplicates {shape:?} r={r}");
                assert_eq!(unique, expected, "wrong cells {shape:?} r={r}");
                assert_eq!(cells.len() as u64, total_chunks(shape, r), "count {shape:?} r={r}");
            }
        }
    }

    #[test]
    fn walks_outward_from_the_centre() {
        let cells = walk(Shape::Square, 20);
        let dist: Vec<i32> = cells.iter().map(|(x, z)| x.abs().max(z.abs())).collect();
        let mut sorted = dist.clone();
        sorted.sort();
        assert_eq!(dist, sorted);
        assert_eq!(cells[0], (0, 0));
    }

    #[test]
    fn resume_from_saved_cursor_gives_the_same_walk() {
        let all = walk(Shape::Circle, 12);
        let mut c = Cursor::default();
        for _ in 0..57 {
            c.next(Shape::Circle, 12);
        }
        let saved = c; // pretend this was written to disk and reloaded
        let mut resumed = saved;
        let mut rest = Vec::new();
        while let Some(cell) = resumed.next(Shape::Circle, 12) {
            rest.push(cell);
        }
        assert_eq!(&all[57..], &rest[..]);
    }

    #[test]
    fn chunk_conversion_handles_negatives() {
        assert_eq!(block_to_chunk(0), 0);
        assert_eq!(block_to_chunk(15), 0);
        assert_eq!(block_to_chunk(16), 1);
        assert_eq!(block_to_chunk(-1), -1);
        assert_eq!(block_to_chunk(-16), -1);
        assert_eq!(block_to_chunk(-17), -2);
        assert_eq!(blocks_to_chunk_radius(1), 1);
        assert_eq!(blocks_to_chunk_radius(16), 1);
        assert_eq!(blocks_to_chunk_radius(17), 2);
        assert_eq!(blocks_to_chunk_radius(1000), 63);
    }
}
