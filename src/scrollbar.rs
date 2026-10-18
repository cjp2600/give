//! Giv's overview scale: round(line / total * (height - 1)).
use crate::git::{DiffLine, LineKind};

pub fn marker_position(line: usize, total: usize, height: f32) -> f32 {
    (line as f32 / total.max(1) as f32 * (height - 1.).max(0.))
        .round()
        .clamp(0., (height - 1.).max(0.))
}

pub fn changed_lines(lines: &[DiffLine]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l.kind, LineKind::Added | LineKind::Removed))
        .map(|(i, _)| i)
        .collect()
}

pub fn nearest_change(changes: &[usize], total: usize, height: f32, y: f32) -> Option<usize> {
    changes
        .iter()
        .copied()
        .min_by(|a, b| {
            (marker_position(*a, total, height) - y)
                .abs()
                .total_cmp(&(marker_position(*b, total, height) - y).abs())
        })
        .filter(|line| (marker_position(*line, total, height) - y).abs() <= 5.)
}

/// Thumb size is viewport/total; its position uses the available thumb travel.
pub fn thumb(total: usize, visible: f32, offset: f32, height: f32) -> Option<(f32, f32)> {
    thumb_with_minimum(total, visible, offset, height, 1.)
}

pub fn thumb_with_minimum(
    total: usize,
    visible: f32,
    offset: f32,
    height: f32,
    minimum: f32,
) -> Option<(f32, f32)> {
    if total as f32 <= visible || height <= 0. {
        return None;
    }
    let size = (visible / total as f32 * height)
        .round()
        .clamp(minimum.min(height).max(1.), height);
    let percent = (offset / (total as f32 - visible)).clamp(0., 1.);
    Some((percent * (height - size), size))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions_and_thumb_match_giv_scale() {
        assert_eq!(marker_position(0, 100, 20.), 0.);
        assert_eq!(marker_position(50, 100, 20.), 10.);
        assert_eq!(marker_position(99, 100, 20.), 19.);
        assert_eq!(marker_position(200, 100, 20.), 19.);
        assert_eq!(thumb(100, 20., 40., 20.), Some((8., 4.)));
        assert_eq!(thumb(100, 20., 80., 20.), Some((16., 4.)));
        assert_eq!(thumb(10, 20., 0., 20.), None);
        assert_eq!(nearest_change(&[10, 50, 90], 100, 200., 99.), Some(50));
        assert_eq!(nearest_change(&[10, 90], 100, 200., 99.), None);
        assert_eq!(nearest_change(&[], 0, 0., 0.), None);
    }
}
