//! Save As = Export: the format list, the naming rule and the delivery per platform.

use crate::export::{Delivery, ExportError, ExportFormat, ExportList, MAX_FILE_NAME_BYTES, Picked, Runtime, SaveAsPlan, mime_for_ext};

/// A WordCraft-like list: one id (`wordml2003`) whose extension is shared with another format.
const WORD: ExportList = ExportList::new(&[
    ExportFormat::new("pdf", "PDF document (*.pdf)", "pdf", "application/pdf"),
    ExportFormat::new("docx", "Word document (*.docx)", "docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ExportFormat::new("xml", "Word XML Document (*.xml)", "xml", "application/xml"),
    ExportFormat::new("wordml2003", "Word 2003 XML Document (*.xml)", "xml", "application/xml"),
    ExportFormat::new("txt", "Plain text (*.txt)", "txt", "text/plain"),
    ExportFormat::new("png", "Page image (*.png)", "png", "image/png").detail("The current page as a picture"),
])
.replacing(&["doc", "htm"]);

/// A DeckCraft-like list with a many-file format.
const DECK: ExportList = ExportList::new(&[
    ExportFormat::new(
        "pptx",
        "PowerPoint Presentation (*.pptx)",
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ExportFormat::new("png", "PNG Picture (*.png)", "png", "image/png"),
    ExportFormat::new("pngAll", "PNG Pictures, all slides (*.png)", "png", "image/png").many(),
]);

/// A PdfCraft-like list: one image format accepting several extensions.
const PDF: ExportList = ExportList::new(&[
    ExportFormat::new("pdf", "PDF (*.pdf)", "pdf", "application/pdf"),
    ExportFormat::new("image", "Image (*.png, *.jpg, *.tif)", "png", "image/png").also_exts(&["jpg", "jpeg", "tif", "tiff"]).many(),
    ExportFormat::new("text", "Text (*.txt)", "txt", "text/plain"),
]);

fn title(list: &ExportList, t: &str, id: &str) -> String {
    match list.file_name_from_title(t, "Document", id) {
        Ok(n) => n,
        Err(e) => format!("ERR {e}"),
    }
}

fn picked(list: &ExportList, p: &str, id: &str) -> (String, bool) {
    match list.picked_path(p, id) {
        Ok(Picked { path, renamed }) => (path, renamed),
        Err(e) => (format!("ERR {e}"), false),
    }
}

fn name(list: &ExportList, doc: &str, id: &str) -> String {
    match list.file_name(doc, "Document", id) {
        Ok(n) => n,
        Err(e) => format!("ERR {e}"),
    }
}

// ---- the list ----

#[test]
fn list_keeps_the_apps_order_and_finds_by_id() {
    let ids: Vec<&str> = WORD.formats().iter().map(|f| f.id).collect();
    assert_eq!(ids, ["pdf", "docx", "xml", "wordml2003", "txt", "png"]);
    assert_eq!(WORD.get("wordml2003").map(|f| f.ext), Some("xml"));
    assert_eq!(WORD.get("png").map(|f| f.detail), Some("The current page as a picture"));
    assert_eq!(WORD.get("pdf").map(|f| f.detail), Some(""));
    assert!(WORD.get("PDF").is_none(), "ids are exact");
    assert!(WORD.get("gif").is_none());
}

#[test]
fn for_ext_is_the_first_format_writing_it_any_case() {
    assert_eq!(WORD.for_ext("XML").map(|f| f.id), Some("xml"));
    assert_eq!(WORD.for_ext(".pdf").map(|f| f.id), Some("pdf"), "a leading dot is ignored");
    assert_eq!(DECK.for_ext("png").map(|f| f.id), Some("png"));
    assert!(WORD.for_ext("odt").is_none());
    assert!(WORD.for_ext("").is_none());
}

#[test]
fn many_is_off_unless_set() {
    assert!(!DECK.get("png").is_some_and(|f| f.many));
    assert!(DECK.get("pngAll").is_some_and(|f| f.many));
}

#[test]
fn document_exts_are_the_lists_plus_the_extra_ones() {
    for ext in ["pdf", "DOCX", "xml", "txt", "png", "doc", "HTM"] {
        assert!(WORD.is_document_ext(ext), "{ext}");
    }
    for ext in ["v2", "", "odt", "final"] {
        assert!(!WORD.is_document_ext(ext), "{ext}");
    }
}

#[test]
fn check_accepts_good_lists() {
    assert_eq!(WORD.check(), Ok(()));
    assert_eq!(DECK.check(), Ok(()));
}

#[test]
fn check_refuses_bad_lists() {
    let bad = |list: ExportList| matches!(list.check(), Err(ExportError::BadList(_)));
    assert!(bad(ExportList::new(const { &[] })), "empty");
    assert!(
        bad(ExportList::new(const { &[ExportFormat::new("a", "A", "pdf", "application/pdf"), ExportFormat::new("a", "B", "txt", "text/plain")] })),
        "dup id"
    );
    assert!(bad(ExportList::new(const { &[ExportFormat::new("", "A", "pdf", "application/pdf")] })), "empty id");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "", "pdf", "application/pdf")] })), "empty label");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "A", "", "application/pdf")] })), "empty ext");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "A", ".pdf", "application/pdf")] })), "dotted ext");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "A", "PDF", "application/pdf")] })), "upper-case ext");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "A", "p/f", "application/pdf")] })), "separator in ext");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "A", "pdf", "")] })), "empty mime");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "A", "pdf", "pdf")] })), "mime without a slash");
    assert!(bad(ExportList::new(const { &[ExportFormat::new("a", "A", "pdf", "application/pdf")] }).replacing(&[".doc"])), "dotted extra ext");
}

// ---- the naming rule ----

#[test]
fn name_takes_the_chosen_types_extension() {
    assert_eq!(name(&WORD, "notes.docx", "pdf"), "notes.pdf");
    assert_eq!(name(&WORD, "notes.docx", "txt"), "notes.txt");
    assert_eq!(name(&WORD, "notes.docx", "docx"), "notes.docx");
    assert_eq!(name(&WORD, "notes.DOCX", "docx"), "notes.DOCX", "the same type in another case is kept");
    assert_eq!(name(&WORD, "notes.docx", "wordml2003"), "notes.xml", "the format's ext, not its id");
    assert_eq!(name(&WORD, "old.doc", "docx"), "old.docx", "an extra ext gives way");
    assert_eq!(name(&WORD, "page.HTM", "pdf"), "page.pdf");
}

#[test]
fn name_adds_the_extension_to_other_names() {
    assert_eq!(name(&WORD, "notes", "pdf"), "notes.pdf");
    assert_eq!(name(&WORD, "notes.v2", "pdf"), "notes.v2.pdf");
    assert_eq!(name(&WORD, "a.b.c", "txt"), "a.b.c.txt");
    assert_eq!(name(&WORD, "report.odt", "pdf"), "report.odt.pdf", "odt is not in this list");
    assert_eq!(name(&WORD, "a.", "pdf"), "a.pdf", "a trailing dot is dropped");
    assert_eq!(name(&WORD, "a..", "pdf"), "a.pdf");
    assert_eq!(name(&WORD, ".notes", "pdf"), "notes.pdf", "no hidden files: leading dots go");
}

#[test]
fn name_drops_folders_and_control_characters() {
    assert_eq!(name(&WORD, "/home/me/Docs/notes.docx", "pdf"), "notes.pdf");
    assert_eq!(name(&WORD, r"C:\Users\me\notes.docx", "pdf"), "notes.pdf");
    assert_eq!(name(&WORD, "../../etc/passwd", "txt"), "passwd.txt");
    assert_eq!(name(&WORD, "  no\ttes\u{7}.docx  ", "pdf"), "notes.pdf");
    assert_eq!(name(&WORD, "über café.docx", "pdf"), "über café.pdf");
}

#[test]
fn name_falls_back_when_nothing_is_left() {
    for doc in ["", "   ", "/", "dir/", ".", "..", "a/..", "\u{1}"] {
        assert_eq!(name(&WORD, doc, "pdf"), "Document.pdf", "{doc:?}");
    }
    assert_eq!(WORD.file_name("", "", "pdf"), Ok("Untitled.pdf".to_string()), "an empty fallback too");
    assert_eq!(WORD.file_name("", "a/b", "pdf"), Ok("b.pdf".to_string()), "the fallback is cleaned like a name");
}

#[test]
fn name_is_capped_and_keeps_its_extension() {
    let long = "x".repeat(400);
    let n = name(&WORD, &format!("{long}.docx"), "pdf");
    assert_eq!(n.len(), MAX_FILE_NAME_BYTES);
    assert!(n.ends_with("x.pdf"));
    let n = name(&WORD, &format!("{long}.pdf"), "pdf");
    assert_eq!(n.len(), MAX_FILE_NAME_BYTES, "a name already ending in the extension is capped too");
    assert!(n.ends_with("x.pdf"));
    // Bytes, not characters: 255 bytes is the limit on ext4, APFS and NTFS-as-UTF-8 alike.
    let wide = "é".repeat(300);
    let n = name(&WORD, &wide, "txt");
    assert!(n.len() <= MAX_FILE_NAME_BYTES && n.len() >= MAX_FILE_NAME_BYTES - 1, "{}", n.len());
    assert!(n.ends_with("é.txt"));
    let cjk = "漢".repeat(200);
    let n = name(&WORD, &cjk, "pdf");
    assert!(n.len() <= MAX_FILE_NAME_BYTES && n.ends_with("漢.pdf"));
    // A cut that lands before a space leaves no trailing space before the extension.
    let n = name(&WORD, &format!("{} tail", "x".repeat(250)), "pdf");
    assert!(n.len() <= MAX_FILE_NAME_BYTES && n.ends_with("x.pdf"), "{n}");
}

#[test]
fn name_of_an_unknown_format_is_an_error() {
    assert_eq!(WORD.file_name("a.docx", "Document", "gif"), Err(ExportError::UnknownFormat("gif".into())));
}

// ---- a path picked in the desktop dialog ----

#[test]
fn picked_path_with_the_right_extension_is_kept() {
    assert_eq!(WORD.picked_path("/home/me/notes.pdf", "pdf"), Ok(Picked { path: "/home/me/notes.pdf".into(), renamed: false }));
    assert_eq!(WORD.picked_path(r"C:\me\NOTES.PDF", "pdf"), Ok(Picked { path: r"C:\me\NOTES.PDF".into(), renamed: false }));
}

#[test]
fn picked_path_gets_the_extension_and_says_it_was_renamed() {
    assert_eq!(WORD.picked_path("/home/me/notes", "pdf"), Ok(Picked { path: "/home/me/notes.pdf".into(), renamed: true }));
    assert_eq!(
        WORD.picked_path("/home/me/notes.docx", "wordml2003"),
        Ok(Picked { path: "/home/me/notes.docx.xml".into(), renamed: true }),
        "a typed extension of another format is the user's name: the chosen one is added"
    );
    assert_eq!(WORD.picked_path(r"C:\me\notes.v2", "txt"), Ok(Picked { path: r"C:\me\notes.v2.txt".into(), renamed: true }));
    assert_eq!(WORD.picked_path("notes", "pdf"), Ok(Picked { path: "notes.pdf".into(), renamed: true }), "no folder");
    assert_eq!(WORD.picked_path("/home/me.d/notes", "pdf"), Ok(Picked { path: "/home/me.d/notes.pdf".into(), renamed: true }), "a dot in a folder");
}

#[test]
fn picked_path_without_a_name_or_format_is_an_error() {
    assert_eq!(WORD.picked_path("/home/me/", "pdf"), Err(ExportError::NoFileName));
    assert_eq!(WORD.picked_path("", "pdf"), Err(ExportError::NoFileName));
    assert_eq!(WORD.picked_path("/a/b.pdf", "gif"), Err(ExportError::UnknownFormat("gif".into())));
}

// ---- delivery ----

#[test]
fn save_as_downloads_on_the_web_framed_or_not_and_asks_on_the_desktop() {
    assert_eq!(Runtime::Desktop.save_as_delivery(), Delivery::SaveDialog);
    assert_eq!(Runtime::Web.save_as_delivery(), Delivery::Download);
    assert_eq!(Runtime::WebHosted.save_as_delivery(), Delivery::Download, "never the host");
    assert_eq!(Runtime::web(true), Runtime::WebHosted);
    assert_eq!(Runtime::web(false), Runtime::Web);
}

#[test]
fn only_plain_save_on_a_hosted_page_goes_to_the_host() {
    assert!(Runtime::WebHosted.save_goes_to_host());
    assert!(!Runtime::Web.save_goes_to_host());
    assert!(!Runtime::Desktop.save_goes_to_host());
}

#[test]
fn plan_puts_name_format_and_delivery_together() {
    let plan = WORD.plan(Runtime::WebHosted, "Q3 report.docx", "Document", "pdf");
    assert_eq!(plan, Ok(SaveAsPlan { format: WORD.formats()[0], file_name: "Q3 report.pdf".into(), delivery: Delivery::Download }));
    let Ok(plan) = DECK.plan(Runtime::Desktop, "/d/Deck.pptx", "Presentation", "pngAll") else { panic!("plan") };
    assert_eq!(plan.file_name, "Deck.png");
    assert_eq!(plan.delivery, Delivery::SaveDialog);
    assert!(plan.format.many);
    assert_eq!(plan.format.mime, "image/png");
    assert_eq!(WORD.plan(Runtime::Web, "a", "b", "nope"), Err(ExportError::UnknownFormat("nope".into())));
}

// ---- mime table ----

#[test]
fn mime_for_ext_knows_every_apps_formats() {
    let cases = [
        ("pdf", "application/pdf"),
        ("DOCX", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        ("odt", "application/vnd.oasis.opendocument.text"),
        ("ott", "application/vnd.oasis.opendocument.text-template"),
        ("fodt", "application/vnd.oasis.opendocument.text-flat-xml"),
        ("rtf", "application/rtf"),
        ("html", "text/html"),
        ("htm", "text/html"),
        ("md", "text/markdown"),
        ("tex", "application/x-tex"),
        ("txt", "text/plain"),
        ("csv", "text/csv"),
        ("tsv", "text/tab-separated-values"),
        ("json", "application/json"),
        ("xml", "application/xml"),
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        (".jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("tif", "image/tiff"),
        ("tiff", "image/tiff"),
        ("svg", "image/svg+xml"),
        ("xfdf", "application/vnd.adobe.xfdf"),
        ("fdf", "application/vnd.fdf"),
        ("dotx", "application/vnd.openxmlformats-officedocument.wordprocessingml.template"),
        ("deckcraft", "application/octet-stream"),
        ("", "application/octet-stream"),
    ];
    for (ext, mime) in cases {
        assert_eq!(mime_for_ext(ext), mime, "{ext}");
    }
}

#[test]
fn formats_are_const_and_copy() {
    const F: ExportFormat = ExportFormat::new("csv", "CSV (*.csv)", "csv", "text/csv").detail("d").many();
    let g = F;
    assert_eq!(F, g);
    assert!(g.many);
}

// ---- fix round 1 ----

#[test]
fn title_keeps_slashes_as_text() {
    assert_eq!(title(&WORD, "Budget 2025/26", "pdf"), "Budget 2025-26.pdf");
    assert_eq!(title(&WORD, "Q1/Q2 plan.docx", "pdf"), "Q1-Q2 plan.pdf");
    assert_eq!(title(&WORD, r"AC\DC notes", "txt"), "AC-DC notes.txt");
    assert_eq!(title(&WORD, "", "pdf"), "Document.pdf");
    assert_eq!(title(&WORD, "/", "pdf"), "-.pdf");
    assert_eq!(WORD.file_name_from_title("x", "Document", "gif"), Err(ExportError::UnknownFormat("gif".into())));
}

#[test]
fn names_are_safe_on_windows() {
    assert_eq!(title(&WORD, "Q3: Report?", "pdf"), "Q3_ Report_.pdf");
    assert_eq!(title(&WORD, r#"a<b>c"d|e*f"#, "pdf"), "a_b_c_d_e_f.pdf");
    assert_eq!(name(&WORD, "C:/x/what?.docx", "pdf"), "what_.pdf");
    for (doc, want) in [
        ("CON", "_CON.pdf"),
        ("nul.docx", "_nul.pdf"),
        ("com1", "_com1.pdf"),
        ("LPT9.txt", "_LPT9.pdf"),
        ("aux.tar", "_aux.tar.pdf"),
        ("Prn ", "_Prn.pdf"),
    ] {
        assert_eq!(name(&WORD, doc, "pdf"), want, "{doc:?}");
    }
    for doc in ["CONSOLE", "com10", "lpt", "con-x"] {
        assert!(!name(&WORD, doc, "pdf").starts_with('_'), "{doc:?}");
    }
    assert_eq!(name(&WORD, "report . ", "pdf"), "report.pdf", "trailing dots and spaces go");
}

#[test]
fn spoofing_and_invisible_characters_are_dropped() {
    assert_eq!(name(&WORD, "invoice\u{202E}fdp.docx", "pdf"), "invoicefdp.pdf");
    assert_eq!(name(&WORD, "a\u{200B}b\u{FEFF}\u{2066}c", "pdf"), "abc.pdf");
}

#[test]
fn no_hidden_or_doubled_names() {
    for doc in [".pdf", "...", ".docx", "..docx", " . "] {
        assert_eq!(name(&WORD, doc, "pdf"), "Document.pdf", "{doc:?}");
    }
    assert_eq!(name(&WORD, "report.pdf.", "pdf"), "report.pdf");
    assert_eq!(name(&WORD, "notes.docx.", "pdf"), "notes.pdf");
    assert_eq!(name(&WORD, "notes .docx", "pdf"), "notes.pdf");
    assert_eq!(name(&WORD, "a..docx", "pdf"), "a.pdf");
}

#[test]
fn numbered_names_put_the_number_before_the_extension() {
    assert_eq!(DECK.numbered_file_name("/d/Deck.pptx", "Presentation", "pngAll", 3), Ok("Deck-3.png".to_string()));
    assert_eq!(DECK.numbered_file_name("", "Presentation", "pngAll", 12), Ok("Presentation-12.png".to_string()));
    assert_eq!(DECK.numbered_file_name("con", "P", "png", 1), Ok("_con-1.png".to_string()));
    let long = "x".repeat(300);
    let Ok(n) = DECK.numbered_file_name(&long, "P", "pngAll", 1234) else { panic!("numbered") };
    assert!(n.len() <= MAX_FILE_NAME_BYTES && n.ends_with("x-1234.png"), "{n}");
    assert_eq!(DECK.numbered_file_name("a", "P", "gif", 1), Err(ExportError::UnknownFormat("gif".into())));
}

#[test]
fn a_format_can_accept_several_extensions() {
    let Some(image) = PDF.get("image") else { panic!("image") };
    assert!(image.accepts("JPG") && image.accepts(".tiff") && image.accepts("png"));
    assert!(!image.accepts("pdf") && !image.accepts(""));
    assert!(PDF.is_document_ext("jpeg"), "also_exts count as document extensions");
    assert_eq!(PDF.for_ext("tif").map(|f| f.id), Some("image"));
    assert_eq!(PDF.file_name("scan.jpeg", "D", "image"), Ok("scan.jpeg".to_string()), "an accepted extension is kept");
    assert_eq!(PDF.file_name("scan.pdf", "D", "image"), Ok("scan.png".to_string()));
    assert_eq!(picked(&PDF, "/p/scan.jpg", "image"), ("/p/scan.jpg".to_string(), false));
    assert_eq!(picked(&PDF, "/p/scan.TIF", "image"), ("/p/scan.TIF".to_string(), false));
    assert_eq!(picked(&PDF, "/p/scan", "image"), ("/p/scan.png".to_string(), true));
    assert_eq!(PDF.check(), Ok(()));
    let bad = ExportList::new(const { &[ExportFormat::new("a", "A", "png", "image/png").also_exts(&[".jpg"])] });
    assert!(matches!(bad.check(), Err(ExportError::BadList(_))), "also_exts follow the ext rule");
}

#[test]
fn check_refuses_two_formats_with_one_label() {
    let list =
        ExportList::new(const { &[ExportFormat::new("a", "Same", "pdf", "application/pdf"), ExportFormat::new("b", "Same", "txt", "text/plain")] });
    assert!(matches!(list.check(), Err(ExportError::BadList(_))));
}

#[test]
fn picked_path_edge_cases() {
    assert_eq!(picked(&WORD, "/home/me/a.", "pdf"), ("/home/me/a.pdf".to_string(), true));
    assert_eq!(picked(&WORD, "/home/me/report.pdf.", "pdf"), ("/home/me/report.pdf".to_string(), true));
    assert_eq!(picked(&WORD, "/home/me/über café", "pdf"), ("/home/me/über café.pdf".to_string(), true));
    assert_eq!(picked(&WORD, r"C:\me/mixed\notes", "txt"), (r"C:\me/mixed\notes.txt".to_string(), true));
    assert_eq!(picked(&WORD, "/home/me/.notes", "pdf"), ("/home/me/.notes.pdf".to_string(), true), "a typed dotfile stays the user's");
    assert_eq!(picked(&WORD, "/home/me/data.csv", "pdf"), ("/home/me/data.csv.pdf".to_string(), true), "not a list ext: added");
    assert_eq!(
        picked(&WORD, "/home/me/data.txt", "pdf"),
        ("/home/me/data.txt.pdf".to_string(), true),
        "another list format's ext: kept, chosen one added"
    );
    for p in ["/home/me/.", "/home/me/..", "."] {
        assert_eq!(WORD.picked_path(p, "pdf"), Err(ExportError::NoFileName), "{p}");
    }
}

#[test]
fn picked_path_is_capped() {
    let long = format!("/home/me/{}", "x".repeat(253));
    let (path, renamed) = picked(&WORD, &long, "pdf");
    let file = path.rsplit('/').next().unwrap_or_default();
    assert!(renamed && file.len() <= MAX_FILE_NAME_BYTES && file.ends_with("x.pdf"), "{}", file.len());
    assert!(path.starts_with("/home/me/"));
    let wide = format!("/d/{}", "é".repeat(200));
    let (path, _) = picked(&WORD, &wide, "pdf");
    assert!(path.trim_start_matches("/d/").len() <= MAX_FILE_NAME_BYTES && path.ends_with("é.pdf"));
}

#[test]
fn plan_for_formats_sharing_an_extension() {
    for runtime in [Runtime::Desktop, Runtime::WebHosted] {
        let Ok(xml) = WORD.plan(runtime, "n.docx", "D", "xml") else { panic!("xml") };
        let Ok(old) = WORD.plan(runtime, "n.docx", "D", "wordml2003") else { panic!("2003") };
        assert_eq!(xml.file_name, old.file_name);
        assert_ne!(xml.format.id, old.format.id);
        assert_eq!(xml.delivery, runtime.save_as_delivery());
    }
}
