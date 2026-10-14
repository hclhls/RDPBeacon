use std::time::Duration;

#[allow(dead_code)]
pub const FADE_STEP: Duration = Duration::from_millis(16);

/// Calculates the screen opacity alpha (0 = fully transparent, 255 = fully opaque)
/// given elapsed time and total duration.
/// If `duration` is zero, immediately returns 255 (fully opaque).
/// If `elapsed >= duration`, returns 255.
#[allow(dead_code)]
pub fn fade_alpha(elapsed: Duration, duration: Duration) -> u8 {
    if duration.is_zero() || elapsed >= duration {
        return 255;
    }
    let elapsed_as_micros = elapsed.as_micros();
    let duration_as_micros = duration.as_micros();
    let ratio = (elapsed_as_micros as f64) / (duration_as_micros as f64);
    (ratio * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn alpha_is_zero_at_start() {
        assert_eq!(fade_alpha(Duration::ZERO, Duration::from_millis(300)), 0);
    }

    #[test]
    fn alpha_midpoint_is_about_half() {
        let alpha = fade_alpha(Duration::from_millis(150), Duration::from_millis(300));
        assert!(alpha == 127 || alpha == 128, "expected ~half alpha, got {alpha}");
    }

    #[test]
    fn alpha_saturates_after_duration() {
        assert_eq!(fade_alpha(Duration::from_millis(500), Duration::from_millis(300)), 255);
    }

    #[test]
    fn alpha_with_zero_duration_is_opaque() {
        assert_eq!(fade_alpha(Duration::ZERO, Duration::ZERO), 255);
        assert_eq!(fade_alpha(Duration::from_millis(100), Duration::ZERO), 255);
    }
}
