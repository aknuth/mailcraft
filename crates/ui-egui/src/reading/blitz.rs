//! PROTOTYPE: an HTML message laid out and drawn by Blitz ([`mailcraft_html`]), shown as tiles
//! of pixels, its links clickable. On unless `MAILCRAFT_BLITZ=0`; a page Blitz refuses (nested
//! too deep, a panic) is drawn the usual way.
//!
//! The page is laid out and painted on the reading lane ([`lane`]); the window shows what came
//! back (a spinner until it's laid out, old tiles until new ones come) and hands on what's asked.
//!
//! Its text is selected as the other messages' is: a press starts a selection, a drag moves its
//! end (scrolling at the view's edge), Shift+click too, a double-click takes a word and a
//! triple-click a block; Ctrl+A, Escape and a copy are the text's while it has the keyboard
//! (from a press in it until a press elsewhere), and its menu copies what's selected or all.
//! Blitz paints the shading, so the tiles are painted again when it changes.
//!
//! Not done yet: dark mode (mail is drawn on white, as most mail expects).

mod lane;

use super::*;
use lane::{Shared, Step, lock};
use mailcraft_html::Page;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Tiles are this many CSS pixels tall.
const TILE: f32 = 512.0;

/// Pages made, counted (each message opened gets a new one).
static PAGES: AtomicU64 = AtomicU64::new(0);

/// The open message's page (on the lane) and its tiles here.
pub(crate) struct HtmlView {
    key: MessageKey,
    shared: Arc<Mutex<Shared>>,
    /// Tiles uploaded, each with the page's version it shows.
    tiles: HashMap<usize, (egui::TextureHandle, u64)>,
    /// Tiles asked for, with the version they were asked at (not asked again meanwhile).
    asked: HashMap<usize, u64>,
    /// Pictures given or refused (not handed over again).
    answered: HashSet<String>,
    /// Where it was drawn last, and the link under the pointer then.
    rect: Option<egui::Rect>,
    hovered: Option<String>,
    /// The pointer's point last handed on.
    pointer: Option<(f32, f32)>,
    /// The text has the keyboard: pressed into, until a press elsewhere.
    active: bool,
}

impl HtmlView {
    fn shared(&self) -> Arc<Mutex<Shared>> {
        self.shared.clone()
    }
}

#[cfg(test)]
impl HtmlView {
    /// Whether Blitz laid it out (and it's drawn as tiles).
    pub(crate) fn drawn(&self) -> bool {
        lock(&self.shared).made.refused.is_none() && !self.tiles.is_empty()
    }

    pub(crate) fn rect(&self) -> Option<egui::Rect> {
        self.rect
    }

    pub(crate) fn hovered_link(&self) -> Option<String> {
        self.hovered.clone()
    }

    pub(crate) fn selected(&self) -> Option<String> {
        lock(&self.shared).made.selected.clone()
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }
}

/// Whether the prototype is on: unless `MAILCRAFT_BLITZ=0` (the tests' windows draw messages
/// the usual way unless a test turns it on).
pub(crate) fn wanted() -> bool {
    match std::env::var_os("MAILCRAFT_BLITZ") {
        Some(v) => v != "0",
        None => !cfg!(test),
    }
}

/// Draw `html` with Blitz; `false` when it can't be (drawn the usual way then).
pub(super) fn draw(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, html: &str, acts: &mut Vec<Action>) -> bool {
    let size = (ui.available_width().floor().max(1.0) as u32, ui.ctx().pixels_per_point());
    if st.html_view.as_ref().is_none_or(|v| v.key != *key) {
        let id = PAGES.fetch_add(1, Ordering::Relaxed);
        st.html_view = Some(HtmlView {
            key: key.clone(),
            shared: Arc::new(Mutex::new(Shared::new(id, html, size))),
            tiles: HashMap::new(),
            asked: HashMap::new(),
            answered: HashSet::new(),
            rect: None,
            hovered: None,
            pointer: None,
            active: false,
        });
    }
    let Some(view) = st.html_view.as_mut() else { return false };
    let shared = view.shared.clone();
    let mut sh = lock(&shared);
    if sh.made.refused.is_some() {
        return false;
    }
    if sh.size != size {
        sh.size = size;
        sh.asks.resize = true;
    }
    // Tiles painted since: uploaded.
    for t in std::mem::take(&mut sh.made.tiles) {
        let img = egui::ColorImage::from_rgba_premultiplied(t.size, &t.pixels);
        match view.tiles.get_mut(&t.index) {
            Some((tex, v)) => {
                tex.set(img, egui::TextureOptions::LINEAR);
                *v = t.version;
            }
            None => {
                let tex = ui.ctx().load_texture(format!("blitz-{}", t.index), img, egui::TextureOptions::LINEAR);
                view.tiles.insert(t.index, (tex, t.version));
            }
        }
    }
    answer(s, p, view, &mut sh);
    if !sh.made.laid_out {
        ui.add_space(8.0);
        ui.spinner();
        if sh.wants_pump() {
            st.html_pump = true;
        }
        return true;
    }
    let (height, version) = (sh.made.height.max(1.0), sh.made.version);
    // A finger dragged over it scrolls (as over other text); a mouse's selects.
    let touch = ui.input(|i| i.has_touch_screen());
    let (rect, response) = ui.allocate_exact_size(vec2(size.0 as f32, height), if touch { Sense::click() } else { Sense::click_and_drag() });
    view.rect = Some(rect);
    view.hovered = None;
    let local = |p: egui::Pos2| (p.x - rect.left(), p.y - rect.top());
    select(ui, &response, rect, &mut sh.asks.steps, &mut view.active);
    if view.active {
        keys(ui, &mut sh);
        if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
            st.text_selection.keys = true;
        }
    }
    menu(&response, sh.made.selected.clone(), &p.text);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let visible = rect.intersect(ui.clip_rect());
    if visible.height() > 0.0 {
        let first = ((visible.top() - rect.top()) / TILE).floor().max(0.0) as usize;
        let last = ((visible.bottom() - rect.top()) / TILE).floor().max(0.0) as usize;
        for i in first..=last {
            let top = i as f32 * TILE;
            if top >= height {
                break;
            }
            let current = view.tiles.get(&i).is_some_and(|(_, v)| *v == version);
            if !current && view.asked.get(&i) != Some(&version) {
                view.asked.insert(i, version);
                sh.asks.tiles.push(i);
            }
            if let Some((tex, _)) = view.tiles.get(&i) {
                let at = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + top), vec2(size.0 as f32, TILE.min(height - top)));
                painter.image(tex.id(), at, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        // Tiles far from view go.
        view.tiles.retain(|i, _| *i + 2 >= first && *i <= last + 2);
        view.asked.retain(|i, _| *i + 2 >= first && *i <= last + 2);
    }
    // The pointer: handed on when it moves; the link and text under it come back.
    let over = response.hover_pos().map(local);
    if over.is_some() && over != view.pointer {
        view.pointer = over;
        sh.asks.hover = over;
    }
    let (link, text) = match (&sh.made.hover, over) {
        (Some((at, link, text)), Some(o)) if *at == o => (link.clone(), *text),
        _ => (None, false),
    };
    if response.dragged() || (link.is_none() && text) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if sh.wants_pump() {
        st.html_pump = true;
    }
    drop(sh);
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

/// What the page asks for: pictures sent with it, and web pictures once allowed.
fn answer(s: &Session, p: &mime::Parsed, view: &mut HtmlView, sh: &mut Shared) {
    let allowed = s.pictures_allowed();
    for url in sh.made.wanted.clone() {
        if view.answered.contains(&url) {
            continue;
        }
        let Some(gift) = gift(s, p, &url, allowed) else { continue };
        view.answered.insert(url.clone());
        sh.asks.gifts.push((url, gift));
    }
}

/// What the page is handed for `url`: a picture sent with the message, or a web picture once
/// allowed and downloaded (`Some(None)`: refused; `None`: not yet, asked again later).
fn gift(s: &Session, p: &mime::Parsed, url: &str, allowed: bool) -> Option<Option<Vec<u8>>> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        if !allowed {
            return None; // asked again once the user allows them
        }
        match s.pictures.get(url) {
            Some(mailcraft_engine::Picture::Ready(bytes)) => Some(Some(bytes.to_vec())),
            Some(mailcraft_engine::Picture::Loading) => None,
            // Only the pictures the message shows are fetched (not its style sheets).
            _ => Some(None),
        }
    } else {
        Some(mime::picture_attachment(p, url).and_then(|i| s.attachment_bytes(i).ok()))
    }
}

/// CSS pixels across the paper: about a browser's on A4 (it shrinks a wider page to fit).
const PRINT_WIDTH: u32 = 720;
/// Pixels per CSS pixel on paper (about 215 dots per inch).
const PRINT_SCALE: f32 = 2.0;

/// The open message as a PDF, its HTML drawn as Blitz draws it in the pane (its styles, and the
/// pictures shown there), laid out again at the paper's width on a thread with room on its
/// stack; `None` when the pane doesn't draw it with Blitz.
pub(crate) fn message_pdf(s: &Session, st: &UiState) -> Option<Result<Vec<u8>, String>> {
    let view = st.html_view.as_ref()?;
    let r = s.reading.as_ref().filter(|r| r.key == view.key)?;
    let p = r.parsed.as_ref()?;
    let html = p.html.clone()?;
    let urls: Vec<String> = {
        let sh = lock(&view.shared);
        if sh.made.refused.is_some() || !sh.made.laid_out {
            return None;
        }
        view.answered.iter().chain(sh.made.wanted.iter()).cloned().collect()
    };
    let allowed = s.pictures_allowed();
    let gifts: HashMap<String, Vec<u8>> = urls.into_iter().filter_map(|u| gift(s, p, &u, allowed).flatten().map(|b| (u, b))).collect();
    let doc = match s.message_doc() {
        Ok(d) => d,
        Err(e) => return Some(Err(e.to_string())),
    };
    let fonts = s.print_fonts.clone();
    let run = move || -> Result<Vec<u8>, String> {
        let mut page = Page::new(&html, PRINT_WIDTH, PRINT_SCALE, false).map_err(|e| e.to_string())?;
        // Pictures (and what they bring in) are handed over as asked for; the rest is refused.
        for _ in 0..4 {
            let wanted = page.wanted();
            if wanted.is_empty() {
                break;
            }
            for url in wanted {
                let _ = match gifts.get(&url) {
                    Some(b) => page.give(&url, b.clone()),
                    None => page.refuse(&url),
                };
            }
            page.update().map_err(|e| e.to_string())?;
        }
        let height = page.height();
        let mut paint = |top: f32, h: f32| page.paint(top, h).ok();
        let fonts = fonts.as_ref().map(|(r, b)| mailcraft_engine::print::Fonts { regular: r, bold: b.as_deref() });
        let drawn = mailcraft_engine::print::Drawn { width: PRINT_WIDTH as f32, height, paint: &mut paint };
        mailcraft_engine::print::pdf_with(&doc, fonts, Some(drawn))
    };
    let made = std::thread::Builder::new()
        .name("print-page".into())
        .stack_size(8 << 20)
        .spawn(run)
        .map_err(|e| e.to_string())
        .and_then(|h| h.join().map_err(|_| "the message couldn't be drawn for printing".to_string()));
    Some(made.and_then(|r| r))
}

/// The pointer on the page: a press, a double or triple click, Shift+click and a drag.
fn select(ui: &mut Ui, response: &egui::Response, rect: egui::Rect, steps: &mut Vec<Step>, active: &mut bool) {
    let local = |p: egui::Pos2| (p.x - rect.left(), p.y - rect.top());
    let (pressed, shift, double, triple, dt) = ui.input(|i| {
        let b = egui::PointerButton::Primary;
        (i.pointer.primary_pressed(), i.modifiers.shift, i.pointer.button_double_clicked(b), i.pointer.button_triple_clicked(b), i.stable_dt)
    });
    if pressed {
        // A press elsewhere takes the keyboard from it; one in it gives it the keyboard.
        *active = response.hovered();
        if let Some((x, y)) = response.interact_pointer_pos().or(response.hover_pos()).filter(|_| *active).map(local) {
            steps.push(if triple {
                Step::Around(x, y, true)
            } else if double {
                Step::Around(x, y, false)
            } else if shift {
                Step::To(x, y)
            } else {
                Step::From(x, y)
            });
        }
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(at) = response.interact_pointer_pos()
    {
        let (x, y) = local(at);
        // Only the latest point of a drag matters.
        if let Some(Step::To(..)) = steps.last() {
            steps.pop();
        }
        steps.push(Step::To(x, y));
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

/// The text's keys while it has the keyboard: Ctrl+A, Escape, and a copy (of what's selected as
/// the lane last said).
fn keys(ui: &mut Ui, sh: &mut Shared) {
    let events = ui.input(|i| i.events.clone());
    let mut used = false;
    for e in &events {
        match e {
            egui::Event::Key { key: egui::Key::A, pressed: true, modifiers, .. } if modifiers.command && !modifiers.shift => {
                sh.asks.steps.push(Step::All);
                used = true;
            }
            egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
                sh.asks.steps.push(Step::Clear);
            }
            egui::Event::Copy | egui::Event::Cut => {
                if let Some(text) = sh.made.selected.clone() {
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
fn menu(response: &egui::Response, selected: Option<String>, whole: &str) {
    response.context_menu(|ui| {
        if let Some(text) = selected
            && ui.button(tl!("Copy")).clicked()
        {
            ui.ctx().copy_text(text);
            ui.close();
        }
        if ui.button(tl!("Copy the whole message")).clicked() {
            ui.ctx().copy_text(whole.trim().to_string());
            ui.close();
        }
    });
}
