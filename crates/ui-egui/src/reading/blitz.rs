//! PROTOTYPE: an HTML message laid out and drawn by Blitz ([`mailcraft_html`]), shown as tiles
//! of pixels, its links clickable. On with `MAILCRAFT_BLITZ=1`; a page Blitz refuses (nested
//! too deep, a panic) is drawn the usual way.
//!
//! Its text is selected as the other messages' is: a press starts a selection, a drag moves its
//! end (scrolling at the view's edge), Shift+click too, a double-click takes a word and a
//! triple-click a block; Ctrl+A, Escape and a copy are the text's while it has the keyboard
//! (from a press in it until a press elsewhere), and its menu copies what's selected or all.
//! Blitz paints the shading, so the tiles are painted again when it changes (the old ones shown
//! until then).
//!
//! Not done yet: the reading lane (it lays out on the window's thread), dark mode (mail is drawn
//! on white, as most mail expects).

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
    /// Painted tiles, each with the version of the page it shows.
    tiles: HashMap<usize, (egui::TextureHandle, u64)>,
    /// The page's version: one more each time it's laid out again or its shading changes.
    version: u64,
    /// Where it was drawn last, and the link under the pointer then.
    rect: Option<egui::Rect>,
    hovered: Option<String>,
    /// The text has the keyboard: pressed into, until a press elsewhere.
    active: bool,
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

    pub(crate) fn selected(&self) -> Option<String> {
        self.page.as_ref().ok().and_then(Page::selected_text)
    }

    pub(crate) fn active(&self) -> bool {
        self.active
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
        st.html_view = Some(HtmlView { key: key.clone(), page, tiles: HashMap::new(), version: 0, rect: None, hovered: None, active: false });
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
                    let _ = page.give(&url, bytes.to_vec());
                }
                Some(mailcraft_engine::Picture::Loading) => {}
                // Only the pictures the message shows are fetched (not its style sheets).
                _ => {
                    let _ = page.refuse(&url);
                }
            }
        } else {
            match mime::picture_attachment(p, &url).and_then(|i| s.attachment_bytes(i).ok()) {
                Some(bytes) => {
                    let _ = page.give(&url, bytes);
                }
                None => {
                    let _ = page.refuse(&url);
                }
            }
        }
    }
    // Laid out again once for all given this frame.
    changed |= matches!(page.update(), Ok(true));
    let height = page.height().max(1.0);
    // A finger dragged over it scrolls (as over other text); a mouse's selects.
    let touch = ui.input(|i| i.has_touch_screen());
    let (rect, response) = ui.allocate_exact_size(vec2(width as f32, height), if touch { Sense::click() } else { Sense::click_and_drag() });
    view.rect = Some(rect);
    view.hovered = None;
    let before = page.selection();
    select(ui, &response, rect, page, &mut view.active);
    if view.active {
        keys(ui, page);
        if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
            st.text_selection.keys = true;
        }
    }
    menu(&response, page);
    if changed || page.selection() != before {
        view.version += 1;
    }
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
            if view.tiles.get(&i).is_none_or(|(_, v)| *v != view.version) {
                if painted >= TILES_PER_FRAME {
                    ui.ctx().request_repaint();
                } else {
                    painted += 1;
                    let Ok((w, ph, px)) = page.paint(top, h) else { return false };
                    let img = egui::ColorImage::from_rgba_premultiplied([w as usize, ph as usize], &px);
                    match view.tiles.get_mut(&i) {
                        Some((tex, v)) => {
                            tex.set(img, egui::TextureOptions::LINEAR);
                            *v = view.version;
                        }
                        None => {
                            let tex = ui.ctx().load_texture(format!("blitz-{i}"), img, egui::TextureOptions::LINEAR);
                            view.tiles.insert(i, (tex, view.version));
                        }
                    }
                }
            }
            if let Some((tex, _)) = view.tiles.get(&i) {
                let at = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + top), vec2(width as f32, h));
                painter.image(tex.id(), at, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        // Tiles far from view go.
        view.tiles.retain(|i, _| *i + 2 >= first && *i <= last + 2);
    }
    // Links: the hand, where it goes, and a click opens it; over text, the I-beam.
    let over = response.hover_pos().map(|at| (at.x - rect.left(), at.y - rect.top()));
    let link = over.and_then(|(x, y)| page.link_at(x, y));
    if response.dragged() || (link.is_none() && over.is_some_and(|(x, y)| page.text_at(x, y))) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if let Some(link) = link {
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

/// The pointer on the page: a press, a double or triple click, Shift+click and a drag.
fn select(ui: &mut Ui, response: &egui::Response, rect: egui::Rect, page: &mut Page, active: &mut bool) {
    let local = |p: egui::Pos2| (p.x - rect.left(), p.y - rect.top());
    let (pressed, shift, double, triple, dt) = ui.input(|i| {
        let b = egui::PointerButton::Primary;
        (i.pointer.primary_pressed(), i.modifiers.shift, i.pointer.button_double_clicked(b), i.pointer.button_triple_clicked(b), i.stable_dt)
    });
    if pressed {
        // A press elsewhere takes the keyboard from it; one in it gives it the keyboard.
        *active = response.hovered();
        if let Some((x, y)) = response.interact_pointer_pos().or(response.hover_pos()).filter(|_| *active).map(local) {
            if triple {
                page.select_around(x, y, true);
            } else if double {
                page.select_around(x, y, false);
            } else if !(shift && page.select_to(x, y)) {
                page.select_from(x, y);
            }
        }
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(at) = response.interact_pointer_pos()
    {
        let (x, y) = local(at);
        page.select_to(x, y);
        // Held at or past the view's top or bottom: it scrolls, faster the farther out.
        let delta = text_selection::edge_delta(at.y, ui.clip_rect(), dt);
        if delta != 0.0 {
            ui.scroll_with_delta(vec2(0.0, delta));
            ui.ctx().request_repaint();
        }
    }
    if ui.ctx().text_edit_focused() {
        *active = false;
    }
}

/// The text's keys while it has the keyboard: Ctrl+A, Escape, and a copy.
fn keys(ui: &mut Ui, page: &mut Page) {
    let events = ui.input(|i| i.events.clone());
    let mut used = false;
    for e in &events {
        match e {
            egui::Event::Key { key: egui::Key::A, pressed: true, modifiers, .. } if modifiers.command && !modifiers.shift => {
                page.select_all();
                used = true;
            }
            egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
                page.clear_selection();
            }
            egui::Event::Copy | egui::Event::Cut => {
                if let Some(text) = page.selected_text() {
                    ui.ctx().copy_text(text);
                    used = true;
                }
            }
            _ => {}
        }
    }
    if used {
        ui.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key: egui::Key::A, modifiers, .. } => !modifiers.command,
                egui::Event::Copy | egui::Event::Cut => false,
                _ => true,
            })
        });
    }
}

/// Its menu: copy what's selected, or the whole message.
fn menu(response: &egui::Response, page: &Page) {
    response.context_menu(|ui| {
        if let Some(text) = page.selected_text()
            && ui.button(tl!("Copy")).clicked()
        {
            ui.ctx().copy_text(text);
            ui.close();
        }
        if ui.button(tl!("Copy the whole message")).clicked() {
            ui.ctx().copy_text(page.text().trim().to_string());
            ui.close();
        }
    });
}
