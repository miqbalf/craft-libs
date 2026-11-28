//! Save As = Export: one flow for every Craft app on the web and the desktop.
//!
//! Each app fills one [`ExportList`] with every format it can write. Save As shows that list;
//! the chosen format gives a file name ([`ExportList::file_name`], the naming rule) and the
//! platform gives the delivery ([`Runtime::save_as_delivery`]):
//!
//! - **Desktop** ([`Delivery::SaveDialog`]): the normal system save dialog, suggesting
//!   [`SaveAsPlan::file_name`]; the picked path goes through [`ExportList::picked_path`].
//! - **Web, framed by a Nextcloud page or not** ([`Delivery::Download`]): the copy downloads to the
//!   user's computer (`craft_web_host::web::download(name, mime, bytes)`). Save As never goes to
//!   the hosting page; only a plain Save of the page's document does
//!   ([`Runtime::save_goes_to_host`]).
//!
//! The copy never becomes the document's file: the document keeps its path, its name and its
//! dirty state. Plain Save is not part of this module. There is deliberately no "host save"
//! delivery, so a Save As can not reach the host.

use std::fmt;

/// Kept for code written against the first version. Names are capped by [`MAX_FILE_NAME_BYTES`],
/// which is stricter (a name of at most 255 bytes also has at most 255 characters, so
/// `craft_web_host::MAX_NAME_CHARS` never cuts it).
pub const MAX_FILE_NAME_CHARS: usize = 255;

/// The most UTF-8 bytes of a file name this module gives (the extension is kept): the limit of
/// ext4, APFS and most other file systems.
pub const MAX_FILE_NAME_BYTES: usize = 255;

/// What a name with nothing left after cleaning, and an empty fallback, is called.
const UNTITLED: &str = "Untitled";

/// One format an app can write a copy in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportFormat {
    /// The app's own id for the format (its engine's `format` value or command suffix), unique in
    /// the list. Several ids may write the same extension (`xml` and `wordml2003`).
    pub id: &'static str,
    /// The text shown in the list and as the save dialog's filter name, in English: the app's
    /// translation key (`tl!(format.label)`).
    pub label: &'static str,
    /// An optional second line under the label (a translation key too), `""` for none.
    pub detail: &'static str,
    /// The extension the file gets: lower case, without the dot.
    pub ext: &'static str,
    /// The MIME type of a download.
    pub mime: &'static str,
    /// The format writes several files (one per slide, page or image). The name is the base the
    /// app numbers from with its own rule; on the web each file is its own download.
    pub many: bool,
    /// Other extensions the format writes (lower case, no dot), e.g. `jpg` and `tif` for an image
    /// format whose `ext` is `png`. A name or picked path ending in one of them keeps it.
    pub also_exts: &'static [&'static str],
}

impl ExportFormat {
    /// A one-file format with no detail line.
    pub const fn new(id: &'static str, label: &'static str, ext: &'static str, mime: &'static str) -> Self {
        Self { id, label, detail: "", ext, mime, many: false, also_exts: &[] }
    }

    /// The same format with a detail line.
    pub const fn detail(self, detail: &'static str) -> Self {
        Self { detail, ..self }
    }

    /// The same format, also writing these extensions (lower case, no dot).
    pub const fn also_exts(self, also_exts: &'static [&'static str]) -> Self {
        Self { also_exts, ..self }
    }

    /// Whether the format writes files with extension `ext` (any case, a leading dot ignored):
    /// its `ext` or one of its `also_exts`.
    pub fn accepts(&self, ext: &str) -> bool {
        let ext = ext.strip_prefix('.').unwrap_or(ext);
        !ext.is_empty() && (self.ext.eq_ignore_ascii_case(ext) || self.also_exts.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    /// The same format, writing several files.
    pub const fn many(self) -> Self {
        Self { many: true, ..self }
    }
}

/// One app's Save As list: every format it can write, in the order shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportList {
    formats: &'static [ExportFormat],
    /// Extensions the app reads but does not write (`doc`, `htm`, `xls`…): on a document's name
    /// they give way to the chosen format's, as the list's own extensions do.
    extra: &'static [&'static str],
}

/// Why a name, a path or a list was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// No format in the list has this id.
    UnknownFormat(String),
    /// The picked path has no file name (empty, or ends in a folder separator).
    NoFileName,
    /// The list breaks a rule of [`ExportList::check`]; the text says which.
    BadList(String),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat(id) => write!(f, "unknown export format “{id}”"),
            Self::NoFileName => f.write_str("the chosen path has no file name"),
            Self::BadList(why) => write!(f, "bad export list: {why}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// A path picked in the desktop save dialog, with the format's extension applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub path: String,
    /// The path differs from the one picked: the dialog did not confirm this name, so the app
    /// must not overwrite an existing file at `path` without asking.
    pub renamed: bool,
}

/// Where the app runs, for the delivery of a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// The native desktop app.
    Desktop,
    /// A web build with no hosting page (or a host that did not say `saves: true`).
    Web,
    /// A web build framed by a page that stores saves (`?host=parent`, `<app>:host` with
    /// `saves: true`).
    WebHosted,
}

/// How a Save As copy is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The browser downloads the bytes (`craft_web_host::web::download`).
    Download,
    /// The system save dialog picks the path, and the app writes the file there.
    SaveDialog,
}

impl Runtime {
    /// The runtime of a web build: whether the hosting page said `saves: true`.
    pub fn web(host_saves: bool) -> Self {
        if host_saves { Self::WebHosted } else { Self::Web }
    }

    /// How Save As (any format, any entry point) delivers: a download on the web, framed or not,
    /// and the save dialog on the desktop. Never the hosting page.
    pub fn save_as_delivery(self) -> Delivery {
        match self {
            Self::Desktop => Delivery::SaveDialog,
            Self::Web | Self::WebHosted => Delivery::Download,
        }
    }

    /// Whether a plain Save of the open document goes to the hosting page. Only plain Save does,
    /// and only on a page that stores saves; how Save is done otherwise is the app's own.
    pub fn save_goes_to_host(self) -> bool {
        self == Self::WebHosted
    }
}

/// Everything a Save As needs once a format is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAsPlan {
    pub format: ExportFormat,
    /// The name to download under, or to suggest in the save dialog.
    pub file_name: String,
    pub delivery: Delivery,
}

impl ExportList {
    /// The list of `formats`, in the order shown.
    pub const fn new(formats: &'static [ExportFormat]) -> Self {
        Self { formats, extra: &[] }
    }

    /// The same list, where these extensions (lower case, no dot) on a document's name also give
    /// way to the chosen format's.
    pub const fn replacing(self, extra: &'static [&'static str]) -> Self {
        Self { extra, ..self }
    }

    /// Every format, in the order shown.
    pub fn formats(&self) -> &'static [ExportFormat] {
        self.formats
    }

    /// The format with this id (exact match).
    pub fn get(&self, id: &str) -> Option<ExportFormat> {
        self.formats.iter().find(|f| f.id == id).copied()
    }

    /// The first format writing `ext` (any case, a leading dot ignored): the one to preselect for
    /// a document of that type.
    pub fn for_ext(&self, ext: &str) -> Option<ExportFormat> {
        let ext = ext.strip_prefix('.').unwrap_or(ext);
        if ext.is_empty() {
            return None;
        }
        self.formats.iter().find(|f| f.ext.eq_ignore_ascii_case(ext)).or_else(|| self.formats.iter().find(|f| f.accepts(ext))).copied()
    }

    /// Whether `ext` is one the list writes or one of its [`Self::replacing`] extensions: such an
    /// extension on a name gives way to the chosen format's; any other (`notes.v2`) is part of
    /// the name.
    pub fn is_document_ext(&self, ext: &str) -> bool {
        !ext.is_empty() && (self.formats.iter().any(|f| f.accepts(ext)) || self.extra.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    /// Whether the list follows the rules: not empty; ids unique and not empty; labels not
    /// empty and unique (a label is also the save dialog's filter name); extensions (`ext`,
    /// `also_exts`, `replacing`) lower case ASCII letters and digits; MIME types `type/subtype`. Each
    /// app's tests call it on its list.
    pub fn check(&self) -> Result<(), ExportError> {
        let bad = |why: String| Err(ExportError::BadList(why));
        let ext_ok = |e: &str| !e.is_empty() && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if self.formats.is_empty() {
            return bad("no formats".into());
        }
        for (i, f) in self.formats.iter().enumerate() {
            if f.id.is_empty() {
                return bad(format!("format {i} has no id"));
            }
            if self.formats[..i].iter().any(|g| g.id == f.id) {
                return bad(format!("id “{}” is used twice", f.id));
            }
            if f.label.trim().is_empty() {
                return bad(format!("“{}” has no label", f.id));
            }
            if self.formats[..i].iter().any(|g| g.label == f.label) {
                return bad(format!("label “{}” is used twice", f.label));
            }
            if let Some(e) = f.also_exts.iter().find(|e| !ext_ok(e)) {
                return bad(format!("“{}” also has extension “{e}”: use lower case letters and digits, no dot", f.id));
            }
            if !ext_ok(f.ext) {
                return bad(format!("“{}” has extension “{}”: use lower case letters and digits, no dot", f.id, f.ext));
            }
            let mime_ok = f.mime.split_once('/').is_some_and(|(t, s)| !t.is_empty() && !s.is_empty() && !f.mime.chars().any(char::is_whitespace));
            if !mime_ok {
                return bad(format!("“{}” has MIME type “{}”", f.id, f.mime));
            }
        }
        if let Some(e) = self.extra.iter().find(|e| !ext_ok(e)) {
            return bad(format!("extra extension “{e}”: use lower case letters and digits, no dot"));
        }
        Ok(())
    }

    /// The naming rule for a document **path or file name** `doc_name`: the name a copy in format
    /// `id` gets. Use [`Self::file_name_from_title`] for a title, which may contain `/`.
    ///
    /// - Only the last component counts (folders before `/` or `\` are dropped).
    /// - Control characters and invisible format characters (bidi overrides, zero-width space,
    ///   BOM, soft hyphen) are dropped; the characters Windows refuses, `< > : " | ? *`, become
    ///   `_`. Space is trimmed, leading dots are dropped (no hidden files), and trailing dots and
    ///   spaces are dropped (Windows drops them too).
    /// - With nothing left, or only a document extension left (`.pdf`, `...`), `fallback` is
    ///   used, cleaned the same way (then `Untitled`).
    /// - The format's extension replaces a document extension ([`Self::is_document_ext`]):
    ///   `notes.docx` as PDF is `notes.pdf`, and the stem loses trailing dots and spaces
    ///   (`notes .docx` is `notes.pdf`). An extension the format accepts is kept as typed
    ///   (`notes.PDF`; `scan.jpeg` for an image format with `also_exts` `jpeg`). Any other name gets
    ///   the extension added (`notes.v2.pdf`, `report.pdf.` is `report.pdf`).
    /// - A stem that is a Windows device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`,
    ///   `LPT1`-`LPT9`, any case, before its first dot) gets a `_` prefix: `_CON.pdf`.
    /// - At most [`MAX_FILE_NAME_BYTES`] UTF-8 bytes, cut from the stem on a character boundary
    ///   (not between a letter and its combining marks or joiners), so the extension stays.
    pub fn file_name(&self, doc_name: &str, fallback: &str, id: &str) -> Result<String, ExportError> {
        self.named(doc_name, fallback, id, Source::Path, None)
    }

    /// [`Self::file_name`] for a document **title** (not a path): `/` and `\` are ordinary
    /// characters and become `-` (`Budget 2025/26` is `Budget 2025-26.pdf`); every other rule is
    /// the same. The fallback is read as a title too.
    pub fn file_name_from_title(&self, title: &str, fallback: &str, id: &str) -> Result<String, ExportError> {
        self.named(title, fallback, id, Source::Title, None)
    }

    /// The name of file number `n` of a many-file format ([`ExportFormat::many`]):
    /// [`Self::file_name`] with `-{n}` before the extension (`Deck-3.png`), capped the same way
    /// with the number and extension kept.
    pub fn numbered_file_name(&self, doc_name: &str, fallback: &str, id: &str, n: usize) -> Result<String, ExportError> {
        self.named(doc_name, fallback, id, Source::Path, Some(n))
    }

    /// A path picked in the desktop save dialog for format `id`. The folders are kept as given;
    /// only the file name may change:
    ///
    /// - trailing dots and spaces are dropped (`report.pdf.` is `report.pdf`);
    /// - an extension the format accepts ([`ExportFormat::accepts`]) is kept as typed;
    /// - any other name keeps everything the user typed and gets the format's extension added:
    ///   `notes` is `notes.pdf`, and `data.csv` picked for PDF is `data.csv.pdf`, even when `csv`
    ///   is another format of the list (the chosen format, not the typed extension, decides what
    ///   is written, and the user's name is never replaced);
    /// - at most [`MAX_FILE_NAME_BYTES`] bytes, cut from the stem.
    ///
    /// No other cleaning: the dialog already accepted the name. When the path changes,
    /// [`Picked::renamed`] says so: the dialog's overwrite question was about the other name, so
    /// the app must not overwrite an existing file at the new path without asking.
    pub fn picked_path(&self, picked: &str, id: &str) -> Result<Picked, ExportError> {
        let format = self.format(id)?;
        let cut = picked.rfind(['/', '\\']).map_or(0, |i| i + 1);
        let (dir, file) = picked.split_at(cut);
        let file = file.trim_end_matches(['.', ' ']);
        if file.is_empty() {
            return Err(ExportError::NoFileName);
        }
        let (stem, ext) = match file.rfind('.') {
            Some(i) if format.accepts(&file[i + 1..]) => (&file[..i], &file[i + 1..]),
            _ => (file, format.ext),
        };
        let path = format!("{dir}{}", cap(stem, &format!(".{ext}")));
        let renamed = path != picked;
        Ok(Picked { path, renamed })
    }

    /// [`Self::file_name`] and [`Runtime::save_as_delivery`] together.
    pub fn plan(&self, runtime: Runtime, doc_name: &str, fallback: &str, id: &str) -> Result<SaveAsPlan, ExportError> {
        let file_name = self.file_name(doc_name, fallback, id)?;
        let format = self.format(id)?;
        Ok(SaveAsPlan { format, file_name, delivery: runtime.save_as_delivery() })
    }

    fn format(&self, id: &str) -> Result<ExportFormat, ExportError> {
        self.get(id).ok_or_else(|| ExportError::UnknownFormat(id.to_string()))
    }

    /// The naming rule of [`Self::file_name`], for a path or a title, numbered or not.
    fn named(&self, raw: &str, fallback: &str, id: &str, source: Source, n: Option<usize>) -> Result<String, ExportError> {
        let format = self.format(id)?;
        let base = self.base(raw, source).or_else(|| self.base(fallback, source)).unwrap_or_else(|| UNTITLED.to_string());
        let (stem, ext) = match base.rfind('.').filter(|&i| i > 0) {
            Some(i) if format.accepts(&base[i + 1..]) => (base[..i].to_string(), base[i + 1..].to_string()),
            Some(i) if self.is_document_ext(&base[i + 1..]) => (base[..i].trim_end_matches(['.', ' ']).to_string(), format.ext.to_string()),
            _ => (base.clone(), format.ext.to_string()),
        };
        let stem = if stem.is_empty() { UNTITLED.to_string() } else { stem };
        let stem = if is_device_name(&stem) { format!("_{stem}") } else { stem };
        let suffix = match n {
            Some(n) => format!("-{n}.{ext}"),
            None => format!(".{ext}"),
        };
        Ok(cap(&stem, &suffix))
    }

    /// `raw` cleaned ([`Self::file_name`]); `None` when nothing is left, or only a document
    /// extension after a dot (`.pdf`).
    fn base(&self, raw: &str, source: Source) -> Option<String> {
        let raw = match source {
            Source::Path => raw.rsplit(['/', '\\']).next().unwrap_or_default(),
            Source::Title => raw,
        };
        let kept: String = raw
            .chars()
            .filter(|&c| !c.is_control() && !is_invisible(c))
            .map(|c| match c {
                '/' | '\\' => '-',
                '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
                c => c,
            })
            .collect();
        let trimmed = kept.trim();
        let undotted = trimmed.trim_start_matches('.').trim_start();
        let name = undotted.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
        let only_ext = undotted.len() < trimmed.len() && !name.contains('.') && self.is_document_ext(name);
        (!name.is_empty() && !only_ext).then(|| name.to_string())
    }
}

/// Where a name comes from: a path (its last component counts) or a title (all of it).
#[derive(Debug, Clone, Copy)]
enum Source {
    Path,
    Title,
}

/// Whether `stem` (before its first dot, trailing space ignored) is a Windows device name.
fn is_device_name(stem: &str) -> bool {
    let head = stem.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    match head.as_bytes() {
        [a, b, c] => matches!([*a, *b, *c], [b'C', b'O', b'N'] | [b'P', b'R', b'N'] | [b'A', b'U', b'X'] | [b'N', b'U', b'L']),
        [a, b, c, d] => matches!([*a, *b, *c], [b'C', b'O', b'M'] | [b'L', b'P', b'T']) && (b'1'..=b'9').contains(d),
        _ => false,
    }
}

/// Invisible format characters that can hide or reorder text in a name: bidi controls, zero-width
/// space, word joiners, BOM, soft hyphen. Zero-width (non-)joiners stay; scripts need them.
fn is_invisible(c: char) -> bool {
    matches!(c as u32, 0xAD | 0x61C | 0x180E | 0x200B | 0x200E | 0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF)
}

/// Whether `c` belongs to the character before it: a combining mark, variation selector, joiner
/// or emoji modifier.
fn joins(c: char) -> bool {
    matches!(
        c as u32,
        0x300..=0x36F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200C | 0x200D | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0x1F3FB..=0x1F3FF | 0xE0100..=0xE01EF
    )
}

/// `stem` + `suffix` (`.ext` or `-n.ext`) in at most [`MAX_FILE_NAME_BYTES`] bytes, cut from the
/// stem on a character boundary, never between a character and what [`joins`] it, with trailing
/// dots and spaces of a cut stem dropped.
fn cap(stem: &str, suffix: &str) -> String {
    let budget = MAX_FILE_NAME_BYTES.saturating_sub(suffix.len());
    if stem.len() <= budget {
        return format!("{stem}{suffix}");
    }
    let mut end = budget;
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    let mut kept: Vec<char> = stem[..end].chars().collect();
    let mut next = stem[end..].chars().next();
    // Drop the last kept character while what follows it belongs to it, or it is a joiner.
    while let Some(&last) = kept.last() {
        if next.is_some_and(joins) || joins(last) {
            next = kept.pop();
        } else {
            break;
        }
    }
    let cut: String = kept.into_iter().collect();
    let cut = cut.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    format!("{}{suffix}", if cut.is_empty() { "_" } else { cut })
}

/// The MIME type for a file extension (any case, a leading dot ignored), covering what the Craft
/// apps write; `application/octet-stream` for anything else. For filling [`ExportFormat::mime`].
pub fn mime_for_ext(ext: &str) -> &'static str {
    let ext = ext.strip_prefix('.').unwrap_or(ext).to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "dotx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ott" => "application/vnd.oasis.opendocument.text-template",
        "fodt" => "application/vnd.oasis.opendocument.text-flat-xml",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "rtf" => "application/rtf",
        "html" | "htm" => "text/html",
        "md" => "text/markdown",
        "tex" => "application/x-tex",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" => "application/json",
        "xml" => "application/xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "xfdf" => "application/vnd.adobe.xfdf",
        "fdf" => "application/vnd.fdf",
        _ => "application/octet-stream",
    }
}
