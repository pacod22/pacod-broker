//! A decayed weighted mean.

use serde::{Deserialize, Serialize};

/// A weighted sum of observations in [0, 1] that halves every half-life.
///
/// Decay is applied lazily: the sums are scaled to the time of each update and when read, so
/// an update costs a multiplication and an `exp`, whatever the history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Decayed {
    /// Σ weight × value.
    pub wx: f64,
    /// Σ weight.
    pub w: f64,
    /// Number of observations, decayed like the weights.
    pub n: f64,
    /// Milliseconds since the epoch the sums are scaled to.
    pub at: i64,
}

impl Decayed {
    fn factor(from: i64, to: i64, half_life_ms: f64) -> f64 {
        if to <= from || half_life_ms <= 0.0 { 1.0 } else { 0.5f64.powf((to - from) as f64 / half_life_ms) }
    }

    fn decay_to(&mut self, at: i64, half_life_ms: f64) {
        let f = Self::factor(self.at, at, half_life_ms);
        self.wx *= f;
        self.w *= f;
        self.n *= f;
        self.at = self.at.max(at);
    }

    /// Adds an observation `x` in [0, 1] with `weight`, at `at`.
    pub fn add(&mut self, x: f64, weight: f64, at: i64, half_life_ms: f64) {
        if !(x.is_finite() && weight.is_finite() && weight > 0.0) {
            return;
        }
        let x = x.clamp(0.0, 1.0);
        if self.at == 0 {
            self.at = at;
        }
        // Evidence that arrives late (a batch of usage reports) is decayed to its own time
        // before it is added, so it never counts more than evidence that arrived on time.
        let f = if at < self.at { Self::factor(at, self.at, half_life_ms) } else { 1.0 };
        self.decay_to(at, half_life_ms);
        self.wx += weight * x * f;
        self.w += weight * f;
        self.n += f;
    }

    /// The sums as they stand at `now`.
    pub fn at(&self, now: i64, half_life_ms: f64) -> Decayed {
        let mut c = *self;
        c.decay_to(now, half_life_ms);
        c
    }

    /// The smoothed mean: the evidence pulled toward `prior` with weight `k`.
    pub fn smoothed(&self, prior: f64, k: f64) -> f64 {
        if self.w + k <= 0.0 { prior } else { (self.wx + k * prior) / (self.w + k) }
    }

    /// The plain mean, when there is evidence.
    pub fn mean(&self) -> Option<f64> {
        (self.w > 0.0).then(|| self.wx / self.w)
    }
}

/// A decayed mean of a value that is not in [0, 1] (latency, in milliseconds).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct DecayedValue {
    pub sum: f64,
    pub w: f64,
    pub at: i64,
}

impl DecayedValue {
    pub fn add(&mut self, v: f64, weight: f64, at: i64, half_life_ms: f64) {
        if !(v.is_finite() && v >= 0.0 && weight > 0.0) {
            return;
        }
        if self.at == 0 {
            self.at = at;
        }
        let f = Decayed::factor(self.at, at, half_life_ms);
        self.sum *= f;
        self.w *= f;
        self.at = self.at.max(at);
        self.sum += v * weight;
        self.w += weight;
    }

    pub fn mean(&self) -> Option<f64> {
        (self.w > 1e-9).then(|| self.sum / self.w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: f64 = 86_400_000.0;

    #[test]
    fn evidence_halves_every_half_life() {
        let mut d = Decayed::default();
        d.add(1.0, 1.0, 1, 30.0 * DAY);
        let later = d.at(1 + (30.0 * DAY) as i64, 30.0 * DAY);
        assert!((later.w - 0.5).abs() < 1e-9);
        assert_eq!(later.mean(), Some(1.0));
    }

    #[test]
    fn smoothing_pulls_little_evidence_toward_the_prior() {
        let mut d = Decayed::default();
        for i in 0..3 {
            d.add(1.0, 1.0, 1 + i, 30.0 * DAY);
        }
        assert!((d.smoothed(0.5, 10.0) - (3.0 + 5.0) / 13.0).abs() < 1e-9);
        for i in 0..300 {
            d.add(1.0, 1.0, 10 + i, 30.0 * DAY);
        }
        assert!(d.smoothed(0.5, 10.0) > 0.98);
    }

    #[test]
    fn late_evidence_counts_less() {
        let mut d = Decayed::default();
        d.add(1.0, 1.0, (60.0 * DAY) as i64, 30.0 * DAY);
        d.add(0.0, 1.0, (30.0 * DAY) as i64, 30.0 * DAY);
        assert!((d.w - 1.5).abs() < 1e-9, "{}", d.w);
    }
}
