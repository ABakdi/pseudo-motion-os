//! Calibrated gaze estimation (CSL spec §6, ABI 1.17).
//!
//! Research-backed design: webcam gaze accuracy comes from **per-user
//! calibration**, not from heavier models — WebGazer reaches ~4° with a
//! self-calibrating regression over eye features, and the strongest
//! MediaPipe-based results use exactly this shape (iris + head features →
//! screen point via user-specific regression). We already run the
//! FaceLandmarker, so the whole upgrade is: a 9-point calibration screen
//! fitting a ridge regression from the per-frame feature vector to the
//! user's actual screen. Falls back to the coarse heuristic uncalibrated.

use serde::{Deserialize, Serialize};

/// Raw features per frame, in this order (produced by the gesture worker):
/// iris-in-eye ratios (hxR, hxL, vyR, vyL), head yaw/pitch proxies, the
/// eyeLook blendshape pair (lookH, lookV), nose position (x, y — head
/// translation), inter-ocular distance (camera distance) and eye-line roll.
pub const FEATS: usize = 12;

/// Design vector: bias + raw features + a few interaction/quadratic terms
/// (screen mapping is mildly nonlinear in head pose × eye-in-head).
pub fn expand(f: &[f32]) -> Option<Vec<f32>> {
    if f.len() < FEATS || f.iter().take(FEATS).any(|v| !v.is_finite()) {
        return None;
    }
    let hx = (f[0] + f[1]) * 0.5;
    let vy = (f[2] + f[3]) * 0.5;
    let (yaw, pitch) = (f[4], f[5]);
    let mut v = Vec::with_capacity(FEATS + 5);
    v.push(1.0);
    v.extend_from_slice(&f[..FEATS]);
    v.extend_from_slice(&[hx * yaw, vy * pitch, hx * hx, vy * vy]);
    Some(v)
}

/// The fitted per-user mapping, persisted to /settings/gaze_calib.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GazeCalib {
    pub wx: Vec<f32>,
    pub wy: Vec<f32>,
}

impl GazeCalib {
    /// Screen-fraction prediction from a raw feature vector.
    pub fn predict(&self, feats: &[f32]) -> Option<(f32, f32)> {
        let x = expand(feats)?;
        if x.len() != self.wx.len() || x.len() != self.wy.len() {
            return None; // stale calibration from an older feature layout
        }
        let dot = |w: &[f32]| w.iter().zip(&x).map(|(a, b)| a * b).sum::<f32>();
        Some((dot(&self.wx), dot(&self.wy)))
    }
}

/// Ridge regression via the normal equations (XᵀX + λI)w = Xᵀy, solved with
/// Gaussian elimination + partial pivoting in f64. Sample counts here are
/// tiny (9 points × ~25 frames), so this is exact and instant.
pub fn ridge_fit(xs: &[Vec<f32>], ys: &[f32], lambda: f64) -> Option<Vec<f32>> {
    let n = xs.len();
    if n == 0 || n != ys.len() {
        return None;
    }
    let d = xs[0].len();
    let mut a = vec![vec![0f64; d]; d];
    let mut b = vec![0f64; d];
    for (x, &y) in xs.iter().zip(ys) {
        if x.len() != d {
            return None;
        }
        for i in 0..d {
            b[i] += x[i] as f64 * y as f64;
            for j in 0..d {
                a[i][j] += x[i] as f64 * x[j] as f64;
            }
        }
    }
    for (i, row) in a.iter_mut().enumerate() {
        row[i] += lambda;
    }
    // Forward elimination with partial pivoting.
    for col in 0..d {
        let piv = (col..d).max_by(|&r1, &r2| {
            a[r1][col].abs().partial_cmp(&a[r2][col].abs()).unwrap()
        })?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for row in (col + 1)..d {
            let k = a[row][col] / a[col][col];
            for j in col..d {
                a[row][j] -= k * a[col][j];
            }
            b[row] -= k * b[col];
        }
    }
    // Back substitution.
    let mut w = vec![0f64; d];
    for i in (0..d).rev() {
        let mut s = b[i];
        for j in (i + 1)..d {
            s -= a[i][j] * w[j];
        }
        w[i] = s / a[i][i];
    }
    if w.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(w.into_iter().map(|v| v as f32).collect())
}

/// Fit both axes from (features, target) samples; returns the calibration
/// and the mean absolute residual (screen fractions) as a quality figure.
pub fn fit(samples: &[(Vec<f32>, f32, f32)]) -> Option<(GazeCalib, f32)> {
    let xs: Vec<Vec<f32>> = samples.iter().filter_map(|(f, _, _)| expand(f)).collect();
    if xs.len() < samples.len().min(20) || xs.len() < 20 {
        return None; // too many invalid frames or too few samples overall
    }
    let valid: Vec<&(Vec<f32>, f32, f32)> = samples
        .iter()
        .filter(|(f, _, _)| expand(f).is_some())
        .collect();
    let tx: Vec<f32> = valid.iter().map(|(_, x, _)| *x).collect();
    let ty: Vec<f32> = valid.iter().map(|(_, _, y)| *y).collect();
    const LAMBDA: f64 = 1e-4;
    let calib = GazeCalib {
        wx: ridge_fit(&xs, &tx, LAMBDA)?,
        wy: ridge_fit(&xs, &ty, LAMBDA)?,
    };
    let mut err = 0.0f32;
    for (f, x, y) in &*valid {
        let (px, py) = calib.predict(f)?;
        err += ((px - x).powi(2) + (py - y).powi(2)).sqrt();
    }
    Some((calib, err / valid.len() as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random features (no RNG dependency).
    fn feat(i: usize) -> Vec<f32> {
        let s = i as f32;
        (0..FEATS)
            .map(|j| ((s * 1.7 + j as f32 * 2.3).sin() * 0.3 + 0.5))
            .collect()
    }

    #[test]
    fn ridge_recovers_a_linear_mapping() {
        // Ground truth: y depends on hxR, yaw and a constant.
        let truth = |f: &[f32]| 0.2 + 0.9 * f[0] - 0.4 * f[4];
        let samples: Vec<(Vec<f32>, f32, f32)> = (0..120)
            .map(|i| {
                let f = feat(i);
                let t = truth(&f);
                (f, t, 1.0 - t)
            })
            .collect();
        let (calib, err) = fit(&samples).expect("fit succeeds");
        assert!(err < 0.01, "residual should be tiny on clean data: {err}");
        let f = feat(999);
        let (px, py) = calib.predict(&f).unwrap();
        assert!((px - truth(&f)).abs() < 0.02);
        assert!((py - (1.0 - truth(&f))).abs() < 0.02);
    }

    #[test]
    fn fit_refuses_starved_or_broken_input() {
        assert!(fit(&[]).is_none());
        let bad = vec![(vec![f32::NAN; FEATS], 0.5f32, 0.5f32); 40];
        assert!(fit(&bad).is_none());
        // A stale calibration (wrong width) predicts None, never panics.
        let calib = GazeCalib { wx: vec![0.0; 3], wy: vec![0.0; 3] };
        assert!(calib.predict(&feat(1)).is_none());
    }
}
