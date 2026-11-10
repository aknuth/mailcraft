//! Printing with the desktop's own print dialog on Linux and the BSDs: the message, as a PDF, is
//! handed to the freedesktop portal (`org.freedesktop.portal.Print`), whose dialog chooses the
//! printer and its settings and prints it. On Windows MailCraft's own print dialog chooses the
//! printer and the copies, and the PDF is printed by Windows' own PDF renderer
//! (`Windows.Data.Pdf`, through `winprint`). Without either (and on macOS) the message opens as a
//! PDF, to print from the viewer.

/// Whether the desktop has a print portal (asked once, giving up after 2 s).
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn available() -> bool {
    match portal::version() {
        Ok(v) => v >= 1,
        Err(e) => {
            log::info!("print: no desktop print portal ({e}): the PDF opens in the viewer");
            false
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(crate) fn available() -> bool {
    false
}

/// Print a PDF through the portal's dialog. The PDF is written to a file only the portal gets
/// (its name is gone before the dialog opens); the dialog runs on a thread of its own, so the
/// window isn't held while it's open, and what goes wrong there is logged.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn print(title: &str, pdf: &[u8]) -> Result<(), String> {
    let file = portal::unnamed_file(pdf).map_err(|e| format!("the PDF couldn't be written for printing ({e})"))?;
    let title = title.to_string();
    std::thread::Builder::new()
        .name("print".into())
        .spawn(move || {
            if let Err(e) = portal::print(&title, &file) {
                log::info!("print: the desktop's print dialog failed: {e}");
            }
        })
        .map(|_| ())
        .map_err(|e| format!("the print dialog couldn't be opened ({e})"))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(crate) fn print(_: &str, _: &[u8]) -> Result<(), String> {
    Err("this system has no print dialog MailCraft can use".into())
}

/// The printers to choose from, the user's default first (Windows; elsewhere none).
#[cfg(windows)]
pub(crate) fn printers() -> Vec<String> {
    use winprint::printer::PrinterDevice;
    let mut names: Vec<String> = match PrinterDevice::all() {
        Ok(all) => all.iter().map(|d| d.name().to_string()).collect(),
        Err(e) => {
            log::info!("print: the printers couldn't be listed ({e}): the PDF opens in the viewer");
            return Vec::new();
        }
    };
    let default = PrinterDevice::user_default().ok().flatten().map(|d| d.name().to_string());
    if let Some(i) = default.and_then(|d| names.iter().position(|n| *n == d)) {
        let name = names.remove(i);
        names.insert(0, name);
    }
    names
}

#[cfg(not(windows))]
pub(crate) fn printers() -> Vec<String> {
    Vec::new()
}

/// Print a PDF on `printer`, `copies` times (Windows). The PDF is written to a folder of its own
/// under the temporary folder, named by `title` (what the printer's queue shows), printed on a
/// thread of its own so the window isn't held, and removed once it's printed.
#[cfg(windows)]
pub(crate) fn print_on(printer: &str, copies: u16, title: &str, pdf: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("mailcraft-print-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let name: String =
        title.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).take(80).collect();
    let name = if name.trim().is_empty() { "message".to_string() } else { name.trim().to_string() };
    let path = dir.join(format!("{name}.pdf"));
    std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, pdf)).map_err(|e| format!("the PDF couldn't be written for printing ({e})"))?;
    let printer = printer.to_string();
    std::thread::Builder::new()
        .name("print".into())
        .spawn(move || {
            if let Err(e) = windows_print(&printer, copies, &path) {
                log::info!("print: printing failed: {e}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        })
        .map(|_| ())
        .map_err(|e| format!("the message couldn't be printed ({e})"))
}

#[cfg(windows)]
fn windows_print(printer: &str, copies: u16, path: &std::path::Path) -> Result<(), String> {
    use winprint::printer::{FilePrinter, PrinterDevice, TimedPrinter, WinPdfPrinter};
    use winprint::ticket::{Copies, PrintTicketBuilder};
    let device = PrinterDevice::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|d| d.name() == printer)
        .ok_or_else(|| format!("the printer {printer} is gone"))?;
    let mut ticket = PrintTicketBuilder::new(&device).map_err(|e| e.to_string())?;
    ticket.merge(Copies(copies.max(1))).map_err(|e| e.to_string())?;
    let ticket = ticket.build().map_err(|e| e.to_string())?;
    let mut pdf = WinPdfPrinter::new(device);
    // A driver that never answers would hold the thread (and the PDF's folder) for good.
    pdf.set_timeout(Some(std::time::Duration::from_secs(300)));
    pdf.print(path, ticket).map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub(crate) fn print_on(_: &str, _: u16, _: &str, _: &[u8]) -> Result<(), String> {
    Err("this system has no printers MailCraft can use".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use zbus::zvariant::{Fd, OwnedValue, Value};

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const PRINT: &str = "org.freedesktop.portal.Print";

    /// Each print's request gets a token of its own.
    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn connect() -> zbus::Result<zbus::blocking::Connection> {
        // A portal that takes a call and never answers would hold the caller for good.
        zbus::blocking::connection::Builder::session()?.method_timeout(Duration::from_secs(2)).build()
    }

    /// The print portal's version (an error: there's none).
    pub(super) fn version() -> zbus::Result<u32> {
        let conn = connect()?;
        let reply = conn.call_method(Some(PORTAL), PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(PRINT, "version"))?;
        let body = reply.body();
        let v: OwnedValue = body.deserialize()?;
        match &*v {
            Value::U32(n) => Ok(*n),
            Value::Value(inner) => match **inner {
                Value::U32(n) => Ok(n),
                _ => Err(zbus::Error::Failure("the print portal's version isn't a number".into())),
            },
            _ => Err(zbus::Error::Failure("the print portal's version isn't a number".into())),
        }
    }

    /// The PDF in a file of the temporary folder, opened for reading, its name removed at once:
    /// nothing is left behind, and only who's handed the file can read it.
    pub(super) fn unnamed_file(pdf: &[u8]) -> std::io::Result<File> {
        let path = std::env::temp_dir().join(format!("mailcraft-print-{}-{}.pdf", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let written = (|| {
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
            f.write_all(pdf)?;
            f.sync_all()?;
            File::open(&path)
        })();
        let _ = std::fs::remove_file(&path);
        written
    }

    /// Ask the portal to print the file: it shows its dialog, then prints with what's chosen
    /// there. Returns when the dialog is closed (printed or cancelled).
    pub(super) fn print(title: &str, file: &File) -> zbus::Result<()> {
        let conn = connect()?;
        let token = format!("mailcraft_print_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        // The request's answer comes as a signal on a path the token names (subscribed to
        // before the call, so it can't be missed).
        let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
        let request = format!("{PATH}/request/{sender}/{token}");
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(PORTAL)?
            .interface("org.freedesktop.portal.Request")?
            .member("Response")?
            .path(request.as_str())?
            .build();
        let answers = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(1))?;
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("modal", Value::from(true));
        conn.call_method(Some(PORTAL), PATH, Some(PRINT), "Print", &("", title, Fd::from(file), options))?;
        if let Some(msg) = answers.into_iter().next() {
            let msg = msg?;
            let body = msg.body();
            let (code, _): (u32, HashMap<String, OwnedValue>) = body.deserialize()?;
            // 0: printed; 1: cancelled; 2: ended some other way.
            if code == 2 {
                return Err(zbus::Error::Failure("the print dialog ended without printing".into()));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The file handed to the portal holds the PDF, and no name is left behind.
        #[test]
        fn the_pdf_is_handed_over_without_a_name() {
            let before = |dir: &std::path::Path| {
                std::fs::read_dir(dir)
                    .map(|d| d.filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("mailcraft-print-")).count())
                    .unwrap_or(0)
            };
            let dir = std::env::temp_dir();
            let had = before(&dir);
            let mut f = unnamed_file(b"%PDF-1.7 test").unwrap();
            let mut read = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut read).unwrap();
            assert_eq!(read, b"%PDF-1.7 test");
            assert_eq!(before(&dir), had);
        }

        /// Against the desktop's portal, run by hand (`MAILCRAFT_PORTAL=1`): it has a print
        /// portal, and its dialog opens for a one-page PDF (print it or cancel it).
        #[test]
        #[ignore]
        fn the_desktops_print_dialog_opens() {
            if std::env::var_os("MAILCRAFT_PORTAL").is_none() {
                eprintln!("MAILCRAFT_PORTAL isn't set: skipped");
                return;
            }
            assert!(version().unwrap() >= 1);
            let pdf = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
            let file = unnamed_file(pdf).unwrap();
            print("MailCraft test page", &file).unwrap();
        }
    }
}
