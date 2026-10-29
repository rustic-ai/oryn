//! Native normalized media intervals with fixed physical storage.
//! Resource and playback callers own availability and clock/session validation.

/// Bound each native interval inventory independently of Vec growth policy.
pub(super) const CAPACITY: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Ranges {
    intervals: [(f64, f64); CAPACITY],
    len: usize,
}

impl Default for Ranges {
    fn default() -> Self {
        Self {
            intervals: [(0., 0.); CAPACITY],
            len: 0,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Error {
    InvalidInterval,
    Capacity,
    IndexSize,
}

impl Ranges {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn interval(&self, index: u32) -> Result<(f64, f64), Error> {
        self.as_slice()
            .get(index as usize)
            .copied()
            .ok_or(Error::IndexSize)
    }

    fn as_slice(&self) -> &[(f64, f64)] {
        &self.intervals[..self.len]
    }

    pub(super) fn clear(&mut self) {
        self.intervals[..self.len].fill((0., 0.));
        self.len = 0;
    }

    pub(super) fn insert(&mut self, start: f64, end: f64) -> Result<(), Error> {
        if !start.is_finite() || !end.is_finite() || start < 0. || end < start {
            return Err(Error::InvalidInterval);
        }
        if start == end {
            return Ok(());
        }
        let first = self.as_slice().partition_point(|range| range.1 < start);
        let mut stop = first;
        let mut merged = (start, end);
        while stop < self.len && self.intervals[stop].0 <= merged.1 {
            merged.0 = merged.0.min(self.intervals[stop].0);
            merged.1 = merged.1.max(self.intervals[stop].1);
            stop += 1;
        }
        let count = self.len - (stop - first) + 1;
        if count > CAPACITY {
            return Err(Error::Capacity);
        }
        // No mutation before capacity admission, no heap allocation during insertion.
        self.intervals.copy_within(stop..self.len, first + 1);
        self.intervals[first] = merged;
        if count < self.len {
            self.intervals[count..self.len].fill((0., 0.));
        }
        self.len = count;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_snapshots_preserve_bounds_and_are_independent() {
        let mut ranges = Ranges::default();
        ranges.insert(4., 6.).unwrap();
        ranges.insert(1., 2.).unwrap();
        let snapshot = ranges.clone();
        ranges.insert(2., 4.).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges.interval(0), Ok((1., 6.)));
        assert_eq!(snapshot.interval(0), Ok((1., 2.)));
        assert_eq!(snapshot.interval(1), Ok((4., 6.)));
        assert_eq!(snapshot.interval(2), Err(Error::IndexSize));
        assert_eq!(snapshot.interval(u32::MAX), Err(Error::IndexSize));
        ranges.clear();
        assert_eq!(ranges.len(), 0);
        assert_eq!(ranges.interval(0), Err(Error::IndexSize));
    }

    #[test]
    fn capacity_failure_is_atomic_and_merges_still_admitted() {
        let mut ranges = Ranges::default();
        for i in 0..CAPACITY {
            ranges.insert((2 * i) as f64, (2 * i + 1) as f64).unwrap();
        }
        let snapshot = ranges.clone();
        assert_eq!(ranges.insert(1024., 1025.), Err(Error::Capacity));
        assert_eq!(ranges, snapshot);
        ranges.insert(0., (2 * CAPACITY) as f64).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(
            std::mem::size_of::<Ranges>(),
            CAPACITY * 16 + std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn invalid_interval_leaves_existing_state_unchanged() {
        let mut ranges = Ranges::default();
        ranges.insert(1., 2.).unwrap();
        let snapshot = ranges.clone();
        for (a, b) in [(f64::NAN, 1.), (0., f64::INFINITY), (-1., 0.), (2., 1.)] {
            assert_eq!(ranges.insert(a, b), Err(Error::InvalidInterval));
            assert_eq!(ranges, snapshot);
        }
        ranges.insert(3., 3.).unwrap();
        assert_eq!(ranges, snapshot);
    }
}
