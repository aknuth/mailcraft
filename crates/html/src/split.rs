//! An inline element holding a block is split around it, as browsers do (CSS 2.1, section 9.2.1.1):
//! what comes before the block stays on its line, the block is laid out as a block, and what
//! follows goes on in a piece of its own. Blitz instead starts a new line where the inline
//! element starts, so a forwarded mail's `From: "eBay" <ebay@ebay.com>` (the address read as an
//! element that never ends and holds the whole mail) had its header broken after the name.
//! The pieces are copies of the element (its name and attributes: a link stays a link, its colour
//! is still inherited); a block's copy is a block without boxes of its own, as the anonymous
//! block a browser makes there has none.

use super::*;
use blitz_dom::{Attribute, NodeId};
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// Splits each inline element that holds a block; `true` when any was.
pub(crate) fn split(doc: &mut BaseDocument) -> bool {
    let mut plan: Vec<(NodeId, Vec<Piece>)> = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        stack.extend(node.children.iter().copied());
        let Some(d) = node.primary_styles().map(|s| s.clone_display()) else { continue };
        if !(node.is_element() && d.outside() == DisplayOutside::Inline && d.inside() == DisplayInside::Flow) {
            continue;
        }
        if !node.children.iter().any(|c| is_block(doc, *c)) {
            continue;
        }
        // The first run of inline content stays in the element; then a piece for each block
        // and for each run between and after them.
        let mut pieces: Vec<Piece> = Vec::new();
        let mut seen_block = false;
        for c in node.children.iter().copied() {
            if is_block(doc, c) {
                seen_block = true;
                pieces.push(Piece::Block(c));
            } else if seen_block {
                match pieces.last_mut() {
                    Some(Piece::Inline(run)) => run.push(c),
                    _ => pieces.push(Piece::Inline(vec![c])),
                }
            }
        }
        plan.push((id, pieces));
    }
    if plan.is_empty() {
        return false;
    }
    for (id, pieces) in plan {
        let Some(node) = doc.get_node(id) else { continue };
        let Some(name) = node.data.downcast_element().map(|e| e.name.clone()) else { continue };
        let attrs: Vec<Attribute> = node.attrs().map(<[Attribute]>::to_vec).unwrap_or_default();
        let parent = node.parent;
        let moved: Vec<NodeId> = pieces
            .iter()
            .flat_map(|p| match p {
                Piece::Block(b) => vec![*b],
                Piece::Inline(run) => run.clone(),
            })
            .collect();
        // Blitz keeps the boxes it laid out last time in these lists; with children moved out
        // they'd still name them (built again from the tree when empty).
        for n in [Some(id), parent].into_iter().flatten() {
            if let Some(n) = doc.get_node(n) {
                *n.layout_children.borrow_mut() = None;
            }
        }
        for n in &moved {
            if let Some(n) = doc.get_node(*n) {
                n.layout_parent.set(None);
            }
        }
        let mut m = doc.mutate();
        let mut after = id;
        for piece in pieces {
            let (copy, kids) = match piece {
                Piece::Block(b) => (m.create_element(name.clone(), block_attrs(&attrs)), vec![b]),
                Piece::Inline(run) => (m.create_element(name.clone(), attrs.clone()), run),
            };
            m.insert_nodes_after(after, &[copy]);
            m.append_children(copy, &kids);
            after = copy;
        }
    }
    true
}

enum Piece {
    Block(NodeId),
    Inline(Vec<NodeId>),
}

/// A block-level child in the flow (not floated or taken out of it).
fn is_block(doc: &BaseDocument, id: NodeId) -> bool {
    let Some(node) = doc.get_node(id) else { return false };
    let Some(s) = node.primary_styles() else { return false };
    node.is_element()
        && s.clone_display().outside() == DisplayOutside::Block
        && !s.clone_position().is_absolutely_positioned()
        && s.clone_float() == style::computed_values::float::T::None
}

/// The element's attributes for a block's copy: its own boxes (margin, padding, border, background)
/// stay with the inline pieces.
fn block_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    let plain = "display:block!important;margin:0!important;padding:0!important;border:0!important;background:none!important";
    let mut out: Vec<Attribute> = attrs.iter().filter(|a| a.name.local != local_name!("style")).cloned().collect();
    let style = attrs.iter().find(|a| a.name.local == local_name!("style")).map_or(String::new(), |a| a.value.clone());
    let name = attrs.iter().find(|a| a.name.local == local_name!("style")).map(|a| a.name.clone());
    let value = if style.trim().is_empty() { plain.to_string() } else { format!("{style};{plain}") };
    out.push(Attribute { name: name.unwrap_or_else(|| blitz_dom::QualName::new(None, blitz_dom::ns!(), "style".into())), value });
    out
}
