//! A body drawn as pictures: an HTML message laid out and painted as a browser would, placed
//! under the header in strips, a page's worth at a time, each strip ending where a row is one
//! colour all across (between lines of text, not through them) when there's one near the end.

use std::fmt::Write as _;

use super::{MARGIN, MAX_PAGES, PAGE_H, Placed, add, compress};

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

/// The drawn body placed from `y` down, a page's room at a time (each strip painted then, and
/// ended between lines where it can be), above `bottom`; `true` when it didn't all fit in
/// [`MAX_PAGES`].
pub(crate) fn place(d: Drawn<'_>, width: f32, bottom: f32, pages: &mut Vec<Vec<Placed>>, strips: &mut Vec<Vec<Strip>>, y: &mut f32) -> bool {
    if !(d.width.is_finite() && d.height.is_finite()) || d.width < 1.0 || d.height <= 0.0 {
        return false;
    }
    // Points per CSS pixel: the page's width across the text's.
    let k = width / d.width;
    let mut top = 0.0f32;
    while top < d.height - 0.5 {
        let room = *y - bottom;
        if room < 36.0 {
            if pages.len() >= MAX_PAGES {
                return true;
            }
            pages.push(Vec::new());
            *y = PAGE_H - MARGIN;
            continue;
        }
        let want = (room / k).min(d.height - top);
        let Some((px_w, px_h, rgba)) = (d.paint)(top, want) else { return false };
        if px_w == 0 || px_h == 0 {
            return false;
        }
        let last = top + want >= d.height - 0.5;
        let keep = if last { px_h } else { rows_to_keep(px_w, px_h, &rgba) }.max(1);
        let used = want * keep as f32 / px_h as f32;
        let h = used * k;
        *y -= h;
        strips.resize_with(pages.len(), Vec::new);
        if let Some(on) = strips.last_mut() {
            on.push(Strip { x: MARGIN, y: *y, w: width, h, px_w, px_h: keep, data: compress(&rgb_on_white(px_w, keep, &rgba)) });
        }
        top += used;
        if !last {
            *y = bottom; // the page is full: the next strip starts a new one
        }
    }
    false
}

/// A page's strips drawn (its content stream's start).
pub(crate) fn draw_strips(strips: Option<&Vec<Strip>>) -> String {
    let mut c = String::new();
    for (j, st) in strips.into_iter().flatten().enumerate() {
        let _ = writeln!(c, "q {:.2} 0 0 {:.2} {:.2} {:.2} cm /St{} Do Q", st.w, st.h, st.x, st.y, j + 1);
    }
    c
}

/// A page's strips as picture objects: their entries in the page's `/XObject` dictionary.
pub(crate) fn strip_objects(strips: Option<&mut Vec<Strip>>, objects: &mut Vec<Vec<u8>>) -> String {
    let mut names = String::new();
    for (j, st) in strips.into_iter().flatten().enumerate() {
        let data = std::mem::take(&mut st.data);
        let mut s = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            st.px_w,
            st.px_h,
            data.len()
        )
        .into_bytes();
        s.extend_from_slice(&data);
        s.extend_from_slice(b"\nendstream");
        let _ = write!(names, "/St{} {} 0 R ", j + 1, add(objects, s));
    }
    names
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
