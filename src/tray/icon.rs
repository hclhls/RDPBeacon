pub fn icon_rgba() -> (Vec<u8>, u32, u32) {
    let size = 32;
    let mut rgba = vec![0; (size * size * 4) as usize];
    for y in 3..29 {
        for x in 3..29 {
            let index = ((y * size + x) * 4) as usize;
            let border = x == 3 || x == 28 || y == 3 || y == 24;
            let stand = y > 24 && (13..19).contains(&x);
            let pulse = (7..25).contains(&x) && y == 14 || x == 15 && (9..20).contains(&y);
            let color = if border || stand {
                [40, 170, 220, 255]
            } else if pulse && y < 24 {
                [240, 250, 255, 255]
            } else if y < 24 {
                [20, 40, 60, 255]
            } else {
                [0, 0, 0, 0]
            };
            rgba[index..index + 4].copy_from_slice(&color);
        }
    }
    (rgba, size, size)
}
