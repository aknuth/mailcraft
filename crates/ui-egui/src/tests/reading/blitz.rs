//! PROTOTYPE: HTML mail drawn by Blitz in the reading pane.

use super::*;

fn html_message(html: &str) -> Vec<u8> {
    format!("From: Shop <orders@shop.example>\r\nSubject: Your order\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n").into_bytes()
}

fn opened(raw: Vec<u8>, name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    a.ui.blitz = true;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    a.session.apply(MailEvent::Body { key, raw });
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, dir, ctx)
}

#[test]
fn an_html_message_is_drawn_as_tiles_with_its_links() {
    let html = r##"<table width="100%" cellpadding="0" cellspacing="0" bgcolor="#F5F3EF"><tr>
        <td style="padding:12px">Shop</td>
        <td align="right" style="width:100%"><a href="https://shop.example/orders" style="display:inline-block;padding:12px 16px">Your Orders</a></td>
      </tr></table><h1>Your package was shipped!</h1>"##;
    let (mut a, dir, ctx) = opened(html_message(html), "blitz-tiles");
    let view = a.ui.html_view.as_ref().expect("laid out by Blitz");
    assert!(view.drawn(), "a page with tiles");
    // Hovering "Your Orders" (at the header's right) shows where it goes.
    let area = view.rect().expect("drawn somewhere");
    let at = egui::pos2(area.right() - 30.0, area.top() + 20.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
    let link = a.ui.html_view.as_ref().and_then(|v| v.hovered_link());
    assert_eq!(link.as_deref(), Some("https://shop.example/orders"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Nested too deep for Blitz: drawn the usual way.
#[test]
fn a_page_blitz_refuses_is_drawn_the_usual_way() {
    let html = format!("{}Deep text{}", "<div>".repeat(400), "</div>".repeat(400));
    let (a, dir, _) = opened(html_message(&html), "blitz-refused");
    assert!(a.ui.html_view.as_ref().is_some_and(|v| !v.drawn()));
    assert!(a.ui.text_selection.shown().into_iter().next().is_some(), "the text is drawn");
    let _ = std::fs::remove_dir_all(dir);
}
