//! The anonymous table boxes Blitz doesn't make yet (CSS 2.1, section 17.2.1): a table's child
//! that isn't a row (or a row group, a caption, a column) is put in a row of its own with a cell
//! around it, and a row's child that isn't a cell in a cell, with the siblings next to it that
//! aren't either. Blitz leaves such a child out with all it holds, and mail does this all the
//! time: a `<td>` stacked on phones with `display: block`, columns floated inside a
//! `display: table` box. The boxes are made as elements of their own (`mailcraft-row`,
//! `mailcraft-cell`, which no mail's style sheet names), once the styles say what's displayed how.

use super::*;
use blitz_dom::{Attribute, NodeId, QualName, ns};
use style::values::specified::box_::{DisplayInside, DisplayOutside};

/// What a box needs around the children that don't fit in it.
#[derive(Clone, Copy, PartialEq)]
enum Needs {
    /// A row and a cell (in a table or a row group).
    Row,
    /// A cell (in a row).
    Cell,
}

/// Puts the children that don't fit their table in anonymous boxes; `true` when any were (the
/// page has to be styled and laid out again).
pub(crate) fn wrap(doc: &mut BaseDocument) -> bool {
    let plan = runs(doc);
    if plan.is_empty() {
        return false;
    }
    let mut m = doc.mutate();
    for (needs, run) in plan {
        let Some((first, _)) = run.first().copied() else { continue };
        let cell = || style("display: table-cell !important");
        match needs {
            Needs::Cell => {
                let c = m.create_element(name("mailcraft-cell"), vec![cell()]);
                m.insert_nodes_before(first, &[c]);
                m.append_children(c, &run.iter().map(|(id, _)| *id).collect::<Vec<_>>());
            }
            // In the row, the cells stay cells; the rest between them gets a cell around it.
            Needs::Row => {
                let row = m.create_element(name("mailcraft-row"), vec![style("display: table-row !important")]);
                m.insert_nodes_before(first, &[row]);
                let mut rest: Vec<NodeId> = Vec::new();
                for (id, is_cell) in run {
                    if is_cell {
                        if !rest.is_empty() {
                            let c = m.create_element(name("mailcraft-cell"), vec![cell()]);
                            m.append_children(row, &[c]);
                            m.append_children(c, &std::mem::take(&mut rest));
                        }
                        m.append_children(row, &[id]);
                    } else {
                        rest.push(id);
                    }
                }
                if !rest.is_empty() {
                    let c = m.create_element(name("mailcraft-cell"), vec![cell()]);
                    m.append_children(row, &[c]);
                    m.append_children(c, &rest);
                }
            }
        }
    }
    true
}

fn name(local: &str) -> QualName {
    QualName::new(None, ns!(html), local.into())
}

fn style(value: &str) -> Attribute {
    Attribute { name: QualName::new(None, ns!(), "style".into()), value: value.to_string() }
}

/// Every run of children (siblings next to each other) that doesn't fit its table box, with
/// what it needs around it, and which of them are cells.
fn runs(doc: &BaseDocument) -> Vec<(Needs, Vec<(NodeId, bool)>)> {
    let mut plan = Vec::new();
    let mut stack = vec![doc.root_node().id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        stack.extend(node.children.iter().copied());
        let Some(display) = node.primary_styles().map(|s| s.clone_display()) else { continue };
        let needs = match display.inside() {
            DisplayInside::Table | DisplayInside::TableRowGroup | DisplayInside::TableHeaderGroup | DisplayInside::TableFooterGroup => Needs::Row,
            DisplayInside::TableRow => Needs::Cell,
            _ => continue,
        };
        let mut run: Vec<(NodeId, bool)> = Vec::new();
        let mut any = false;
        for child in node.children.iter().copied() {
            match fits(doc, child, needs) {
                Fit::Yes => {
                    if any {
                        plan.push((needs, std::mem::take(&mut run)));
                    }
                    run.clear();
                    any = false;
                }
                Fit::No => {
                    run.push((child, false));
                    any = true;
                }
                // A cell straight in a table goes in the anonymous row with its neighbours, but
                // alone (no neighbour that needs one) Blitz lays it out well as it is.
                Fit::Cell => run.push((child, true)),
                Fit::Neither => {}
            }
        }
        if any {
            plan.push((needs, run));
        }
    }
    plan
}

enum Fit {
    Yes,
    No,
    /// A cell straight in a table or row group.
    Cell,
    /// No box of its own (white space, a comment, `display: none`, taken out of the flow):
    /// left where it is, without ending a run.
    Neither,
}

fn fits(doc: &BaseDocument, id: NodeId, needs: Needs) -> Fit {
    let Some(node) = doc.get_node(id) else { return Fit::Neither };
    if node.is_text_node() {
        return if node.text_content().trim().is_empty() { Fit::Neither } else { Fit::No };
    }
    if !node.is_element() {
        return Fit::Neither;
    }
    let Some(display) = node.primary_styles().map(|s| s.clone_display()) else { return Fit::Neither };
    if display.outside() == DisplayOutside::None
        || display.inside() == DisplayInside::Contents
        || node.primary_styles().is_some_and(|s| s.clone_position().is_absolutely_positioned())
    {
        return Fit::Neither;
    }
    let fits = match needs {
        Needs::Cell => display.inside() == DisplayInside::TableCell,
        Needs::Row => {
            display.outside() == DisplayOutside::TableCaption
                || matches!(
                    display.inside(),
                    DisplayInside::TableRow
                        | DisplayInside::TableRowGroup
                        | DisplayInside::TableHeaderGroup
                        | DisplayInside::TableFooterGroup
                        | DisplayInside::TableColumn
                        | DisplayInside::TableColumnGroup
                )
        }
    };
    if needs == Needs::Row && display.inside() == DisplayInside::TableCell {
        return Fit::Cell;
    }
    if fits { Fit::Yes } else { Fit::No }
}
