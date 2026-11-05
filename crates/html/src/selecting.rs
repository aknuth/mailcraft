//! Selecting a page's text: Blitz keeps the selection (two ends, each a block of text and a
//! byte in it) and paints it shaded; this finds the ends under the pointer, a word or a block
//! around a point, the whole text, and what's selected as text to copy.

use super::*;
use blitz_dom::NodeId;

/// How far to the left of the pointer a drag looks for text when it's beside it (in a margin, or
/// past a line's end), in CSS pixels at a time.
const STEP: f32 = 16.0;

impl Page {
    /// Where in the text the point `x`, `y` (CSS pixels from the page's top left) is: the block
    /// of text and a byte in it.
    fn text_position(&self, x: f32, y: f32) -> Option<(NodeId, usize)> {
        let doc = &self.doc;
        guarded(|| doc.find_text_position(x, y)).ok().flatten()
    }

    /// The nearest text on the point's row, looking left from it (a drag past a line's end).
    fn text_position_near(&self, x: f32, y: f32) -> Option<(NodeId, usize)> {
        let steps = (x.max(0.0) / STEP).ceil().min(256.0) as usize;
        (0..=steps).find_map(|i| self.text_position((x - i as f32 * STEP).max(0.0), y))
    }

    /// Whether there's text under the point (for the pointer's I-beam).
    pub fn text_at(&self, x: f32, y: f32) -> bool {
        self.text_position(x, y).is_some()
    }

    /// A press at the point: the selection starts there (nothing selected yet), or is let go of
    /// when there's no text there. `true` when the shading changed.
    pub fn select_from(&mut self, x: f32, y: f32) -> bool {
        let had = self.has_selection();
        let at = self.text_position(x, y);
        let doc = &mut self.doc;
        let _ = guarded(|| match at {
            Some((n, o)) => doc.set_text_selection(n, o, n, o),
            None => doc.clear_text_selection(),
        });
        self.anchored = at.is_some();
        had
    }

    /// A drag (or Shift+click) to the point: the selection's end moves there. `true` when it moved.
    pub fn select_to(&mut self, x: f32, y: f32) -> bool {
        if !self.anchored {
            return false;
        }
        let Some((n, o)) = self.text_position(x, y).or_else(|| self.text_position_near(x, y)) else {
            return false;
        };
        let doc = &mut self.doc;
        guarded(|| doc.update_selection_focus(n, o)).is_ok()
    }

    /// The word at the point (a double-click), or its block of text (a triple-click).
    pub fn select_around(&mut self, x: f32, y: f32, block: bool) -> bool {
        let Some((n, o)) = self.text_position(x, y) else { return false };
        let doc = &mut self.doc;
        let done = guarded(|| {
            let Some(text) = block_text(doc, n) else { return false };
            let (start, end) = if block { (0, text.len()) } else { word_around(&text, o) };
            doc.set_text_selection(n, start, n, end);
            true
        })
        .unwrap_or(false);
        self.anchored |= done;
        done
    }

    /// The whole text (Ctrl+A).
    pub fn select_all(&mut self) -> bool {
        let doc = &mut self.doc;
        let done = guarded(|| {
            let blocks = text_blocks(doc);
            let (Some(first), Some(last)) = (blocks.first(), blocks.last()) else { return false };
            let end = block_text(doc, *last).map_or(0, |t| t.len());
            doc.set_text_selection(*first, 0, *last, end);
            true
        })
        .unwrap_or(false);
        self.anchored |= done;
        done
    }

    /// Nothing selected (Escape, another message). `true` when something was.
    pub fn clear_selection(&mut self) -> bool {
        let had = self.has_selection();
        let doc = &mut self.doc;
        let _ = guarded(|| doc.clear_text_selection());
        self.anchored = false;
        had
    }

    /// What's selected, to tell whether it changed (its shading has to be painted again).
    pub fn selection(&self) -> Vec<(u64, usize, usize)> {
        let doc = &self.doc;
        guarded(|| doc.get_text_selection_ranges().into_iter().map(|(n, a, b)| (n.as_u64(), a, b)).collect()).unwrap_or_default()
    }

    pub fn has_selection(&self) -> bool {
        let doc = &self.doc;
        guarded(|| doc.has_text_selection()).unwrap_or(false)
    }

    /// What's selected, a block of text to a line; `None` when nothing is.
    pub fn selected_text(&self) -> Option<String> {
        let doc = &self.doc;
        guarded(|| {
            let mut out = String::new();
            for (n, start, end) in doc.get_text_selection_ranges() {
                let Some(text) = block_text(doc, n) else { continue };
                let Some(part) = text.get(start.min(end)..end.max(start)) else { continue };
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(part);
            }
            Some(out).filter(|t| !t.is_empty())
        })
        .ok()
        .flatten()
    }
}

/// The text of a block laid out as lines (what selections count bytes in).
fn block_text(doc: &BaseDocument, n: NodeId) -> Option<String> {
    Some(doc.get_node(n)?.element_data()?.inline_layout_data.as_ref()?.text.clone())
}

/// Every block of text, in the order they're laid out.
fn text_blocks(doc: &BaseDocument) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        if node.element_data().is_some_and(|e| e.inline_layout_data.is_some()) {
            found.push(id);
            continue; // what's inside is laid out as its lines
        }
        let kids: Vec<NodeId> = match node.layout_children.borrow().as_ref() {
            Some(k) => k.to_vec(),
            None => node.children.to_vec(),
        };
        stack.extend(kids.into_iter().rev());
    }
    found
}

/// The word around byte `at` of `text` (letters, digits and `_`; or the run of other characters
/// that isn't a space).
fn word_around(text: &str, at: usize) -> (usize, usize) {
    let at = (0..=at.min(text.len())).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
    let kind = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            1
        } else if c.is_whitespace() {
            0
        } else {
            2
        }
    };
    // The character the point is on (or the one before it, at a word's end).
    let on = text[at..].chars().next().filter(|c| kind(*c) != 0).or_else(|| text[..at].chars().next_back());
    let Some(k) = on.map(kind) else { return (at, at) };
    let start = text[..at].char_indices().rev().take_while(|(_, c)| kind(*c) == k).last().map_or(at, |(i, _)| i);
    let end = text[at..].char_indices().find(|(_, c)| kind(*c) != k).map_or(text.len(), |(i, _)| at + i);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::word_around;

    #[test]
    fn words() {
        let t = "Hello, wörld_1 here";
        assert_eq!(
            &t[{
                let (a, b) = word_around(t, 9);
                a..b
            }],
            "wörld_1"
        );
        let (a, b) = word_around(t, 2);
        assert_eq!(&t[a..b], "Hello");
        let (a, b) = word_around(t, 4);
        assert_eq!(&t[a..b], "Hello");
        // Punctuation is a word of its own; past the text's end, the last word.
        let (a, b) = word_around(t, 5);
        assert_eq!(&t[a..b], ",");
        let (a, b) = word_around(t, 1000);
        assert_eq!(&t[a..b], "here");
        assert_eq!(word_around("", 0), (0, 0));
        // Inside a multi-byte letter: still that word.
        let (a, b) = word_around(t, 9);
        assert!(t.is_char_boundary(a) && t.is_char_boundary(b));
    }
}
