//! PROTOTYPE: the open message's Blitz page lives on the reading lane, where it's laid out
//! (30 to 100 ms for a newsletter, frames the window couldn't draw) and painted. Blitz's pages
//! can't move between threads (two of their parts aren't `Send`), so the page stays on the
//! lane's thread (in a thread local) and the window and the lane share only what's asked of it
//! and what it made: the size, pictures given, selection steps, the pointer, tiles wanted; its
//! height, the tiles painted, the link under the pointer, what's selected.
//!
//! The window hands the lane one pump at a time; the pump does all that's been asked, then
//! looks again, and ends when nothing is. A lane that's given a new thread (or a test's, which
//! starts one for each piece of work) lays the page out again from its HTML; where there's no
//! lane the pump runs in the window after the frame.

use super::*;
use crate::{Lane, MailApp, Work};
use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A step of the selection, at a point (CSS pixels from the page's top left).
#[derive(Clone, Copy, Debug)]
pub(super) enum Step {
    From(f32, f32),
    To(f32, f32),
    /// The word there, or (`true`) its block.
    Around(f32, f32, bool),
    All,
    Clear,
}

/// What the window asks of the page since the lane last looked.
#[derive(Default)]
pub(super) struct Asks {
    /// Laid out at the size in [`Shared::size`] (a new page, a pane resized).
    pub resize: bool,
    /// Pictures given (`Some`) or refused, by address.
    pub gifts: Vec<(String, Option<Vec<u8>>)>,
    pub steps: Vec<Step>,
    /// The pointer moved here.
    pub hover: Option<(f32, f32)>,
    /// Tiles to paint (by number, from the top).
    pub tiles: Vec<usize>,
}

impl Asks {
    fn is_empty(&self) -> bool {
        !self.resize && self.gifts.is_empty() && self.steps.is_empty() && self.hover.is_none() && self.tiles.is_empty()
    }
}

/// A tile painted: its number, the page's version it shows, and its pixels.
pub(super) struct Tile {
    pub index: usize,
    pub version: u64,
    pub size: [usize; 2],
    pub pixels: Vec<u8>,
}

/// What the lane made of the page.
#[derive(Default)]
pub(super) struct Made {
    pub laid_out: bool,
    /// Blitz refused it (drawn the usual way then).
    pub refused: Option<String>,
    pub height: f32,
    /// One more each time it's laid out again or its shading changes.
    pub version: u64,
    /// Addresses it asked for and hasn't been given or refused.
    pub wanted: Vec<String>,
    /// Painted, for the window to take.
    pub tiles: Vec<Tile>,
    /// The pointer's point, the link there, and whether there's text there.
    pub hover: Option<((f32, f32), Option<String>, bool)>,
    pub selected: Option<String>,
}

/// What the window and the lane share for one page.
pub(super) struct Shared {
    /// The page (one for each message opened, counted up).
    pub id: u64,
    pub html: Arc<str>,
    /// Its width in CSS pixels, and the screen's pixels per CSS pixel.
    pub size: (u32, f32),
    pub asks: Asks,
    pub made: Made,
    /// A pump is handed over and hasn't ended.
    pub pumping: bool,
}

impl Shared {
    pub(super) fn new(id: u64, html: &str, size: (u32, f32)) -> Shared {
        Shared { id, html: html.into(), size, asks: Asks { resize: true, ..Asks::default() }, made: Made::default(), pumping: false }
    }

    /// Whether a pump needs handing over (something's asked and none is at work); it's counted
    /// as handed over from now.
    pub(super) fn wants_pump(&mut self) -> bool {
        if self.pumping || self.asks.is_empty() {
            return false;
        }
        self.pumping = true;
        true
    }
}

thread_local! {
    /// The page this thread laid out last, by its id.
    static PAGE: RefCell<Option<(u64, Page)>> = const { RefCell::new(None) };
}

/// The lane's side: what's asked is done to the page, until nothing more is.
pub(super) fn pump(shared: &Mutex<Shared>, ctx: &egui::Context) {
    loop {
        let (id, html, size, asks) = {
            let mut s = lock(shared);
            if s.asks.is_empty() || s.made.refused.is_some() {
                s.pumping = false;
                return;
            }
            (s.id, s.html.clone(), s.size, std::mem::take(&mut s.asks))
        };
        PAGE.with(|cell| serve(&mut cell.borrow_mut(), shared, id, &html, size, asks));
        ctx.request_repaint();
    }
}

/// One round of what was asked of page `id`.
fn serve(slot: &mut Option<(u64, Page)>, shared: &Mutex<Shared>, id: u64, html: &str, size: (u32, f32), asks: Asks) {
    let fresh = slot.as_ref().is_none_or(|(i, _)| *i != id);
    if fresh {
        *slot = None; // the page before goes first (it can be big)
        let started = web_time::Instant::now();
        match Page::new(html, size.0, size.1, false) {
            Ok(page) => {
                log::info!("blitz: laid out in {:?} ({} high)", started.elapsed(), page.height());
                *slot = Some((id, page));
            }
            Err(e) => {
                log::info!("blitz: refused ({e}); drawn the usual way");
                let mut s = lock(shared);
                if s.id == id {
                    s.made.refused = Some(e.to_string());
                }
                return;
            }
        }
    }
    let Some((_, page)) = slot.as_mut() else { return };
    let mut changed = fresh;
    changed |= matches!(page.set_size(size.0, size.1), Ok(true));
    for (url, gift) in asks.gifts {
        let _ = match gift {
            Some(bytes) => page.give(&url, bytes),
            None => page.refuse(&url),
        };
    }
    changed |= matches!(page.update(), Ok(true));
    let before = page.selection();
    for step in asks.steps {
        match step {
            Step::From(x, y) => {
                page.select_from(x, y);
            }
            Step::To(x, y) => {
                page.select_to(x, y);
            }
            Step::Around(x, y, block) => {
                page.select_around(x, y, block);
            }
            Step::All => {
                page.select_all();
            }
            Step::Clear => {
                page.clear_selection();
            }
        }
    }
    let reselected = page.selection() != before;
    let hover = asks.hover.map(|(x, y)| ((x, y), page.link_at(x, y), page.text_at(x, y)));
    let version = {
        let mut s = lock(shared);
        if s.id != id {
            return; // the window moved on
        }
        let m = &mut s.made;
        if changed || reselected {
            m.version = m.version.wrapping_add(1);
        }
        m.laid_out = true;
        m.height = page.height();
        m.wanted = page.wanted();
        if hover.is_some() {
            m.hover = hover;
        }
        if fresh || reselected {
            m.selected = page.selected_text();
        }
        m.version
    };
    // Painted outside the lock (the window draws meanwhile).
    let height = page.height().max(1.0);
    let mut painted = Vec::new();
    for index in asks.tiles {
        let top = index as f32 * TILE;
        if top >= height {
            continue;
        }
        if let Ok((w, h, pixels)) = page.paint(top, TILE.min(height - top)) {
            painted.push(Tile { index, version, size: [w as usize, h as usize], pixels });
        }
    }
    let mut s = lock(shared);
    if s.id == id && s.made.version == version {
        s.made.tiles.extend(painted);
    }
}

impl MailApp {
    /// The open HTML message's pump goes to the reading lane, when the pane asked for one (or
    /// runs here when no lane takes it).
    pub(crate) fn send_html_work(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.ui.html_pump) {
            return;
        }
        let Some(shared) = self.ui.html_view.as_ref().map(HtmlView::shared) else { return };
        let ctx = ctx.clone();
        let run = Box::new(move || pump(&shared, &ctx));
        if let Err(work) = self.services.background(Lane::Reading, Work { run, cancelled: Arc::new(AtomicBool::new(false)), first: false }) {
            (work.run)();
        }
    }
}
