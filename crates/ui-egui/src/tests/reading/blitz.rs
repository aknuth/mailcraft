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
    // The lane says what's under the pointer; the frame after shows it.
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
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

fn button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

const PROSE: &str = r#"<div style="margin:0;font-size:16px;line-height:20px">
<p style="margin:0 0 20px">First paragraph here.</p><p style="margin:0">Second one, with words.</p></div>"#;

#[test]
fn its_text_is_selected_by_a_drag_and_copied() {
    let (mut a, dir, ctx) = opened(html_message(PROSE), "blitz-select-drag");
    let area = a.ui.html_view.as_ref().and_then(|v| v.rect()).expect("drawn");
    // The body's margin (8 pixels, Blitz's own) before the text.
    let start = area.left_top() + egui::vec2(9.0, 18.0);
    let end = area.left_top() + egui::vec2(60.0, 58.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(start)]);
    frame(&ctx, &mut a, vec![button(start, true)]);
    for i in 1..=5 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(start + (end - start) * (i as f32 / 5.0))]);
    }
    frame(&ctx, &mut a, vec![button(end, false)]);
    let view = a.ui.html_view.as_ref().unwrap();
    assert!(view.active(), "the text has the keyboard");
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.contains("paragraph here.\nSecond"), "{got:?}");
    // Ctrl+A takes it all; Escape lets go.
    // Selected on the lane; copied the frame after.
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let all = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(all, "First paragraph here.\nSecond one, with words.");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(a.ui.html_view.as_ref().and_then(|v| v.selected()), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_double_click_takes_a_word_and_a_press_elsewhere_the_keyboard() {
    let (mut a, dir, ctx) = opened(html_message(PROSE), "blitz-select-word");
    let area = a.ui.html_view.as_ref().and_then(|v| v.rect()).expect("drawn");
    let at = area.left_top() + egui::vec2(60.0, 18.0); // in "paragraph"
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
    frame(&ctx, &mut a, vec![button(at, true), button(at, false)]);
    frame(&ctx, &mut a, vec![button(at, true), button(at, false)]);
    let view = a.ui.html_view.as_ref().unwrap();
    assert_eq!(view.selected().as_deref(), Some("paragraph"));
    // A press in the message list: the text's keys go (Ctrl+A is the list's again).
    let list = egui::pos2(area.left() - 200.0, area.top() + 40.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(list), button(list, true), button(list, false)]);
    assert!(!a.ui.html_view.as_ref().unwrap().active());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_drag_past_the_bottom_scrolls_and_keeps_selecting() {
    let body: String = (0..200).map(|i| format!("<p style=\"margin:0;line-height:20px\">Line {i:03}</p>")).collect();
    let (mut a, dir, ctx) = opened(html_message(&body), "blitz-select-scroll");
    let area = a.ui.html_view.as_ref().and_then(|v| v.rect()).expect("drawn");
    let start = area.left_top() + egui::vec2(9.0, 18.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(start)]);
    frame(&ctx, &mut a, vec![button(start, true)]);
    let below = egui::pos2(start.x + 40.0, 860.0);
    for _ in 0..40 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(below)]);
    }
    let moved = a.ui.html_view.as_ref().and_then(|v| v.rect()).expect("drawn");
    assert!(moved.top() < area.top() - 300.0, "scrolled: {} from {}", moved.top(), area.top());
    frame(&ctx, &mut a, vec![button(below, false)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.starts_with("Line 000") && got.contains("Line 040"), "{} lines", got.lines().count());
    let _ = std::fs::remove_dir_all(dir);
}

/// Services that keep the work handed to them, with its lane, for the test to run.
struct Held(std::sync::Arc<std::sync::Mutex<Vec<(crate::Lane, crate::Work)>>>);

impl crate::Services for Held {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn background(&mut self, lane: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        self.0.lock().unwrap().push((lane, work));
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

#[test]
fn the_page_is_laid_out_on_the_reading_lane_while_the_window_draws() {
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let dir = std::env::temp_dir().join(format!("mailcraft-ui-blitz-lane-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let mut a = MailApp::new(s, Box::new(NullBackend), Box::new(Held(held.clone())));
    a.ui.blitz = true;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    a.session.apply(MailEvent::Body { key, raw: html_message(PROSE) });
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    // Handed to the reading lane, once (not again while it's at work); a spinner meanwhile.
    let work: Vec<_> = held.lock().unwrap().drain(..).collect();
    assert_eq!(work.len(), 1, "one pump handed over");
    assert_eq!(work[0].0, crate::Lane::Reading);
    assert!(a.ui.html_view.as_ref().is_some_and(|v| v.rect().is_none()), "not laid out yet");
    // Each piece of work on a thread of its own (as a test's lanes do): the page is laid out
    // again there from its HTML, and comes back.
    let mut runs = work.into_iter().map(|(_, w)| w).collect::<Vec<_>>();
    for _ in 0..10 {
        for w in runs.drain(..) {
            std::thread::spawn(w.run).join().unwrap();
        }
        frame(&ctx, &mut a, vec![]);
        runs = held.lock().unwrap().drain(..).map(|(_, w)| w).collect();
        if runs.is_empty() && a.ui.html_view.as_ref().is_some_and(|v| v.drawn()) {
            break;
        }
    }
    let view = a.ui.html_view.as_ref().unwrap();
    assert!(view.drawn() && view.rect().is_some(), "laid out and painted on the lane");
    let _ = std::fs::remove_dir_all(dir);
}
