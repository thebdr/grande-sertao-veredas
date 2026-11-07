//! Largest-triangle-three-buckets downsampling: a windowed query shrunk to a chart's point
//! budget with its shape kept, so a peak or an outlier (a plant's deviation) survives where a
//! stride would drop it. Server-side, so every client sees the same picture for one window.

/// Downsample to at most `budget` points. The first and the last point always survive; a
/// budget under three degenerates to the endpoints; an input at or under the budget passes
/// through unchanged. The input is time-ascending (the store orders it).
pub fn downsample(points: &[(i64, f64)], budget: usize) -> Vec<(i64, f64)> {
    let n = points.len();
    if n <= budget || n <= 2 {
        return points.to_vec();
    }
    if budget <= 2 {
        return vec![points[0], points[n - 1]];
    }
    let mut out = Vec::with_capacity(budget);
    out.push(points[0]);
    // The middle points fall into budget - 2 buckets.
    let buckets = budget - 2;
    let span = (n - 2) as f64 / buckets as f64;
    let mut prev = points[0];
    for b in 0..buckets {
        let start = 1 + (b as f64 * span).floor() as usize;
        let end = (1 + ((b + 1) as f64 * span).floor() as usize).min(n - 1).max(start + 1);
        // The next anchor is the following bucket's average; the last bucket anchors on the
        // final point.
        let (next_start, next_end) = if b + 1 < buckets {
            (1 + ((b + 1) as f64 * span).floor() as usize, (1 + ((b + 2) as f64 * span).floor() as usize).min(n - 1))
        } else {
            (n - 1, n)
        };
        let next_len = (next_end - next_start).max(1);
        let (mut ax, mut ay) = (0.0, 0.0);
        for p in &points[next_start..next_start + next_len] {
            ax += p.0 as f64;
            ay += p.1;
        }
        ax /= next_len as f64;
        ay /= next_len as f64;
        // The bucket's point forming the largest triangle with the kept point and the anchor.
        let mut best = points[start];
        let mut best_area = -1.0f64;
        for &p in &points[start..end] {
            let area = ((prev.0 as f64 - ax) * (p.1 - prev.1) - (prev.0 as f64 - p.0 as f64) * (ay - prev.1)).abs();
            if area > best_area {
                best_area = area;
                best = p;
            }
        }
        out.push(best);
        prev = best;
    }
    out.push(points[n - 1]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(n: usize) -> Vec<(i64, f64)> {
        (0..n).map(|i| (i as i64, (i as f64 * 0.1).sin())).collect()
    }

    #[test]
    fn under_budget_passes_through() {
        let pts = series(50);
        assert_eq!(downsample(&pts, 100), pts);
        assert_eq!(downsample(&pts, 50), pts);
    }

    #[test]
    fn respects_budget_and_keeps_endpoints() {
        let pts = series(10_000);
        let out = downsample(&pts, 200);
        assert_eq!(out.len(), 200);
        assert_eq!(out[0], pts[0]);
        assert_eq!(*out.last().unwrap(), *pts.last().unwrap());
        assert!(out.windows(2).all(|w| w[0].0 < w[1].0), "one point per bucket, time ascending");
    }

    #[test]
    fn a_spike_survives_downsampling() {
        let mut pts = series(5_000);
        pts[2_517].1 = 99.0;
        let out = downsample(&pts, 100);
        assert!(out.iter().any(|p| p.1 == 99.0), "the spike must survive");
    }

    #[test]
    fn tiny_budgets_degenerate_to_endpoints() {
        let pts = series(100);
        assert_eq!(downsample(&pts, 2), vec![pts[0], pts[99]]);
    }
}
