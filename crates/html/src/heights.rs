//! A table cell's or row's height is the least it may be: what it holds makes it taller
//! (CSS 2.1, section 17.5.3). Blitz holds it to that height instead, so mail's leftovers for
//! Outlook (`<td height="21">` around a 40 pixel button) let the next button slide over this
//! one. Such a height becomes a `min-height` (and the height `auto`), which Blitz grows past.

use super::*;
use blitz_dom::NodeId;
use style::values::generics::length::GenericSize;
use style::values::specified::box_::DisplayInside;

/// Cells and rows of a set height get it as their least; `true` when any did.
pub(crate) fn fix(doc: &mut BaseDocument) -> bool {
    let mut found: Vec<(NodeId, f32)> = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        stack.extend(node.children.iter().copied());
        let Some(s) = node.primary_styles() else { continue };
        if !matches!(s.clone_display().inside(), DisplayInside::TableCell | DisplayInside::TableRow) {
            continue;
        }
        let GenericSize::LengthPercentage(h) = s.clone_height() else { continue };
        let Some(h) = h.0.to_length().map(|l| l.px()).filter(|h| h.is_finite() && *h > 0.0) else { continue };
        let least = match s.clone_min_height() {
            GenericSize::LengthPercentage(m) => m.0.to_length().map_or(0.0, |l| l.px()),
            _ => 0.0,
        };
        found.push((id, h.max(least)));
    }
    if found.is_empty() {
        return false;
    }
    let mut m = doc.mutate();
    for (id, h) in found {
        m.set_style_property(id, "min-height", &format!("{h}px"));
        m.set_style_property(id, "height", "auto");
    }
    true
}
