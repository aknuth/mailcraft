//! What a table's `cellspacing` and `cellpadding` say, as browsers read them (HTML, section 15.3.8
//! "Tables"): the space between its cells, and the padding of its own cells (not those of a table
//! inside one). Blitz reads neither, so every mail table had the default 2 pixel gaps and 1 pixel
//! padding, adding up in nested tables. They come in as a sheet of the lowest rank, which any of
//! the mail's own rules outranks, as a browser's presentational hints are.

use super::*;
use blitz_dom::UserAgentStylesheet;
use std::collections::BTreeSet;

/// The most different values one mail gets rules for (each is a rule or two).
const MOST: usize = 64;

/// The sheet for the values `doc`'s tables use, if any.
pub(crate) fn sheet(doc: &BaseDocument) -> Option<UserAgentStylesheet> {
    let (mut spacing, mut padding) = (BTreeSet::new(), BTreeSet::new());
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        stack.extend(node.children.iter().copied());
        if !node.data.is_element_with_tag_name(&local_name!("table")) {
            continue;
        }
        if let Some(v) = node.attr(local_name!("cellspacing")).filter(|v| pixels(v).is_some()) {
            spacing.insert(v.to_string());
        }
        if let Some(v) = node.attr(local_name!("cellpadding")).filter(|v| pixels(v).is_some()) {
            padding.insert(v.to_string());
        }
    }
    let mut css = String::new();
    for v in spacing.iter().take(MOST) {
        let px = pixels(v).unwrap_or(0);
        css.push_str(&format!("table[cellspacing=\"{v}\"]{{border-spacing:{px}px}}"));
    }
    for v in padding.iter().take(MOST) {
        let px = pixels(v).unwrap_or(0);
        let t = format!("table[cellpadding=\"{v}\"]");
        css.push_str(&format!(
            "{t}>tbody>tr>td,{t}>tbody>tr>th,{t}>thead>tr>td,{t}>thead>tr>th,{t}>tfoot>tr>td,{t}>tfoot>tr>th,{t}>tr>td,{t}>tr>th{{padding:{px}px}}"
        ));
    }
    (!css.is_empty()).then(|| UserAgentStylesheet::parse(&css))
}

/// A value of whole pixels (`"0"`, `" 5 "`), as the attributes are written; at most 100.
fn pixels(v: &str) -> Option<u32> {
    let v = v.trim();
    if v.is_empty() || v.len() > 3 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse().ok().filter(|n| *n <= 100)
}
