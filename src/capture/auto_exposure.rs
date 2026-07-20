use crate::camera::CaptureParams;

pub struct ExposureLimits {
    pub min_exposure_us: u64,
    pub max_exposure_us: u64,
    pub min_gain: f64,
    pub max_gain: f64,
}

/// Brightness within +-DEADBAND of the target counts as converged.
const DEADBAND: f64 = 8.0;
/// A clipped sensor hides the true scene brightness, so the measured ratio
/// badly understates the correction. Above/below these levels the frame is
/// treated as blown/crushed and corrected by a fixed large factor instead of
/// the (useless) measured ratio, so we escape the rail in a few steps.
const SAT_HI: f64 = 250.0;
const SAT_LO: f64 = 5.0;
const SAT_CUT: f64 = 0.1; // blown white: cut light to 10% per step
const BLACK_BOOST: f64 = 10.0; // crushed black: 10x per step
/// Widest single-step light change, so sensor noise can't fling the loop.
const MAX_RATIO: f64 = 32.0;
/// Per-step gain reduction while settling toward the floor (see `settle_gain`).
const GAIN_SETTLE: f64 = 0.8;

/// Brightness is close enough to the target — the loop can stop hunting.
pub fn converged(mean: f64, target: f64) -> bool {
    (mean - target).abs() <= DEADBAND
}

/// The frame is clipped (blown white or crushed black) and therefore not
/// worth keeping — the capture loop skips persisting these while it hunts.
pub fn is_clipped(mean: f64) -> bool {
    mean >= SAT_HI || mean <= SAT_LO
}

/// Once brightness is on target, walk gain down toward its floor whenever
/// exposure has room to grow and hold the light — lower gain means less noise.
/// One gentle step per call; the capture loop runs this at the full interval,
/// so it never disturbs live viewing. Exposure compensation assumes a linear
/// gain (true for the mock; an approximation for real cameras that the
/// brightness feedback corrects on the next frame). Gain rises are handled by
/// `next_params`' overflow path, never here.
fn settle_gain(cur: CaptureParams, lim: &ExposureLimits) -> CaptureParams {
    if cur.gain <= lim.min_gain {
        return cur; // already at the floor
    }
    let new_gain = (cur.gain * GAIN_SETTLE).max(lim.min_gain);
    let compensated = cur.exposure_us as f64 * (cur.gain / new_gain); // hold the light
    if compensated > lim.max_exposure_us as f64 {
        return cur; // no exposure headroom to absorb the drop — keep the gain
    }
    CaptureParams {
        exposure_us: (compensated.round() as u64).clamp(lim.min_exposure_us, lim.max_exposure_us),
        gain: new_gain,
    }
}

/// One auto-exposure step, proportional and exposure-primary (like
/// indi-allsky). Exposure is the big knob (microseconds by day to seconds by
/// night); gain only absorbs what exposure's range can't, so gain — and thus
/// noise — stays low by day and rises only when exposure is maxed at night.
/// A blown or crushed frame is punched by a fixed factor so the loop escapes
/// a clipped sensor in a handful of steps instead of crawling.
pub fn next_params(
    mean: f64,
    target: f64,
    cur: CaptureParams,
    lim: &ExposureLimits,
) -> CaptureParams {
    if converged(mean, target) {
        // Brightness is fine — spend the idle step lowering gain toward the
        // floor so a bright day isn't shot at a leftover-from-night high gain.
        return settle_gain(cur, lim);
    }

    // How much more (>1) or less (<1) light we want.
    let mut ratio = target / mean.max(1.0);
    if mean >= SAT_HI {
        ratio = ratio.min(SAT_CUT); // clipped white — we know we're at least this over
    } else if mean <= SAT_LO {
        ratio = ratio.max(BLACK_BOOST); // crushed black — at least this under
    }
    ratio = ratio.clamp(1.0 / MAX_RATIO, MAX_RATIO);

    let want_exposure = cur.exposure_us as f64 * ratio;
    let (min_e, max_e) = (lim.min_exposure_us as f64, lim.max_exposure_us as f64);
    let mut next = cur;
    if want_exposure > max_e {
        // Too dark even at the longest exposure: max out exposure, put the
        // leftover factor (>1) into gain.
        next.exposure_us = lim.max_exposure_us;
        next.gain = (cur.gain * (want_exposure / max_e)).clamp(lim.min_gain, lim.max_gain);
    } else if want_exposure < min_e {
        // Too bright even at the shortest exposure: floor exposure, put the
        // leftover factor (<1) into gain, lowering it.
        next.exposure_us = lim.min_exposure_us;
        next.gain = (cur.gain * (want_exposure / min_e)).clamp(lim.min_gain, lim.max_gain);
    } else {
        // Exposure alone covers it; leave gain where it is.
        next.exposure_us =
            (want_exposure.round() as u64).clamp(lim.min_exposure_us, lim.max_exposure_us);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::CaptureParams;

    const LIM: ExposureLimits = ExposureLimits {
        min_exposure_us: 32,
        max_exposure_us: 10_000_000,
        min_gain: 1.0,
        max_gain: 16.0,
    };

    #[test]
    fn inside_deadband_brightness_is_unchanged() {
        // At the gain floor there is nothing to settle, so an in-band frame
        // leaves the params untouched.
        let cur = CaptureParams {
            exposure_us: 1_000_000,
            gain: LIM.min_gain,
        };
        assert_eq!(next_params(104.0, 100.0, cur, &LIM), cur);
    }

    #[test]
    fn gain_settles_toward_min_when_exposure_has_headroom() {
        // Converged at max gain (as if left over from night) with exposure far
        // below its ceiling: gain must walk down to the floor, trading up into
        // exposure, so a bright day isn't shot at a noisy high gain.
        // Linear scene mean = k * exposure * gain held at the target.
        let k = 100.0 / (7_000.0 * 16.0);
        let mut cur = CaptureParams {
            exposure_us: 7_000,
            gain: 16.0,
        };
        for _ in 0..30 {
            let mean = (k * cur.exposure_us as f64 * cur.gain).min(255.0);
            cur = next_params(mean, 100.0, cur, &LIM);
        }
        assert_eq!(cur.gain, LIM.min_gain, "gain did not reach the floor");
        assert!(
            cur.exposure_us > 7_000,
            "exposure did not absorb the gain drop"
        );
        let mean = (k * cur.exposure_us as f64 * cur.gain).min(255.0);
        assert!((mean - 100.0).abs() <= 8.0, "brightness drifted: {mean}");
    }

    #[test]
    fn gain_holds_when_exposure_is_already_maxed() {
        // Night: exposure railed at max and gain high — there is no exposure
        // headroom to absorb a gain reduction, so gain must stay put.
        let cur = CaptureParams {
            exposure_us: LIM.max_exposure_us,
            gain: 12.0,
        };
        assert_eq!(next_params(100.0, 100.0, cur, &LIM), cur);
    }

    #[test]
    fn too_dark_raises_exposure_first_then_gain() {
        let cur = CaptureParams {
            exposure_us: 1_000_000,
            gain: 2.0,
        };
        let next = next_params(20.0, 100.0, cur, &LIM);
        assert!(next.exposure_us > cur.exposure_us);
        assert_eq!(next.gain, cur.gain); // exposure has room — gain untouched
        let at_max = CaptureParams {
            exposure_us: LIM.max_exposure_us,
            gain: 2.0,
        };
        let next2 = next_params(20.0, 100.0, at_max, &LIM);
        assert_eq!(next2.exposure_us, LIM.max_exposure_us);
        assert!(next2.gain > at_max.gain); // exposure maxed — overflow into gain
    }

    #[test]
    fn too_bright_lowers_exposure_first_then_gain() {
        // Exposure is the primary knob: a bright (but not clipped) frame drops
        // exposure and leaves gain alone.
        let cur = CaptureParams {
            exposure_us: 5_000_000,
            gain: 8.0,
        };
        let next = next_params(220.0, 100.0, cur, &LIM);
        assert!(next.exposure_us < cur.exposure_us);
        assert_eq!(next.gain, cur.gain);
        // Only once exposure is floored does gain come down.
        let at_min_exp = CaptureParams {
            exposure_us: LIM.min_exposure_us,
            gain: 8.0,
        };
        let next2 = next_params(220.0, 100.0, at_min_exp, &LIM);
        assert_eq!(next2.exposure_us, LIM.min_exposure_us);
        assert!(next2.gain < at_min_exp.gain);
    }

    #[test]
    fn escapes_daytime_saturation_to_sub_millisecond() {
        // Bright f/1.8 daylight: the scene truly needs ~tens of microseconds,
        // but a sensor clipped at 255 hides it. Start from a night-ish
        // 5 s / gain 8 and confirm it drives down to a sub-millisecond
        // exposure and hits the target within a handful of steps.
        let k = 100.0 / (50.0 * 1.0); // scene: mean == 100 at 50us, gain 1
        let mut cur = CaptureParams {
            exposure_us: 5_000_000,
            gain: 8.0,
        };
        for _ in 0..15 {
            let mean = (k * cur.exposure_us as f64 * cur.gain).min(255.0);
            cur = next_params(mean, 100.0, cur, &LIM);
        }
        let final_mean = (k * cur.exposure_us as f64 * cur.gain).min(255.0);
        assert!(
            (final_mean - 100.0).abs() <= DEADBAND,
            "did not converge: mean {final_mean}, exp {}us gain {}",
            cur.exposure_us,
            cur.gain
        );
        assert!(
            cur.exposure_us < 1_000,
            "daylight exposure should be sub-millisecond, got {}us",
            cur.exposure_us
        );
    }

    #[test]
    fn converges_on_a_linear_scene_within_a_few_steps() {
        // scene: mean = k * exposure_us * gain
        let k = 100.0 / (2_000_000.0 * 4.0);
        let mut cur = CaptureParams {
            exposure_us: 32,
            gain: 1.0,
        };
        for _ in 0..6 {
            let mean = (k * cur.exposure_us as f64 * cur.gain).min(255.0);
            cur = next_params(mean, 100.0, cur, &LIM);
        }
        let final_mean = (k * cur.exposure_us as f64 * cur.gain).min(255.0);
        assert!(
            (final_mean - 100.0).abs() <= DEADBAND,
            "final mean {final_mean}"
        );
    }

    #[test]
    fn never_leaves_the_limits() {
        let mut cur = CaptureParams {
            exposure_us: 32,
            gain: 1.0,
        };
        for mean in [0.0, 255.0, 0.0, 255.0, 1.0] {
            cur = next_params(mean, 100.0, cur, &LIM);
            assert!(
                cur.exposure_us >= LIM.min_exposure_us && cur.exposure_us <= LIM.max_exposure_us
            );
            assert!(cur.gain >= LIM.min_gain && cur.gain <= LIM.max_gain);
        }
    }

    #[test]
    fn converged_and_clipped_helpers() {
        assert!(converged(100.0, 100.0));
        assert!(converged(107.0, 100.0));
        assert!(!converged(120.0, 100.0));
        assert!(is_clipped(255.0));
        assert!(is_clipped(2.0));
        assert!(!is_clipped(100.0));
    }
}
