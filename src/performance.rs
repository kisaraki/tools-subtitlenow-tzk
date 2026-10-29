//! Numeric, bounded, process-local diagnostics. No content, I/O or worker.
pub mod live;
pub mod resources;
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

const SUB_BUCKET_BITS: u32 = 3;
const SUB_BUCKETS: usize = 1 << SUB_BUCKET_BITS;
/// Eight subdivisions per power of two: exact below 16 ns; elsewhere the
/// reported quantile is a bucket upper bound, within 12.5% of its lower bound.
pub const DURATION_BUCKETS: usize =
    SUB_BUCKETS + (u64::BITS as usize - SUB_BUCKET_BITS as usize) * SUB_BUCKETS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurationDistribution {
    pub samples: u64,
    pub rejected_samples: u64,
    pub saturated: bool,
    pub p50_upper_ns: Option<u64>,
    pub p95_upper_ns: Option<u64>,
    pub max_ns: Option<u64>,
}

/// One fixed allocation at owner construction. Recording needs only clock and
/// atomic operations; percentile scans run on the diagnostic reader, not capture.
/// Concurrent snapshots are not transactions: max may include a newer sample.
pub(crate) struct DurationHistogram {
    buckets: [AtomicU64; DURATION_BUCKETS],
    maximum: AtomicU64,
    rejected: AtomicU64,
    saturated: AtomicBool,
}
impl Default for DurationHistogram {
    fn default() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            maximum: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            saturated: AtomicBool::new(false),
        }
    }
}
fn bucket(value: u64) -> usize {
    if value < SUB_BUCKETS as u64 {
        return value as usize;
    }
    let exponent = u64::BITS - 1 - value.leading_zeros();
    SUB_BUCKETS
        + (exponent - SUB_BUCKET_BITS) as usize * SUB_BUCKETS
        + ((value - (1u64 << exponent)) >> (exponent - SUB_BUCKET_BITS)) as usize
}
fn upper(index: usize) -> u64 {
    if index < SUB_BUCKETS {
        return index as u64;
    }
    let exponent = (index - SUB_BUCKETS) / SUB_BUCKETS + SUB_BUCKET_BITS as usize;
    let part = (index - SUB_BUCKETS) % SUB_BUCKETS;
    let bound = (1u128 << exponent)
        + (part as u128 + 1) * (1u128 << (exponent - SUB_BUCKET_BITS as usize))
        - 1;
    // The final bucket ends at u64::MAX, never at the following power of two.
    u64::try_from(bound).unwrap_or(u64::MAX)
}
impl DurationHistogram {
    fn increment(&self, value: &AtomicU64) {
        if value
            .fetch_update(Ordering::Release, Ordering::Relaxed, |old| {
                old.checked_add(1)
            })
            .is_err()
        {
            self.saturated.store(true, Ordering::Release);
        }
    }
    pub(crate) fn record(&self, duration: Duration) -> bool {
        let Ok(value) = u64::try_from(duration.as_nanos()) else {
            self.increment(&self.rejected);
            return false;
        };
        self.maximum.fetch_max(value, Ordering::Relaxed);
        self.increment(&self.buckets[bucket(value)]);
        true
    }
    pub(crate) fn record_since(&self, start: Instant) -> bool {
        if let Some(duration) = Instant::now().checked_duration_since(start) {
            self.record(duration)
        } else {
            self.increment(&self.rejected);
            false
        }
    }
    pub(crate) fn start(&self) -> DurationObservation<'_> {
        DurationObservation {
            histogram: self,
            start: Instant::now(),
        }
    }
    pub(crate) fn snapshot(&self) -> DurationDistribution {
        let counts: [u64; DURATION_BUCKETS] =
            std::array::from_fn(|index| self.buckets[index].load(Ordering::Acquire));
        let total: u128 = counts.iter().map(|count| u128::from(*count)).sum();
        let saturated = self.saturated.load(Ordering::Acquire) || total > u128::from(u64::MAX);
        let percentile = |percent: u128| {
            if total == 0 || saturated {
                return None;
            }
            let rank = (total * percent).div_ceil(100);
            let mut cumulative = 0u128;
            for (index, count) in counts.iter().enumerate() {
                cumulative += u128::from(*count);
                if cumulative >= rank {
                    return Some(upper(index));
                }
            }
            None
        };
        DurationDistribution {
            samples: u64::try_from(total).unwrap_or(u64::MAX),
            rejected_samples: self.rejected.load(Ordering::Relaxed),
            saturated,
            p50_upper_ns: percentile(50),
            p95_upper_ns: percentile(95),
            max_ns: (total > 0).then(|| self.maximum.load(Ordering::Relaxed)),
        }
    }
}
pub(crate) struct DurationObservation<'a> {
    histogram: &'a DurationHistogram,
    start: Instant,
}
impl Drop for DurationObservation<'_> {
    fn drop(&mut self) {
        self.histogram.record_since(self.start);
    }
}

#[cfg(test)]
#[path = "performance_tests.rs"]
mod tests;
