//! HTML mail laid out and drawn as a browser would: Blitz (Stylo, Servo's CSS engine; Taffy for
//! block, inline, flex, grid and table layout; Parley for text) into pixels on the CPU.
//!
//! A [`Page`] is one message's HTML at one width. Nothing here touches the network or runs
//! scripts: Blitz has neither, and every picture or style sheet the HTML names is only *asked
//! for* ([`Page::wanted`]); the caller decides what to [`Page::give`] (a picture sent with the
//! message, a web picture the user allowed) and the rest is never loaded.
//!
//! Like everything from mail the HTML is hostile input. Parsing is safe at any depth, but
//! styling, layout and painting recurse: a document nested deeper than [`MAX_DEPTH`] is refused
//! (the caller shows it another way), and every call into Blitz runs under `catch_unwind`, a
//! panic refusing the page instead of ending MailCraft. Run it on a thread with room on its
//! stack: 1 MiB held [`MAX_DEPTH`] in tests, 8 MiB four times as much.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, UserAgentStylesheet, local_name};
use blitz_html::HtmlDocument;
use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};

/// Elements nested deeper than this: the page is refused.
pub const MAX_DEPTH: usize = 192;
/// The tallest page laid out, in CSS pixels (the rest is cut off).
pub const MAX_HEIGHT: f32 = 200_000.0;
/// The widest page, in CSS pixels.
pub const MAX_WIDTH: u32 = 4_000;
/// Things asked for by one page, at most (pictures, style sheets).
const MAX_WANTED: usize = 400;

/// Why a page can't be drawn this way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Nested deeper than [`MAX_DEPTH`].
    TooDeep(usize),
    /// Blitz panicked (a bug of its own, met by hostile or unusual markup).
    Panicked,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::TooDeep(d) => write!(f, "nested {d} deep"),
            Refused::Panicked => write!(f, "the HTML renderer failed"),
        }
    }
}

/// What a page asked for and hasn't been given yet.
#[derive(Default)]
struct Asked {
    open: Vec<(String, Box<dyn NetHandler>)>,
    /// Every address asked for, in order (answered or not).
    seen: Vec<String>,
}

/// Collects what the page asks for; nothing is fetched here.
struct Collector(Arc<Mutex<Asked>>);

impl NetProvider for Collector {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.to_string();
        if let Ok(mut a) = self.0.lock()
            && a.seen.len() < MAX_WANTED
        {
            a.seen.push(url.clone());
            a.open.push((url, handler));
        }
    }
}

/// One message's HTML, laid out at one width.
pub struct Page {
    doc: HtmlDocument,
    asked: Arc<Mutex<Asked>>,
    width: u32,
    scale: f32,
    dark: bool,
    height: f32,
}

fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, Refused> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| Refused::Panicked)
}

/// How deep the document's elements go (counted without recursion).
fn depth(doc: &BaseDocument) -> usize {
    let mut most = 0;
    let mut stack = vec![(doc.root_node().id, 0usize)];
    while let Some((id, d)) = stack.pop() {
        most = most.max(d);
        if d > MAX_DEPTH {
            break;
        }
        if let Some(n) = doc.get_node(id) {
            stack.extend(n.children.iter().map(|c| (*c, d + 1)));
        }
    }
    most
}

/// The style sheets it links to (resolved as Blitz asks for them). A mail's web style sheets
/// are never loaded: Blitz waits for those in `<head>` before laying anything out, so they're
/// answered empty at once.
/// What mail needs on top of Blitz's own sheet. Blitz leaves out a table cell whose `display`
/// is not `table-cell` with all it holds (a browser wraps it in a cell of its own), and mail
/// templates set `display: inline-block` or `block` on their cells to stack them on phones. A
/// cell given one in its `style` attribute stays a cell (a user agent's `!important` outranks
/// the mail's own); one hidden that way stays hidden.
fn mail_sheet() -> String {
    let mut cells = Vec::new();
    for tag in ["td", "th"] {
        for value in ["inline", "block", "flex", "grid", "list-item"] {
            for gap in ["", " "] {
                cells.push(format!("{tag}[style*=\"display:{gap}{value}\" i]"));
            }
        }
    }
    format!("{} {{ display: table-cell !important; }}", cells.join(", "))
}

const BASE: &str = "mailcraft-message:/";

fn style_sheets(doc: &BaseDocument) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(n) = doc.get_node(id) else { continue };
        stack.extend(n.children.iter().copied());
        if n.data.is_element_with_tag_name(&local_name!("link"))
            && n.attr(local_name!("rel")).is_some_and(|r| r.split_ascii_whitespace().any(|r| r.eq_ignore_ascii_case("stylesheet")))
            && let Some(href) = n.attr(local_name!("href"))
            && let Ok(url) = doc.base_url().join(href)
        {
            found.push(url.to_string());
        }
    }
    found
}

fn viewport(width: u32, scale: f32, dark: bool) -> Viewport {
    let px = |v: f32| (v * scale).round().clamp(1.0, f32::from(u16::MAX)) as u32;
    Viewport::new(px(width as f32), px(1000.0), scale, if dark { ColorScheme::Dark } else { ColorScheme::Light })
}

impl Page {
    /// `html` laid out `width` CSS pixels wide, for a screen of `scale` pixels per CSS pixel,
    /// light or `dark` (what `prefers-color-scheme` answers).
    pub fn new(html: &str, width: u32, scale: f32, dark: bool) -> Result<Page, Refused> {
        let width = width.clamp(1, MAX_WIDTH);
        let scale = if scale.is_finite() { scale.clamp(0.5, 4.0) } else { 1.0 };
        let asked = Arc::new(Mutex::new(Asked::default()));
        let config = DocumentConfig {
            viewport: Some(viewport(width, scale, dark)),
            net_provider: Some(Arc::new(Collector(asked.clone()))),
            // A relative address has nothing to resolve against in a mail (Blitz fails on one
            // without a base); this base makes it one nothing is ever given for.
            base_url: Some(BASE.to_string()),
            ua_stylesheets: vec![UserAgentStylesheet::default_sheet(), UserAgentStylesheet::parse(&mail_sheet())],
            ..Default::default()
        };
        let doc = guarded(|| HtmlDocument::from_html(html, config))?;
        let d = guarded(|| depth(&doc))?;
        if d > MAX_DEPTH {
            return Err(Refused::TooDeep(d));
        }
        let sheets = guarded(|| style_sheets(&doc))?;
        let mut page = Page { doc, asked, width, scale, dark, height: 0.0 };
        for url in sheets {
            page.answer(&url, bytes::Bytes::new())?;
        }
        page.lay_out()?;
        Ok(page)
    }

    fn lay_out(&mut self) -> Result<(), Refused> {
        let doc = &mut self.doc;
        let h = guarded(|| {
            doc.handle_messages();
            doc.resolve(0.0);
            doc.root_element().final_layout().size.height
        })?;
        self.height = if h.is_finite() { h.clamp(0.0, MAX_HEIGHT) } else { 0.0 };
        Ok(())
    }

    /// Its height in CSS pixels.
    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn dark(&self) -> bool {
        self.dark
    }

    /// Laid out again at another width or scale (a pane resized, a window moved to another
    /// screen); `true` when anything changed.
    pub fn set_size(&mut self, width: u32, scale: f32) -> Result<bool, Refused> {
        let width = width.clamp(1, MAX_WIDTH);
        if width == self.width && (scale - self.scale).abs() < 0.01 {
            return Ok(false);
        }
        (self.width, self.scale) = (width, scale);
        let vp = viewport(width, scale, self.dark);
        let doc = &mut self.doc;
        guarded(|| doc.set_viewport(vp))?;
        self.lay_out()?;
        Ok(true)
    }

    /// What it asked for and hasn't been given (pictures, style sheets), in order.
    pub fn wanted(&self) -> Vec<String> {
        self.asked.lock().map(|a| a.open.iter().map(|(u, _)| u.clone()).collect()).unwrap_or_default()
    }

    /// Every address it asked for, given or not.
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().map(|a| a.seen.clone()).unwrap_or_default()
    }

    /// Hand it what it asked for at `url` (a picture's bytes), and lay it out again.
    pub fn give(&mut self, url: &str, bytes: Vec<u8>) -> Result<(), Refused> {
        if self.answer(url, bytes::Bytes::from(bytes))? {
            self.lay_out()?;
        }
        Ok(())
    }

    /// Never to be given: answered empty, so nothing waits for it, and laid out again.
    pub fn refuse(&mut self, url: &str) -> Result<(), Refused> {
        if self.answer(url, bytes::Bytes::new())? {
            self.lay_out()?;
        }
        Ok(())
    }

    /// Every open request for `url` answered with `data`; whether there was one.
    fn answer(&mut self, url: &str, data: bytes::Bytes) -> Result<bool, Refused> {
        let handlers: Vec<Box<dyn NetHandler>> = match self.asked.lock() {
            Ok(mut a) => {
                let (these, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut a.open).into_iter().partition(|(u, _)| u == url);
                a.open = rest;
                these.into_iter().map(|(_, h)| h).collect()
            }
            Err(_) => Vec::new(),
        };
        let any = !handlers.is_empty();
        for h in handlers {
            let (u, d) = (url.to_string(), data.clone());
            guarded(move || h.bytes(u, d))?;
        }
        Ok(any)
    }

    /// The part of the page from `top`, `height` CSS pixels tall, as RGBA pixels
    /// (`width × scale` by `height × scale`, rounded); with its size in pixels.
    pub fn paint(&mut self, top: f32, height: f32) -> Result<(u32, u32, Vec<u8>), Refused> {
        let px = |v: f32| (v * self.scale).round().clamp(1.0, 16_384.0) as u32;
        let (w, h) = (px(self.width as f32), px(height.max(1.0)));
        let scale = f64::from(self.scale);
        let doc = &mut self.doc;
        // `paint_scene`'s offsets move the drawing down the picture; the document's own scroll
        // is what brings a part further down into it. Put back afterwards: hit tests count from
        // the page's top.
        let pixels = guarded(|| {
            doc.set_viewport_scroll(blitz_dom::Point { x: 0.0, y: f64::from(top.max(0.0)) });
            let px = render_to_buffer::<VelloCpuImageRenderer, _>(|s| blitz_paint::paint_scene(s, doc, scale, w, h, 0, 0), w, h);
            doc.set_viewport_scroll(blitz_dom::Point { x: 0.0, y: 0.0 });
            px
        })?;
        Ok((w, h, pixels))
    }

    /// The link at `(x, y)` (CSS pixels from the page's top left), when there's one.
    pub fn link_at(&self, x: f32, y: f32) -> Option<String> {
        let doc = &self.doc;
        guarded(|| {
            let hit = doc.hit(x, y)?;
            let mut id = Some(hit.node_id);
            for _ in 0..MAX_DEPTH + 8 {
                let node = doc.get_node(id?)?;
                if node.data.is_element_with_tag_name(&local_name!("a"))
                    && let Some(href) = node.attr(local_name!("href"))
                {
                    return Some(href.trim().to_string());
                }
                id = node.parent;
            }
            None
        })
        .ok()
        .flatten()
    }

    /// The document's text, for search and copying (its body's).
    pub fn text(&self) -> String {
        let doc = &self.doc;
        guarded(|| doc.root_element().text_content()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
