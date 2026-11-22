//! Veredas fork change 14: an inverse distribution function to full precision.
//!
//! statrs' `inverse_cdf` searches for `x` with `cdf(x) = p` and stops at a tolerance, so its last
//! digits depend on the float functions the build happens to use (`std` or `libm`): CHISQ.INV.RT
//! (0.9, 10) came out 4.8651820519… on one and 4.8651820525… on the other, the true value being
//! 4.865182051925328. A few Newton steps on the distribution's own `cdf` and `pdf` bring either to
//! the root, as Excel computes it; a step is kept only when it brings `cdf(x)` closer to `p`, so it
//! can never make the answer worse.

use statrs::distribution::{Continuous, ContinuousCDF};

/// `x0` (a first answer to `cdf(x) = p`) moved to the root by Newton's method on `dist`.
pub(crate) fn polish<D>(dist: &D, p: f64, x0: f64) -> f64
where
    D: ContinuousCDF<f64, f64> + Continuous<f64, f64>,
{
    if !x0.is_finite() {
        return x0;
    }
    let mut x = x0;
    let mut residual = (dist.cdf(x) - p).abs();
    for _ in 0..8 {
        if residual == 0.0 {
            break;
        }
        let density = dist.pdf(x);
        if !(density.is_finite() && density > 0.0) {
            break;
        }
        let next = x - (dist.cdf(x) - p) / density;
        if !next.is_finite() || next == x {
            break;
        }
        let next_residual = (dist.cdf(next) - p).abs();
        if next_residual >= residual {
            break;
        }
        x = next;
        residual = next_residual;
    }
    x
}

#[cfg(test)]
mod tests {
    use statrs::distribution::{ChiSquared, Gamma};

    use super::polish;

    #[test]
    fn the_chi_squared_quantile_reaches_the_root() {
        let dist = ChiSquared::new(10.0).unwrap();
        // the true 0.1 quantile, from P(5, y) = 0.1 in closed form
        let truth = 4.865182051925328;
        for start in [4.8651820519, 4.8651820525, 4.8, 5.0] {
            let x = polish(&dist, 0.1, start);
            // within what the distribution's own cdf can tell apart (about two units in the last
            // place here)
            assert!((x - truth).abs() <= 1e-14 * truth, "{start}: {x}");
        }
    }

    #[test]
    fn a_step_that_would_not_improve_is_not_taken() {
        let dist = Gamma::new(2.0, 1.0).unwrap();
        // at the root already, nothing moves; a non-finite start is returned as it is
        let x = polish(&dist, 0.5, polish(&dist, 0.5, 1.6783469900166608));
        assert_eq!(polish(&dist, 0.5, x), x);
        assert!(polish(&dist, 0.5, f64::NAN).is_nan());
        assert_eq!(polish(&dist, 0.5, f64::INFINITY), f64::INFINITY);
    }
}
