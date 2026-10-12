//! Latency summaries.

use std::time::Duration;

/// Timings of one measure, in milliseconds.
#[derive(Debug, Clone)]
pub struct Summary {
    pub samples: usize,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

/// The nearest-rank percentile `p` (0 to 100) of `samples`, which must not be empty.
pub fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * sorted_ms.len() as f64).ceil() as usize;
    sorted_ms[rank.clamp(1, sorted_ms.len()) - 1]
}

pub fn summarise(samples: &[Duration]) -> Summary {
    let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    Summary {
        samples: ms.len(),
        p50: percentile(&ms, 50.0),
        p95: percentile(&ms, 95.0),
        max: *ms.last().expect("at least one sample"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank() {
        let ms: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&ms, 95.0), 95.0);
        assert_eq!(percentile(&ms, 50.0), 50.0);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
        let s = summarise(&[Duration::from_millis(3), Duration::from_millis(1)]);
        assert_eq!((s.p50, s.p95, s.max), (1.0, 3.0, 3.0));
    }
}
