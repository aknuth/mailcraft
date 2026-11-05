use super::*;

const MAIL: &str = r##"<html><body style="margin:0;font-family:sans-serif">
  <table width="100%" cellpadding="0" cellspacing="0" bgcolor="#F5F3EF"><tr>
    <td><img src="cid:logo@shop.example" width="120" height="30" alt="Shop"></td>
    <td align="right" style="width:100%"><a href="https://shop.example/orders" style="display:inline-block;padding:10px">Your Orders</a></td>
  </tr></table>
  <h1>Your package was shipped!</h1>
  <p>Hello <img src="https://tracker.example/open.gif" width="1" height="1"></p>
  <div style="background:url(https://shop.example/bg.png);border-radius:16px;padding:16px">Box</div>
</body></html>"##;

#[test]
fn lays_out_and_paints() {
    let mut p = Page::new(MAIL, 600, 1.0, false).unwrap();
    assert!(p.height() > 100.0 && p.height() < 1000.0, "{}", p.height());
    let (w, h, px) = p.paint(0.0, 200.0).unwrap();
    assert_eq!((w, h, px.len()), (600, 200, 600 * 200 * 4));
    // The header's background is drawn at the top left.
    assert_eq!(&px[..3], &[0xF5, 0xF3, 0xEF], "the header's colour");
    // At twice the scale, twice the pixels.
    p.set_size(600, 2.0).unwrap();
    let (w, h, _) = p.paint(0.0, 100.0).unwrap();
    assert_eq!((w, h), (1200, 200));
}

/// Nothing is loaded: pictures and style sheets are only asked for, and given by the caller.
#[test]
fn pictures_are_asked_for_never_fetched() {
    let mut p = Page::new(MAIL, 600, 1.0, false).unwrap();
    let wanted = p.wanted();
    for u in ["cid:logo@shop.example", "https://tracker.example/open.gif", "https://shop.example/bg.png"] {
        assert!(wanted.iter().any(|w| w == u), "{u} in {wanted:?}");
    }
    // A picture given is drawn; asked for no more.
    let png: &[u8] = include_bytes!("tests/red.png");
    p.give("cid:logo@shop.example", png.to_vec()).unwrap();
    assert!(!p.wanted().iter().any(|w| w.starts_with("cid:")));
    let (_, _, px) = p.paint(0.0, 40.0).unwrap();
    let at = (10 * 600 + 10) * 4;
    assert_eq!(&px[at..at + 3], &[255, 0, 0], "the logo is drawn");
    p.refuse("https://tracker.example/open.gif");
    assert!(!p.wanted().iter().any(|w| w.contains("tracker")));
}

#[test]
fn finds_links() {
    let p = Page::new(MAIL, 600, 1.0, false).unwrap();
    // "Your Orders" sits at the right of the header.
    let found = (0..60).find_map(|i| p.link_at(600.0 - 10.0 * i as f32, 15.0));
    assert_eq!(found.as_deref(), Some("https://shop.example/orders"));
    assert_eq!(p.link_at(300.0, p.height() - 5.0), None);
}

/// Deep nesting is refused before anything recurses; everything else, cut anywhere, never panics.
#[test]
fn hostile_markup() {
    let deep = format!("{}x{}", "<div>".repeat(5000), "</div>".repeat(5000));
    assert!(matches!(Page::new(&deep, 600, 1.0, false), Err(Refused::TooDeep(_))));
    let tables = "<table><tr><td>".repeat(3000);
    assert!(matches!(Page::new(&tables, 600, 1.0, false), Err(Refused::TooDeep(_))));
    for (i, _) in MAIL.char_indices().step_by(7) {
        if let Ok(mut p) = Page::new(&MAIL[..i], 300, 1.0, false) {
            let _ = p.paint(0.0, 50.0);
            let _ = p.link_at(10.0, 10.0);
        }
    }
    for odd in [
        "<div style='width:99999999999px;height:1e30px;padding:1e30px'>x</div>",
        "<p style='font-size:1e38px'>big</p>",
        "<div style='width:calc(1px/0)'>n</div>",
        "<table><tr><td colspan=99999999 rowspan=65535>a</td></tr></table>",
        "\0\u{feff}<\0p>",
    ] {
        let mut p = Page::new(odd, 600, 1.0, false).unwrap();
        let _ = p.paint(0.0, p.height().min(500.0));
    }
}

/// Every part of a tall page is drawn, not only its top (the second tile came out empty).
#[test]
fn paints_every_part_of_a_tall_page() {
    let html: String = (0..200).map(|i| format!("<p style='margin:0;line-height:20px'>Line {i}</p>")).collect();
    for scale in [1.0, 1.5, 2.0] {
        let mut p = Page::new(&html, 400, scale, false).unwrap();
        assert!(p.height() > 3900.0, "{}", p.height());
        for top in [0.0, 512.0, 1024.0, 3000.0] {
            let (_, _, px) = p.paint(top, 512.0).unwrap();
            let inked = px.chunks(4).filter(|c| c[3] > 0 && (c[0] < 128)).count();
            assert!(inked > 100, "scale {scale}, from {top}: {inked} dark pixels");
        }
    }
}

/// A link far down is found where it is, after parts below the top were painted.
#[test]
fn links_far_down_after_painting() {
    let mut html: String = (0..150).map(|i| format!("<p style='margin:0;line-height:20px'>Line {i}</p>")).collect();
    html.push_str("<p style='margin:0;line-height:20px'><a href='https://shop.example/far'>Far link</a></p>");
    let mut p = Page::new(&html, 400, 1.5, false).unwrap();
    let _ = p.paint(2560.0, 512.0).unwrap();
    assert_eq!(p.link_at(20.0, 3010.0).as_deref(), Some("https://shop.example/far"));
    assert_eq!(p.link_at(20.0, 10.0), None);
}
