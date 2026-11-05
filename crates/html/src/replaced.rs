//! A picture's size as browsers work it out where Blitz doesn't yet: given a `min-width`, Blitz
//! lets it set the height too (through the picture's proportions) even when the height is set,
//! so Shopify's spacer (`min-width: 600px; height: 0`) came out 600 pixels square. A set height
//! is made its `max-height` as well, which Blitz keeps to.

use super::*;
use blitz_dom::NodeId;
use style::values::generics::length::GenericSize;

/// Pictures that need it get their height as a `max-height`; `true` when any did.
pub(crate) fn fix(doc: &mut BaseDocument) -> bool {
    let mut found: Vec<(NodeId, f32)> = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        stack.extend(node.children.iter().copied());
        if !node.data.is_element_with_tag_name(&local_name!("img")) {
            continue;
        }
        let Some(s) = node.primary_styles() else { continue };
        let min_set = match s.clone_min_width() {
            GenericSize::LengthPercentage(l) => l.0.to_length().is_none_or(|l| l.px() > 0.0),
            _ => false,
        };
        let height = match s.clone_height() {
            GenericSize::LengthPercentage(l) => l.0.to_length().map(|l| l.px()),
            _ => None,
        };
        let capped = match s.clone_max_height() {
            style::values::generics::length::GenericMaxSize::LengthPercentage(l) => l.0.to_length().map(|l| l.px()),
            _ => None,
        };
        if let Some(h) = height.filter(|h| min_set && h.is_finite() && capped.is_none_or(|c| c > *h)) {
            found.push((id, h));
        }
    }
    if found.is_empty() {
        return false;
    }
    let mut m = doc.mutate();
    for (id, h) in found {
        m.set_style_property(id, "max-height", &format!("{h}px"));
    }
    true
}
