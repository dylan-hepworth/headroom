// The tray icon on Windows. A Mac menu bar item has room for a title beside its icon, so usage there is text, rings,
// or both, side by side. A Windows tray icon is a small square with nothing beside it, so everything goes in the square:
// the rings one inside another, the percentage in the middle when there's one ring, and a dot in the corner for
// sessions waiting on the user. The tooltip has the rest in words.
#![cfg_attr(not(windows), allow(dead_code))]

/// The icon's size in pixels. Windows draws it at 16 pixels at 100% scaling and 32 at 200%, shrinking this to fit.
pub const SIZE: u32 = 32;

/// One ring: how full, from 0 to 100, and its color.
pub struct Ring {
    pub pct: f64,
    pub color: [u8; 4],
}

/// What goes in the icon.
pub struct Icon<'a> {
    /// Outermost first.
    pub rings: &'a [Ring],
    /// Shown in the middle: a percentage, or "!" when the last check failed.
    pub text: Option<String>,
    /// Corner dots, for sessions waiting: their colors, one dot for each.
    pub dots: &'a [[u8; 4]],
    /// Faded, while alerts and requests are paused.
    pub faded: bool,
    /// A light taskbar, which needs dark text.
    pub light: bool,
}

const TRACK: [u8; 4] = [142, 142, 147, 150];
const SAMPLES: u32 = 4;

/// The digits and the few marks the icon needs, 3 pixels wide and 5 tall, a row to a string.
fn glyph(c: char) -> Option<[&'static str; 5]> {
    Some(match c {
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => [".#.", "##.", ".#.", ".#.", "###"],
        '2' => ["###", "..#", "###", "#..", "###"],
        '3' => ["###", "..#", "###", "..#", "###"],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "###", "..#", "###"],
        '6' => ["###", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", ".#.", ".#.", ".#."],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "###"],
        '!' => [".#.", ".#.", ".#.", "...", ".#."],
        '…' => ["...", "...", "...", "...", "#.#"],
        _ => return None,
    })
}

/// Lay one color over another, by the top one's alpha.
fn over(under: [f64; 4], top: [f64; 4]) -> [f64; 4] {
    let a = top[3] + under[3] * (1.0 - top[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mix = |i: usize| (top[i] * top[3] + under[i] * under[3] * (1.0 - top[3])) / a;
    [mix(0), mix(1), mix(2), a]
}

fn unit([r, g, b, a]: [u8; 4]) -> [f64; 4] {
    [r as f64, g as f64, b as f64, a as f64 / 255.0]
}

/// Draw the icon as RGBA pixels, `SIZE` by `SIZE`.
pub fn draw(icon: &Icon) -> Vec<u8> {
    let size = SIZE as f64;
    let center = size / 2.0;
    let mut px = vec![[0.0f64; 4]; (SIZE * SIZE) as usize];

    // Each ring is a band; with more than one, they're thinner and nested with a gap between
    let n = icon.rings.len().max(1) as f64;
    let (outer, gap) = (15.5, 1.5);
    let band = if n == 1.0 { 3.5 } else { ((outer - 5.0) - gap * (n - 1.0)) / n };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut acc = [0.0f64; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let dx = x as f64 + (sx as f64 + 0.5) / SAMPLES as f64 - center;
                    let dy = y as f64 + (sy as f64 + 0.5) / SAMPLES as f64 - center;
                    let r = dx.hypot(dy);
                    let around = (dx.atan2(-dy) / std::f64::consts::TAU).rem_euclid(1.0);
                    let mut color = [0.0f64; 4];
                    for (i, ring) in icon.rings.iter().enumerate() {
                        let ring_outer = outer - i as f64 * (band + gap);
                        if (ring_outer - band..=ring_outer).contains(&r) {
                            let filled = ring.pct.clamp(0.0, 100.0) / 100.0;
                            color = unit(if around < filled { ring.color } else { TRACK });
                        }
                    }
                    for c in 0..4 {
                        acc[c] += color[c] * if c == 3 { 1.0 } else { color[3] };
                    }
                }
            }
            let total = (SAMPLES * SAMPLES) as f64;
            let a = acc[3] / total;
            px[(y * SIZE + x) as usize] =
                if a > 0.0 { [acc[0] / total / a, acc[1] / total / a, acc[2] / total / a, a] } else { [0.0; 4] };
        }
    }

    // The text in the middle, in 3x5 glyphs drawn at twice the size, a pixel apart; three characters are drawn smaller
    if let Some(text) = &icon.text {
        let glyphs: Vec<_> = text.chars().filter_map(glyph).collect();
        let scale: u32 = if glyphs.len() >= 3 { 1 } else { 2 };
        let width = glyphs.len() as u32 * (3 * scale + scale) - scale;
        let (left, top) = ((SIZE - width.min(SIZE)) / 2, (SIZE - 5 * scale) / 2);
        let ink = if icon.light { [32.0, 32.0, 32.0, 1.0] } else { [255.0, 255.0, 255.0, 1.0] };
        for (i, rows) in glyphs.iter().enumerate() {
            for (gy, row) in rows.iter().enumerate() {
                for (gx, c) in row.chars().enumerate() {
                    if c != '#' {
                        continue;
                    }
                    for oy in 0..scale {
                        for ox in 0..scale {
                            let x = left + i as u32 * 4 * scale + gx as u32 * scale + ox;
                            let y = top + gy as u32 * scale + oy;
                            if x < SIZE && y < SIZE {
                                px[(y * SIZE + x) as usize] = ink;
                            }
                        }
                    }
                }
            }
        }
    }

    // The dots, from the top right corner leftward, each with a ring cut out around it so it stands off what's under
    for (i, dot) in icon.dots.iter().enumerate() {
        let (cx, cy, radius) = (size - 5.5 - i as f64 * 9.0, 5.5, 4.5);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (mut cover, mut cut) = (0.0, 0.0);
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let dx = x as f64 + (sx as f64 + 0.5) / SAMPLES as f64 - cx;
                        let dy = y as f64 + (sy as f64 + 0.5) / SAMPLES as f64 - cy;
                        let r = dx.hypot(dy);
                        if r <= radius {
                            cover += 1.0;
                        } else if r <= radius + 1.5 {
                            cut += 1.0;
                        }
                    }
                }
                let total = (SAMPLES * SAMPLES) as f64;
                let p = &mut px[(y * SIZE + x) as usize];
                p[3] *= 1.0 - cut / total;
                *p = over(*p, [dot[0] as f64, dot[1] as f64, dot[2] as f64, cover / total]);
            }
        }
    }

    let fade = if icon.faded { 0.45 } else { 1.0 };
    px.iter()
        .flat_map(|[r, g, b, a]| {
            [r.round() as u8, g.round() as u8, b.round() as u8, (a * fade * 255.0).round().min(255.0) as u8]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_square_of_the_right_size() {
        let rings = [Ring { pct: 40.0, color: [217, 119, 87, 255] }];
        let icon = Icon { rings: &rings, text: Some("40".into()), dots: &[[242, 181, 28, 255]], faded: false, light: false };
        let px = draw(&icon);
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        // The middle of the "4" is white, and a corner far from everything is clear
        assert!(px.chunks(4).any(|p| p == [255, 255, 255, 255]));
        assert_eq!(&px[((SIZE - 1) * SIZE * 4) as usize..][..4], &[0, 0, 0, 0]);
    }
}
