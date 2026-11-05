//! A row whose cells take 100% or more between them in percentages leaves the cells without a
//! width their least (CSS 2.1 automatic table layout, as browsers do it): Amazon's step tracker
//! is a 50% bar, an 18 pixel dot, a 50% bar. Blitz doesn't read cells' percentages in such a
//! table and shares the room out instead, so the dot's cell came out 112 wide and the bars
//! stopped short of the dots. A cell like that is given its least width as its width, where
//! that's sure: when all it holds is of a set width (a table or a picture `width="18"`).

use super::*;
use blitz_dom::NodeId;
use style::values::generics::length::GenericSize;
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// Cells that need it get their least width; `true` when any did.
pub(crate) fn fix(doc: &mut BaseDocument) -> bool {
    let mut found: Vec<(NodeId, f32)> = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        stack.extend(node.children.iter().copied());
        if node.primary_styles().is_none_or(|s| s.clone_display().inside() != DisplayInside::TableRow) {
            continue;
        }
        let cells: Vec<NodeId> = node
            .children
            .iter()
            .copied()
            .filter(|c| doc.get_node(*c).and_then(|n| n.primary_styles()).is_some_and(|s| s.clone_display().inside() == DisplayInside::TableCell))
            .collect();
        let mut percent = 0.0;
        let mut auto: Vec<NodeId> = Vec::new();
        for c in &cells {
            let Some(s) = doc.get_node(*c).and_then(|n| n.primary_styles()) else { continue };
            match s.clone_width() {
                GenericSize::LengthPercentage(l) => {
                    if let Some(p) = l.0.to_percentage() {
                        percent += p.0;
                    }
                }
                GenericSize::Auto => auto.push(*c),
                _ => {}
            }
        }
        if percent < 0.999 || auto.is_empty() {
            continue;
        }
        for c in auto {
            if let Some(w) = set_width(doc, c) {
                found.push((c, w));
            }
        }
    }
    if found.is_empty() {
        return false;
    }
    let mut m = doc.mutate();
    for (id, w) in found {
        m.set_style_property(id, "width", &format!("{w}px"));
    }
    true
}

/// The width of what the cell holds, when all of it has a set width (no text, nothing that
/// takes the room it gets).
fn set_width(doc: &BaseDocument, cell: NodeId) -> Option<f32> {
    let node = doc.get_node(cell)?;
    let mut most: Option<f32> = None;
    for c in node.children.iter().copied() {
        let c = doc.get_node(c)?;
        if c.is_text_node() {
            if c.text_content().trim().is_empty() {
                continue;
            }
            return None;
        }
        if !c.is_element() {
            continue;
        }
        let s = c.primary_styles()?;
        if s.clone_display().outside() == DisplayOutside::None {
            continue;
        }
        let GenericSize::LengthPercentage(l) = s.clone_width() else { return None };
        let w = l.0.to_length()?.px();
        if !w.is_finite() || w < 0.0 {
            return None;
        }
        most = Some(most.map_or(w, |m: f32| m.max(w)));
    }
    most
}
