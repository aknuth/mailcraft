//! A body drawn as pictures: an HTML message laid out and painted as a browser would, placed
//! under the header in strips, a page's worth at a time, each strip ending where a row is one
//! colour all across (between lines of text, not through them) when there's one near the end.

/// A body drawn as pictures, in place of the text.
pub struct Drawn<'a> {
    /// Its size in CSS pixels.
    pub width: f32,
    pub height: f32,
    /// Paints the part from `top`, `height` CSS pixels tall: its size in pixels and its RGBA
    /// pixels (premultiplied), or `None` when it can't.
    pub paint: &'a mut dyn FnMut(f32, f32) -> Option<(u32, u32, Vec<u8>)>,
}

/// A strip placed on a page: where (points, from the bottom left), its pixel size, and its
/// pixels as compressed RGB.
pub(crate) struct Strip {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub px_w: u32,
    pub px_h: u32,
    pub data: Vec<u8>,
}

/// How far up from a strip's end a break between lines is looked for (a share of its height).
const LOOK_BACK: f32 = 0.25;

/// The rows of a strip to keep on its page: up to the last row in its last quarter that's one
/// colour all across, else all of them.
pub(crate) fn rows_to_keep(w: u32, h: u32, rgba: &[u8]) -> u32 {
    let row_len = (w as usize).saturating_mul(4);
    if row_len == 0 || h < 8 {
        return h;
    }
    let lowest = ((h as f32) * (1.0 - LOOK_BACK)) as u32;
    for r in (lowest.max(1)..h).rev() {
        let start = (r as usize).saturating_mul(row_len);
        let Some(row) = rgba.get(start..start.saturating_add(row_len)) else { continue };
        let (pixels, _) = row.as_chunks::<4>();
        let Some(first) = pixels.first() else { continue };
        if pixels.iter().all(|p| p == first) {
            return r;
        }
    }
    h
}

/// The first `rows` rows of premultiplied RGBA as RGB on white paper.
pub(crate) fn rgb_on_white(w: u32, rows: u32, rgba: &[u8]) -> Vec<u8> {
    let n = (w as usize).saturating_mul(rows as usize);
    let mut out = Vec::with_capacity(n.saturating_mul(3));
    for p in rgba.as_chunks::<4>().0.iter().take(n) {
        let paper = 255 - p[3];
        out.extend_from_slice(&[p[0].saturating_add(paper), p[1].saturating_add(paper), p[2].saturating_add(paper)]);
    }
    // Pixels missing (a short buffer): white.
    out.resize(n.saturating_mul(3), 255);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32, ink_rows: &[u32]) -> Vec<u8> {
        let mut px = vec![255u8; (w * h * 4) as usize];
        for r in ink_rows {
            let at = (r * w * 4) as usize;
            px[at..at + 4].copy_from_slice(&[0, 0, 0, 255]);
        }
        px
    }

    #[test]
    fn a_strip_ends_between_lines() {
        // Ink on rows 0..=94 except 90: the break is at the blank row 90.
        let ink: Vec<u32> = (0..95).filter(|r| *r != 90).collect();
        assert_eq!(rows_to_keep(10, 100, &picture(10, 100, &ink)), 99);
        let all: Vec<u32> = (0..100).filter(|r| *r != 90).collect();
        assert_eq!(rows_to_keep(10, 100, &picture(10, 100, &all)), 90);
        // No blank row in the last quarter: the whole strip.
        let dense: Vec<u32> = (0..100).collect();
        assert_eq!(rows_to_keep(10, 100, &picture(10, 100, &dense)), 100);
        // Nothing to look at.
        assert_eq!(rows_to_keep(0, 100, &[]), 100);
        assert_eq!(rows_to_keep(10, 100, &[]), 100);
    }

    #[test]
    fn see_through_pixels_are_paper() {
        assert_eq!(rgb_on_white(2, 1, &[0, 0, 0, 0, 10, 20, 30, 255]), [255, 255, 255, 10, 20, 30]);
        assert_eq!(rgb_on_white(1, 1, &[100, 0, 0, 128]), [227, 127, 127]);
        assert_eq!(rgb_on_white(2, 1, &[]), [255; 6]);
    }
}
