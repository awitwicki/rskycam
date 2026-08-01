use crate::camera::CaptureParams;

#[derive(Clone, Copy, Debug)]
pub struct ExposureLimits {
    pub min_exposure_us: u64,
    pub max_exposure_us: u64,
    pub min_gain: f64,
    pub max_gain: f64,
}

/// Brightness within +-DEADBAND of the target counts as converged. Wide
/// enough to absorb real frame-to-frame brightness variance (wind-blown
/// scenery, drifting cloud, sensor noise) without endlessly re-correcting.
const DEADBAND: f64 = 15.0;
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
/// Fraction of the *non-clipped* correction applied per step. A single
/// frame's measured mean is noisy (real scenery, sensor read noise) — a
/// full-strength proportional step overshoots the target on every
/// correction and ping-pongs between two exposures forever instead of
/// settling. Only applied outside the SAT_HI/SAT_LO escape path, which stays
/// at full strength so an actually clipped frame still escapes in a handful
/// of steps.
const DAMPING: f64 = 0.35;

/// Daytime starting exposure for a fresh hunt: short enough that even harsh
/// sunlight is at worst a couple of SAT_CUT steps over, and close enough to
/// a typical daylight exposure that an overcast sky converges upward in a
/// few damped steps.
const DAY_SEED_EXPOSURE_US: u64 = 1_000;

/// Starting point for the auto-exposure hunt when no previous frame exists
/// (service start). The manual settings are the user's night baseline —
/// seeding a daytime restart from them (e.g. 20 s at gain 100) parades a
/// minute of blown, high-gain frames through the dashboard while the loop
/// walks all the way down. By day, start short and at the gain floor
/// instead: gain — and thus noise — never has to descend at all.
pub fn initial_params(night: bool, manual: CaptureParams, lim: &ExposureLimits) -> CaptureParams {
    let (exposure_us, gain) = if night {
        (manual.exposure_us, manual.gain)
    } else {
        (DAY_SEED_EXPOSURE_US, lim.min_gain)
    };
    CaptureParams {
        exposure_us: exposure_us.clamp(lim.min_exposure_us, lim.max_exposure_us),
        gain: gain.clamp(lim.min_gain, lim.max_gain),
    }
}

/// Brightness is close enough to the target — the loop can stop hunting.
pub fn converged(mean: f64, target: f64) -> bool {
    (mean - target).abs() <= DEADBAND
}

/// Why a metered frame is worth persisting, if it is: brightness on target,
/// OR the controller is railed and the next step wouldn't change anything —
/// a moonless sky below the deadband at max exposure/gain is the best frame
/// the camera can produce, and dropping it loses the whole night. `None`
/// means the frame is mid-hunt (the next step still improves things) and
/// should be dropped. The string goes verbatim into the per-frame log line.
pub fn persist_reason(
    mean: f64,
    target: f64,
    taken: CaptureParams,
    lim: &ExposureLimits,
) -> Option<&'static str> {
    if converged(mean, target) {
        Some("on target")
    } else if next_params(mean, target, taken, lim) == taken {
        Some("railed at limits")
    } else {
        None
    }
}

/// Once brightness is on target, walk gain down toward its floor whenever
/// exposure has room to grow and hold the light — lower gain means less
/// noise. Drops gain as far as the current exposure headroom affords in one
/// step (down to the floor outright whenever there's enough room, e.g. a few
/// ms of exposure by day against a multi-second ceiling) rather than a fixed
/// fraction per call — the capture loop runs this at the full interval, so a
/// timid fixed step could take dozens of intervals to reach the floor.
/// Exposure compensation assumes a linear gain (true for the mock; an
/// approximation for real cameras that the brightness feedback corrects on
/// the next frame). Gain rises are handled by `next_params`' overflow path,
/// never here.
fn settle_gain(cur: CaptureParams, lim: &ExposureLimits) -> CaptureParams {
    if cur.gain <= lim.min_gain {
        return cur; // already at the floor
    }
    // The lowest gain the exposure ceiling can afford while holding the same
    // total light (exposure_us * gain constant): compensated = cur.exposure_us
    // * (cur.gain / new_gain) <= max_exposure_us solved for new_gain.
    let min_affordable_gain = (cur.exposure_us as f64 * cur.gain) / lim.max_exposure_us as f64;
    let new_gain = min_affordable_gain.max(lim.min_gain);
    if new_gain >= cur.gain {
        return cur; // no exposure headroom at all to absorb any drop
    }
    let compensated = cur.exposure_us as f64 * (cur.gain / new_gain);
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
    } else {
        // Not clipped — damp the correction so one noisy/variable reading
        // doesn't overshoot the target and bounce back and forth forever.
        ratio = 1.0 + (ratio - 1.0) * DAMPING;
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
    fn railed_frames_persist_mid_hunt_frames_do_not() {
        // A moonless sky can sit below the deadband even at max exposure and
        // max gain. The controller can do no better — that railed frame IS
        // the night and must be persisted, or a whole dark night saves
        // almost nothing (real incident: 28 frames instead of ~225).
        let railed_dark = CaptureParams {
            exposure_us: LIM.max_exposure_us,
            gain: LIM.max_gain,
        };
        assert_eq!(
            persist_reason(40.0, 100.0, railed_dark, &LIM),
            Some("railed at limits")
        );

        // Too bright even at the floor: also railed, also worth keeping.
        let railed_bright = CaptureParams {
            exposure_us: LIM.min_exposure_us,
            gain: LIM.min_gain,
        };
        assert_eq!(
            persist_reason(240.0, 100.0, railed_bright, &LIM),
            Some("railed at limits")
        );

        // Same darkness mid-hunt (exposure still has headroom): drop it —
        // the next step will meaningfully improve the frame.
        let hunting = CaptureParams {
            exposure_us: 1_000_000,
            gain: 2.0,
        };
        assert_eq!(persist_reason(40.0, 100.0, hunting, &LIM), None);

        // On-target frames persist regardless of headroom.
        assert_eq!(
            persist_reason(104.0, 100.0, hunting, &LIM),
            Some("on target")
        );
    }

    #[test]
    fn day_seed_starts_short_at_the_gain_floor_night_seed_uses_manual() {
        // A daytime (re)start must NOT begin the hunt from the manual
        // settings — those are the user's night baseline (e.g. 20 s at gain
        // 100), and walking down from there parades a minute of blown,
        // high-gain frames through the dashboard. Day starts short and at
        // the gain floor; night keeps the manual baseline.
        let manual = CaptureParams {
            exposure_us: 20_000_000,
            gain: 100.0,
        };
        let day = initial_params(false, manual, &LIM);
        assert_eq!(day.gain, LIM.min_gain);
        assert!(
            day.exposure_us <= 10_000,
            "day seed should be a short exposure, got {}us",
            day.exposure_us
        );
        let night = initial_params(true, manual, &LIM);
        assert_eq!(night.exposure_us, LIM.max_exposure_us); // manual clamped in
        assert_eq!(night.gain, LIM.max_gain);
    }

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
        // scene: mean = k * exposure_us * gain. 12 steps, not 6: DAMPING
        // trades some convergence speed for immunity to noisy-measurement
        // oscillation (see damping_settles_into_a_tight_band_...) — still a
        // handful of steps, not a slow crawl.
        let k = 100.0 / (2_000_000.0 * 4.0);
        let mut cur = CaptureParams {
            exposure_us: 32,
            gain: 1.0,
        };
        for _ in 0..12 {
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
    fn damping_settles_into_a_tight_band_despite_noisy_measurements() {
        // Regression test for a real-hardware bug: on the ASI120MM Mini the
        // measured mean carries real frame-to-frame variance (moving
        // scenery, sensor noise) on top of what exposure/gain predict. The
        // undamped correction reacted to each noisy sample at full strength
        // and ping-ponged exposure between two values roughly 30% apart,
        // forever (never inside DEADBAND, so never settling onto the full
        // capture interval). Model the same closed loop (mean depends on
        // exposure/gain) plus a persistent +-20 measurement offset that
        // exceeds DEADBAND, and confirm exposure settles into a tight band
        // instead of continuing to swing widely.
        let k = 100.0 / (5_000.0 * 16.0);
        let mut cur = CaptureParams {
            exposure_us: 5_000,
            gain: 16.0,
        };
        let mut exposures = Vec::new();
        for i in 0..40 {
            let true_mean = (k * cur.exposure_us as f64 * cur.gain).min(255.0);
            let noisy_mean = (true_mean + if i % 2 == 0 { 20.0 } else { -20.0 }).clamp(0.0, 255.0);
            cur = next_params(noisy_mean, 100.0, cur, &LIM);
            exposures.push(cur.exposure_us as f64);
        }
        let last = &exposures[exposures.len() - 6..];
        let min = last.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = last.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(max / min < 1.15, "still oscillating widely: {last:?}");
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
    fn converged_helper() {
        assert!(converged(100.0, 100.0));
        assert!(converged(107.0, 100.0));
        assert!(!converged(120.0, 100.0));
    }
}
