//! PROTOTYPE: an HTML message laid out and drawn by Blitz ([`mailcraft_html`]), shown as tiles
//! of pixels, its links clickable. On with `MAILCRAFT_BLITZ=1`; a page Blitz refuses (nested
//! too deep, a panic) is drawn the usual way.
//!
//! Not done yet: selecting text, the reading lane (it lays out on the window's thread), dark
//! mode (mail is drawn on white, as most mail expects).

use super::*;
use mailcraft_html::{Page, Refused};
use std::collections::HashMap;

/// Tiles are this many CSS pixels tall.
const TILE: f32 = 512.0;
/// New tiles painted in one frame, at most (the rest in the next ones).
const TILES_PER_FRAME: usize = 2;

/// The open message's page and its painted tiles.
pub(crate) struct HtmlView {
    key: MessageKey,
    page: Result<Page, Refused>,
    tiles: HashMap<usize, egui::TextureHandle>,
    /// Where it was drawn last, and the link under the pointer then.
    rect: Option<egui::Rect>,
    hovered: Option<String>,
}

#[cfg(test)]
impl HtmlView {
    /// Whether Blitz laid it out (and it's drawn as tiles).
    pub(crate) fn drawn(&self) -> bool {
        self.page.is_ok() && !self.tiles.is_empty()
    }

    pub(crate) fn rect(&self) -> Option<egui::Rect> {
        self.rect
    }

    pub(crate) fn hovered_link(&self) -> Option<String> {
        self.hovered.clone()
    }
}

/// Whether the prototype is on (`MAILCRAFT_BLITZ` set).
pub(crate) fn wanted() -> bool {
    std::env::var_os("MAILCRAFT_BLITZ").is_some()
}

/// Draw `html` with Blitz; `false` when it can't be (drawn the usual way then).
pub(super) fn draw(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, html: &str, acts: &mut Vec<Action>) -> bool {
    let width = ui.available_width().floor().max(1.0) as u32;
    let scale = ui.ctx().pixels_per_point();
    if st.html_view.as_ref().is_none_or(|v| v.key != *key) {
        let started = web_time::Instant::now();
        let page = Page::new(html, width, scale, false);
        log::info!("blitz: laid out in {:?}: {:?}", started.elapsed(), page.as_ref().map(Page::height).map_err(|e| e.to_string()));
        st.html_view = Some(HtmlView { key: key.clone(), page, tiles: HashMap::new(), rect: None, hovered: None });
    }
    let Some(view) = st.html_view.as_mut() else { return false };
    let Ok(page) = view.page.as_mut() else { return false };
    let mut changed = matches!(page.set_size(width, scale), Ok(true));
    // What it asks for: pictures sent with it, and web pictures once allowed.
    let allowed = s.pictures_allowed();
    for url in page.wanted() {
        let lower = url.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            if !allowed {
                continue; // asked again once the user allows them
            }
            match s.pictures.get(&url) {
                Some(mailcraft_engine::Picture::Ready(bytes)) => {
                    changed |= page.give(&url, bytes.to_vec()).is_ok();
                }
                Some(mailcraft_engine::Picture::Loading) => {}
                // Only the pictures the message shows are fetched (not its style sheets).
                _ => changed |= page.refuse(&url).is_ok(),
            }
        } else {
            match mime::picture_attachment(p, &url).and_then(|i| s.attachment_bytes(i).ok()) {
                Some(bytes) => changed |= page.give(&url, bytes).is_ok(),
                None => changed |= page.refuse(&url).is_ok(),
            }
        }
    }
    if changed {
        view.tiles.clear();
    }
    let height = page.height().max(1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width as f32, height), Sense::click());
    view.rect = Some(rect);
    view.hovered = None;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let visible = rect.intersect(ui.clip_rect());
    if visible.height() > 0.0 {
        let first = ((visible.top() - rect.top()) / TILE).floor().max(0.0) as usize;
        let last = ((visible.bottom() - rect.top()) / TILE).floor().max(0.0) as usize;
        let mut painted = 0;
        for i in first..=last {
            let top = i as f32 * TILE;
            if top >= height {
                break;
            }
            let h = TILE.min(height - top);
            if let std::collections::hash_map::Entry::Vacant(slot) = view.tiles.entry(i) {
                if painted >= TILES_PER_FRAME {
                    ui.ctx().request_repaint();
                    continue;
                }
                painted += 1;
                let Ok((w, ph, px)) = page.paint(top, h) else { return false };
                let img = egui::ColorImage::from_rgba_premultiplied([w as usize, ph as usize], &px);
                slot.insert(ui.ctx().load_texture(format!("blitz-{i}"), img, egui::TextureOptions::LINEAR));
            }
            if let Some(tex) = view.tiles.get(&i) {
                let at = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + top), vec2(width as f32, h));
                painter.image(tex.id(), at, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        // Tiles far from view go.
        view.tiles.retain(|i, _| *i + 2 >= first && *i <= last + 2);
    }
    // Links: the hand, where it goes, and a click opens it.
    if let Some(at) = response.hover_pos()
        && let Some(link) = page.link_at(at.x - rect.left(), at.y - rect.top())
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        view.hovered = Some(link.clone());
        let shown = if s.prefs.clean_links { mime::clean_link(&link) } else { link.clone() };
        let clicked = response.clicked();
        response.on_hover_text(shown);
        if clicked {
            acts.push(Action::OpenUrl(link));
        }
    }
    true
}
