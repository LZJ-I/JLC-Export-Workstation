use crate::error::{Error, Result};

/// Electrical connection grid in mils. Artwork and lettering keep a finer grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchematicGrid(u32);

impl Default for SchematicGrid {
    fn default() -> Self { Self(10) }
}

impl SchematicGrid {
    pub const MIN: u32 = 1;
    pub const MAX: u32 = 1000;

    pub fn new(mils: u32) -> Result<Self> {
        if !(Self::MIN..=Self::MAX).contains(&mils) {
            return Err(Error::msg("PADS 连接栅格必须为 1–1000 mil 的整数"));
        }
        Ok(Self(mils))
    }

    pub fn mils(self) -> u32 { self.0 }

    // Retain the established 10 mil artwork resolution where possible, while
    // allowing e.g. 25 mil and 7 mil terminals to survive ASCII serialization.
    pub(super) fn coordinate(self, mm: f64) -> f64 {
        let (mut a, mut b) = (10, self.0);
        while b != 0 { (a, b) = (b, a % b); }
        let resolution = f64::from(a);
        (super::mils(mm) / resolution).round() * resolution
    }

    pub(super) fn ceil(self, mils: f64) -> f64 {
        let grid = f64::from(self.0);
        (mils / grid - 1e-9).ceil() * grid
    }
}

impl std::str::FromStr for SchematicGrid {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        Self::new(value.parse().map_err(|_| Error::msg("PADS 连接栅格必须为 1–1000 mil 的整数"))?)
    }
}

/// Monotone, expansion-only mapping. Distinct source rows/columns cannot round
/// onto each other; deliberately shared coordinates stay shared. Zero is an
/// explicit constraint so the symbol's insertion origin has the same phase.
#[derive(Debug)]
pub(super) struct AxisMap { points: Vec<(f64, f64)> }

impl AxisMap {
    pub fn new(values: impl Iterator<Item = f64>, lanes: Vec<f64>, min_pitch: f64, grid: SchematicGrid) -> Self {
        let mut source: Vec<_> = values.chain([0.0]).collect();
        source.sort_by(f64::total_cmp);
        source.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
        let origin = source.iter().position(|v| v.abs() < 1e-7).unwrap();
        let mut points: Vec<_> = source.into_iter().map(|v| (v, 0.0)).collect();
        let mut previous_lane = None;
        for i in 0..points.len() {
            if i > 0 {
                points[i].1 = points[i - 1].1 + grid.ceil(points[i].0 - points[i - 1].0);
            }
            if lanes.iter().any(|v| (*v - points[i].0).abs() < 1e-7) {
                if let Some(previous) = previous_lane {
                    points[i].1 = points[i].1.max(previous + grid.ceil(min_pitch));
                }
                previous_lane = Some(points[i].1);
            }
        }
        let offset = points[origin].1;
        for (_, target) in &mut points { *target -= offset; }
        Self { points }
    }

    pub fn map(&self, value: f64) -> f64 {
        let upper = self.points.partition_point(|&(from, _)| from < value);
        if upper == 0 { return value + self.points[0].1 - self.points[0].0; }
        if upper == self.points.len() {
            let &(from, to) = self.points.last().unwrap();
            return value + to - from;
        }
        let (a, x) = self.points[upper - 1];
        let (b, y) = self.points[upper];
        x + (value - a) * (y - x) / (b - a)
    }

    pub fn cuts(&self, start: f64, end: f64) -> impl Iterator<Item = f64> + '_ {
        self.points.iter().filter_map(move |&(from, _)| {
            if (end - start).abs() < 1e-7 { return None; }
            let t = (from - start) / (end - start);
            (t > 1e-7 && t < 1.0 - 1e-7).then_some(t)
        })
    }
}
