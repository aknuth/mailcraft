//! An inline block without a width of its own is as wide as what it holds, but no wider than
//! the room it has (CSS 2.1, section 10.3.9, "shrink-to-fit"). Blitz leaves that last part out
//! when what it holds is a block-level picture `width: 100%` wide: eBay's product pictures (448
//! pixels, in a link that is an inline block, in a 130 pixel column) came out at full size, over
//! their neighbours. Such a block gets `max-width: 100%`, which is the same thing but where its
//! content can't be narrower than the room (a long word), and which Blitz keeps to.

use super::*;
use blitz_dom::NodeId;
use style::values::generics::length::{GenericMaxSize, GenericSize};
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// Inline blocks that need it get `max-width: 100%`; `true` when any did.
pub(crate) fn fix(doc: &mut BaseDocument) -> bool {
    let mut found: Vec<NodeId> = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        stack.extend(node.children.iter().copied());
        let Some(s) = node.primary_styles() else { continue };
        let display = s.clone_display();
        if display.outside() == DisplayOutside::Inline
            && display.inside() == DisplayInside::FlowRoot
            && matches!(s.clone_width(), GenericSize::Auto)
            && matches!(s.clone_max_width(), GenericMaxSize::None)
            && !node.data.is_element_with_tag_name(&local_name!("img"))
        {
            found.push(id);
        }
    }
    if found.is_empty() {
        return false;
    }
    let mut m = doc.mutate();
    for id in found {
        m.set_style_property(id, "max-width", "100%");
    }
    true
}
