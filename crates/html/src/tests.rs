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
    p.refuse("https://tracker.example/open.gif").unwrap();
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

#[test]
fn a_web_style_sheet_does_not_hold_it_up() {
    // Blitz waits for the style sheets linked in `<head>`; a mail's are never loaded.
    let html = r#"<html><head><link href="https://fonts.example/css2?family=Lato" rel="stylesheet">
        <link rel="StyleSheet" href="https://shop.example/mail.css"></head>
        <body><p style="margin:0;height:300px">Hello</p><img src="relative.png" width="10" height="10"></body></html>"#;
    let p = Page::new(html, 600, 1.0, false).unwrap();
    assert!(p.height() >= 300.0, "laid out: {}", p.height());
    assert!(!p.wanted().iter().any(|w| w.contains(".example/")), "{:?}", p.wanted());
}

#[test]
fn a_cell_shown_another_way_is_still_drawn() {
    // Mail templates stack their cells on phones with `display: inline-block` (or block);
    // Blitz on its own leaves such a cell out with all it holds.
    let tall = |display: &str| {
        let html = format!(
            r#"<body style="margin:0"><table width="300" cellpadding="0" cellspacing="0"><tr>
            <td style="overflow: hidden; display:{display};"><div style="height:200px">Hi</div></td>
            </tr></table></body>"#
        );
        Page::new(&html, 600, 1.0, false).unwrap().height()
    };
    for display in ["inline-block", " inline-block", "block", "BLOCK", "inline", "flex", "table-cell"] {
        assert!(tall(display) >= 200.0, "display:{display} drawn {}", tall(display));
    }
    assert!(tall("none") < 50.0, "a hidden cell stays hidden: {}", tall("none"));
}

const PROSE: &str = r#"<body style="margin:0;font-size:16px;line-height:20px">
<p style="margin:0 0 20px">First paragraph here.</p>
<p style="margin:0">Second one, with words.</p></body>"#;

#[test]
fn selects_by_dragging_across_blocks() {
    let mut p = Page::new(PROSE, 600, 1.0, false).unwrap();
    assert!(p.text_at(5.0, 10.0));
    assert!(!p.text_at(5.0, 30.0), "the gap between the paragraphs");
    let (_, _, plain) = p.paint(0.0, 60.0).unwrap();
    p.select_from(1.0, 10.0);
    assert!(!p.has_selection(), "a press alone selects nothing");
    assert!(p.select_to(70.0, 50.0));
    let got = p.selected_text().unwrap();
    assert!(got.starts_with("First paragraph here."), "{got:?}");
    assert!(got.contains("\nSecond"), "a block to a line: {got:?}");
    let (_, _, shaded) = p.paint(0.0, 60.0).unwrap();
    assert_ne!(plain, shaded, "the selection is painted");
    // Past a line's end, in the margin: to the line's end.
    p.select_from(1.0, 10.0);
    assert!(p.select_to(590.0, 10.0));
    assert_eq!(p.selected_text().as_deref(), Some("First paragraph here."));
    // A press beside the text lets go; a drag then selects nothing.
    p.select_from(1.0, 30.0);
    assert!(!p.has_selection());
    assert!(!p.select_to(70.0, 50.0));
}

#[test]
fn selects_a_word_a_block_and_everything() {
    let mut p = Page::new(PROSE, 600, 1.0, false).unwrap();
    // "paragraph" begins after "First " (about 40 pixels in at 16px).
    let x = (0..60).map(|i| i as f32 * 2.0).find(|x| {
        p.select_around(*x, 10.0, false);
        p.selected_text().as_deref() == Some("paragraph")
    });
    assert!(x.is_some(), "a word: {:?}", p.selected_text());
    assert!(p.select_around(5.0, 50.0, true));
    assert_eq!(p.selected_text().as_deref(), Some("Second one, with words."));
    assert!(p.select_all());
    assert_eq!(p.selected_text().as_deref(), Some("First paragraph here.\nSecond one, with words."));
    assert!(p.clear_selection());
    assert_eq!(p.selected_text(), None);
    assert!(!p.select_around(5.0, 30.0, false), "no text there");
}

#[test]
fn what_isnt_a_cell_in_a_table_gets_a_cell_of_its_own() {
    // eBay's: a `display: table` box holding a cell (the picture) and a block (the details)
    // side by side; Blitz on its own left the block out.
    let html = r#"<body style="margin:0;font-size:16px;line-height:20px">
        <div style="display:table;width:600px">
          <div style="display:table-cell;width:200px"><div style="height:100px;background:#f00"></div></div>
          <div style="display:none">gutter</div>
          <div style="display:block;width:300px">Item ID: 1987</div>
        </div>
        <table width="600"><tr><td>A cell</td><div>moved out by the parser</div></tr></table>
        <div style="display:table-row"><span>a row's text</span></div></body>"#;
    let mut p = Page::new(html, 600, 1.0, false).unwrap();
    // The details sit right of the picture, at its top.
    assert!(p.text_at(210.0, 10.0), "the block beside the cell is drawn");
    assert!(!p.text_at(10.0, 10.0), "the picture is at the left");
    assert!(p.select_all());
    let all = p.selected_text().unwrap_or_default();
    assert!(all.contains("Item ID: 1987") && all.contains("a row's text"), "{all:?}");
    assert!(!all.contains("gutter"), "hidden stays hidden: {all:?}");
}

#[test]
fn a_picture_keeps_its_height_beside_a_min_width() {
    // Shopify's spacer: as wide as the mail, no height.
    let png: &[u8] = include_bytes!("tests/red.png");
    let html = r#"<body style="margin:0"><img src="https://shop.example/spacer.png" height="1" style="min-width: 600px; height: 0;"><p style="margin:0;line-height:20px">after</p></body>"#;
    let mut p = Page::new(html, 700, 1.0, false).unwrap();
    p.give("https://shop.example/spacer.png", png.to_vec()).unwrap();
    assert!(p.update().unwrap(), "laid out again");
    assert!(p.height() < 40.0, "not 600 pixels tall: {}", p.height());
}

#[test]
fn a_picture_in_an_inline_block_fits_its_column() {
    // eBay's product pictures: `width: 100%`, in a link that's an inline block, in a narrow
    // column; Blitz drew them at their own size, over the next column.
    let png: &[u8] = include_bytes!("tests/red.png"); // 8 by 8
    let html = r#"<body style="margin:0"><div style="width:4px;font-size:0;line-height:0"><a href="https://shop.example/1" style="display:inline-block"><img src="https://shop.example/1.png" style="width:100%;display:block"></a></div></body>"#;
    let mut p = Page::new(html, 600, 1.0, false).unwrap();
    p.give("https://shop.example/1.png", png.to_vec()).unwrap();
    p.update().unwrap();
    assert!(p.height() <= 6.0, "4 pixels wide, so 4 tall: {}", p.height());
}
