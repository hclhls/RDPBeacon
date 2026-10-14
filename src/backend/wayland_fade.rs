/// Returns the bytes of one little-endian ARGB8888 black pixel with the given opacity.
/// Premultiplied black always has zero colour channels, so only alpha varies.
pub fn premultiplied_black_pixel(alpha: u8) -> [u8; 4] {
    [0, 0, 0, alpha]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixel_is_all_zero() {
        assert_eq!(premultiplied_black_pixel(0), [0, 0, 0, 0]);
    }

    #[test]
    fn opaque_pixel_is_black_with_full_alpha() {
        assert_eq!(premultiplied_black_pixel(255), [0, 0, 0, 255]);
    }

    #[test]
    fn midpoint_keeps_colour_channels_zero() {
        assert_eq!(premultiplied_black_pixel(128), [0, 0, 0, 128]);
    }
}
