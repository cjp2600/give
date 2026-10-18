//! Box-drawing (U+2500–U+257F) and block elements (U+2580–U+259F) as cell
//! rectangles. Font glyphs stop short of the row height, which leaves gaps in
//! TUI frames; like Alacritty and Ghostty, these characters fill the cell.

/// A rectangle in cell coordinates; `alpha` is the coverage of shade characters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub alpha: f32,
}

// Arm weights; 1 is light.
const NONE: u8 = 0;
const HEAVY: u8 = 2;
const DOUBLE: u8 = 3;

/// Arms as `0xURDL`: up, right, down, left, each none/light/heavy/double.
/// Dashed lines use their solid arms here; `dashes` splits them. `FONT` leaves
/// the glyph to the font.
const FONT: u16 = 0xffff;

#[rustfmt::skip]
const BOX: [u16; 128] = [
    // 2500 ─ ━ │ ┃ ┄ ┅ ┆ ┇ ┈ ┉ ┊ ┋ ┌ ┍ ┎ ┏
    0x0101, 0x0202, 0x1010, 0x2020, 0x0101, 0x0202, 0x1010, 0x2020,
    0x0101, 0x0202, 0x1010, 0x2020, 0x0110, 0x0210, 0x0120, 0x0220,
    // 2510 ┐ ┑ ┒ ┓ └ ┕ ┖ ┗ ┘ ┙ ┚ ┛ ├ ┝ ┞ ┟
    0x0011, 0x0012, 0x0021, 0x0022, 0x1100, 0x1200, 0x2100, 0x2200,
    0x1001, 0x1002, 0x2001, 0x2002, 0x1110, 0x1210, 0x2110, 0x1120,
    // 2520 ┠ ┡ ┢ ┣ ┤ ┥ ┦ ┧ ┨ ┩ ┪ ┫ ┬ ┭ ┮ ┯
    0x2120, 0x2210, 0x1220, 0x2220, 0x1011, 0x1012, 0x2011, 0x1021,
    0x2021, 0x2012, 0x1022, 0x2022, 0x0111, 0x0112, 0x0211, 0x0212,
    // 2530 ┰ ┱ ┲ ┳ ┴ ┵ ┶ ┷ ┸ ┹ ┺ ┻ ┼ ┽ ┾ ┿
    0x0121, 0x0122, 0x0221, 0x0222, 0x1101, 0x1102, 0x1201, 0x1202,
    0x2101, 0x2102, 0x2201, 0x2202, 0x1111, 0x1112, 0x1211, 0x1212,
    // 2540 ╀ ╁ ╂ ╃ ╄ ╅ ╆ ╇ ╈ ╉ ╊ ╋ ╌ ╍ ╎ ╏
    0x2111, 0x1121, 0x2121, 0x2112, 0x2211, 0x1122, 0x1221, 0x2212,
    0x1222, 0x2122, 0x2221, 0x2222, 0x0101, 0x0202, 0x1010, 0x2020,
    // 2550 ═ ║ ╒ ╓ ╔ ╕ ╖ ╗ ╘ ╙ ╚ ╛ ╜ ╝ ╞ ╟
    0x0303, 0x3030, 0x0310, 0x0130, 0x0330, 0x0013, 0x0031, 0x0033,
    0x1300, 0x3100, 0x3300, 0x1003, 0x3001, 0x3003, 0x1310, 0x3130,
    // 2560 ╠ ╡ ╢ ╣ ╤ ╥ ╦ ╧ ╨ ╩ ╪ ╫ ╬ ╭ ╮ ╯
    0x3330, 0x1013, 0x3031, 0x3033, 0x0313, 0x0131, 0x0333, 0x1303,
    0x3101, 0x3303, 0x1313, 0x3131, 0x3333, 0x0110, 0x0011, 0x1001,
    // 2570 ╰ ╱ ╲ ╳ ╴ ╵ ╶ ╷ ╸ ╹ ╺ ╻ ╼ ╽ ╾ ╿
    0x1100, FONT, FONT, FONT, 0x0001, 0x1000, 0x0100, 0x0010,
    0x0002, 0x2000, 0x0200, 0x0020, 0x0201, 0x1020, 0x0102, 0x2010,
];

/// Rectangles covering `c` in a `width`×`height` cell, or `None` for the font.
/// `light` is the light stroke width; heavy strokes are twice as wide.
pub fn rects(c: char, width: f32, height: f32, light: f32) -> Option<Vec<Rect>> {
    let code = c as u32;
    match code {
        0x2500..=0x257f => box_drawing(
            BOX[(code - 0x2500) as usize],
            dashes(code),
            width,
            height,
            light,
        ),
        0x2580..=0x259f => Some(block(code, width, height)),
        _ => None,
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
        alpha: 1.,
    }
}

/// ┄┅┆┇ have three dashes, ┈┉┊┋ four and ╌╍╎╏ two.
fn dashes(code: u32) -> usize {
    match code {
        0x2504..=0x2507 => 3,
        0x2508..=0x250b => 4,
        0x254c..=0x254f => 2,
        _ => 0,
    }
}

fn box_drawing(
    entry: u16,
    dashes: usize,
    width: f32,
    height: f32,
    light: f32,
) -> Option<Vec<Rect>> {
    if entry == FONT {
        return None;
    }
    let arm = |shift: u16| ((entry >> shift) & 0xf) as u8;
    let (up, right, down, left) = (arm(12), arm(8), arm(4), arm(0));
    let stroke = |weight: u8| if weight == HEAVY { light * 2. } else { light };
    let (cx, cy) = (width / 2., height / 2.);
    let mut out = Vec::new();
    if dashes > 0 {
        // Dashed entries keep only their axis; the dash pattern repeats per cell.
        let horizontal = right != NONE;
        let t = stroke(if horizontal { right } else { up });
        let length = if horizontal { width } else { height };
        let segment = length / dashes as f32;
        for i in 0..dashes {
            let start = i as f32 * segment + segment * 0.15;
            let size = segment * 0.7;
            out.push(if horizontal {
                rect(start, cy - t / 2., size, t)
            } else {
                rect(cx - t / 2., start, t, size)
            });
        }
        return Some(out);
    }

    // Double lines are two light strokes offset from the centre line.
    let gap = light.max(1.) * 1.5;
    // How far an arm reaches past the centre to meet the perpendicular strokes.
    let reach = |perpendicular: u8| match perpendicular {
        NONE => 0.,
        DOUBLE => gap + light / 2.,
        weight => stroke(weight) / 2.,
    };
    let reach_x = reach(up.max(down));
    let reach_y = reach(left.max(right));
    for (weight, direction, horizontal) in [
        (left, -1., true),
        (right, 1., true),
        (up, -1., false),
        (down, 1., false),
    ] {
        if weight == NONE {
            continue;
        }
        let (center, extent, reach, before, after) = if horizontal {
            (cx, width, reach_x, up, down)
        } else {
            (cy, height, reach_y, left, right)
        };
        // A stroke from `boundary` to the cell edge in `direction`.
        let mut push = |boundary: f32, offset: f32, thickness: f32| {
            let (start, size) = if direction > 0. {
                (boundary, extent - boundary)
            } else {
                (0., boundary)
            };
            out.push(if horizontal {
                rect(start, cy + offset - thickness / 2., size, thickness)
            } else {
                rect(cx + offset - thickness / 2., start, thickness, size)
            });
        };
        if weight != DOUBLE {
            push(center - direction * reach, 0., stroke(weight));
            continue;
        }
        let doubled = before.max(after) == DOUBLE;
        for (side, side_arm) in [(-1., before), (1., after)] {
            // Next to a perpendicular double arm, the inner line stops at its
            // near stroke and the outer line runs to its far stroke.
            let boundary = if doubled {
                center + direction * if side_arm != NONE { gap } else { -gap }
            } else {
                center - direction * reach
            };
            push(boundary, side * gap, light);
        }
    }
    Some(out)
}

fn block(code: u32, width: f32, height: f32) -> Vec<Rect> {
    let (w, h) = (width, height);
    let eighth = |n: f32| n / 8.;
    let quadrant = |upper: bool, left: bool| {
        rect(
            if left { 0. } else { w / 2. },
            if upper { 0. } else { h / 2. },
            w / 2.,
            h / 2.,
        )
    };
    let shade = |alpha: f32| Rect {
        alpha,
        ..rect(0., 0., w, h)
    };
    match code {
        0x2580 => vec![rect(0., 0., w, h / 2.)],
        // ▁ … █: lower n/8.
        0x2581..=0x2588 => {
            let part = h * eighth((code - 0x2580) as f32);
            vec![rect(0., h - part, w, part)]
        }
        // ▉ … ▏: left 7/8 … 1/8.
        0x2589..=0x258f => vec![rect(0., 0., w * eighth((0x2590 - code) as f32), h)],
        0x2590 => vec![rect(w / 2., 0., w / 2., h)],
        0x2591 => vec![shade(0.25)],
        0x2592 => vec![shade(0.5)],
        0x2593 => vec![shade(0.75)],
        0x2594 => vec![rect(0., 0., w, h / 8.)],
        0x2595 => vec![rect(w - w / 8., 0., w / 8., h)],
        0x2596 => vec![quadrant(false, true)],
        0x2597 => vec![quadrant(false, false)],
        0x2598 => vec![quadrant(true, true)],
        0x2599 => vec![
            quadrant(true, true),
            quadrant(false, true),
            quadrant(false, false),
        ],
        0x259a => vec![quadrant(true, true), quadrant(false, false)],
        0x259b => vec![
            quadrant(true, true),
            quadrant(true, false),
            quadrant(false, true),
        ],
        0x259c => vec![
            quadrant(true, true),
            quadrant(true, false),
            quadrant(false, false),
        ],
        0x259d => vec![quadrant(true, false)],
        0x259e => vec![quadrant(true, false), quadrant(false, true)],
        _ => vec![
            quadrant(true, false),
            quadrant(false, true),
            quadrant(false, false),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers(rects: &[Rect], x: f32, y: f32) -> bool {
        rects
            .iter()
            .any(|r| x >= r.x && x <= r.x + r.width && y >= r.y && y <= r.y + r.height)
    }

    #[test]
    fn lines_reach_every_cell_edge_they_connect() {
        let (w, h) = (8.4, 17.);
        let vertical = rects('│', w, h, 1.).unwrap();
        assert!(covers(&vertical, w / 2., 0.) && covers(&vertical, w / 2., h));
        let horizontal = rects('─', w, h, 1.).unwrap();
        assert!(covers(&horizontal, 0., h / 2.) && covers(&horizontal, w, h / 2.));
        let corner = rects('╭', w, h, 1.).unwrap();
        assert!(covers(&corner, w, h / 2.) && covers(&corner, w / 2., h));
        assert!(!covers(&corner, 0., h / 2.) && !covers(&corner, w / 2., 0.));
        let cross = rects('┼', w, h, 1.).unwrap();
        for (x, y) in [(0., h / 2.), (w, h / 2.), (w / 2., 0.), (w / 2., h)] {
            assert!(covers(&cross, x, y));
        }
    }

    #[test]
    fn heavy_and_double_lines_are_wider_than_light_ones() {
        let light = rects('─', 8., 16., 1.).unwrap();
        let heavy = rects('━', 8., 16., 1.).unwrap();
        assert_eq!(light[0].height * 2., heavy[0].height);
        let double = rects('═', 8., 16., 1.).unwrap();
        let rows: Vec<_> = double.iter().map(|r| r.y).collect();
        assert!(rows.iter().any(|y| *y < 7.) && rows.iter().any(|y| *y > 8.));
        // The inside of a double corner stays open.
        let corner = rects('╔', 8., 16., 1.).unwrap();
        assert!(covers(&corner, 8., 6.5) && covers(&corner, 2.5, 16.));
        assert!(covers(&corner, 2.5, 6.5), "outer corner joins");
        assert!(!covers(&corner, 4., 8.5) && !covers(&corner, 4., 12.));
    }

    #[test]
    fn blocks_fill_their_fraction_of_the_cell() {
        assert_eq!(rects('█', 8., 16., 1.).unwrap(), vec![rect(0., 0., 8., 16.)]);
        assert_eq!(rects('▀', 8., 16., 1.).unwrap(), vec![rect(0., 0., 8., 8.)]);
        assert_eq!(rects('▄', 8., 16., 1.).unwrap(), vec![rect(0., 8., 8., 8.)]);
        assert_eq!(rects('▏', 8., 16., 1.).unwrap(), vec![rect(0., 0., 1., 16.)]);
        assert_eq!(rects('▒', 8., 16., 1.).unwrap()[0].alpha, 0.5);
        assert_eq!(rects('▟', 8., 16., 1.).unwrap().len(), 3);
        let dashed = rects('┄', 9., 16., 1.).unwrap();
        assert_eq!(dashed.len(), 3);
    }

    #[test]
    fn other_characters_are_left_to_the_font() {
        for c in ['a', '╱', '╳', '⏺', ' '] {
            assert_eq!(rects(c, 8., 16., 1.), None);
        }
    }
}
