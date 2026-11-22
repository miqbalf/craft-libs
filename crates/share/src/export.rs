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

/// The most characters of a file name [`ExportList::file_name`] gives (the extension is kept). The
/// same cap as `craft_web_host::MAX_NAME_CHARS`, so a download keeps the name unchanged.
pub const MAX_FILE_NAME_CHARS: usize = 255;

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
}

impl ExportFormat {
    /// A one-file format with no detail line.
    pub const fn new(id: &'static str, label: &'static str, ext: &'static str, mime: &'static str) -> Self {
        Self { id, label, detail: "", ext, mime, many: false }
    }

    /// The same format with a detail line.
    pub const fn detail(self, detail: &'static str) -> Self {
        Self { detail, ..self }
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
        self.formats.iter().find(|f| f.ext.eq_ignore_ascii_case(ext)).copied()
    }

    /// Whether `ext` is one the list writes or one of its [`Self::replacing`] extensions: such an
    /// extension on a name gives way to the chosen format's; any other (`notes.v2`) is part of
    /// the name.
    pub fn is_document_ext(&self, ext: &str) -> bool {
        !ext.is_empty() && (self.formats.iter().any(|f| f.ext.eq_ignore_ascii_case(ext)) || self.extra.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    }

    /// Whether the list follows the rules: not empty; ids unique and not empty; labels not
    /// empty; extensions lower case ASCII letters and digits; MIME types `type/subtype`. Each
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

    /// The naming rule: the name a copy of the document `doc_name` (a name or a path) in format
    /// `id` gets.
    ///
    /// - Folders (`/` or `\`) and control characters are dropped and space is trimmed; with
    ///   nothing left (or `.`/`..`) `fallback` is used, cleaned the same way (then `Untitled`).
    /// - The format's extension replaces a document extension ([`Self::is_document_ext`]):
    ///   `notes.docx` as PDF is `notes.pdf`. It is kept when it already matches in any case
    ///   (`notes.PDF`). Any other name gets it added (`notes.v2.pdf`); a trailing `.` is dropped
    ///   first, and a dotfile name (`.docx`) is a stem (`.docx.pdf`).
    /// - At most [`MAX_FILE_NAME_CHARS`] characters, cut from the stem so the extension stays.
    pub fn file_name(&self, doc_name: &str, fallback: &str, id: &str) -> Result<String, ExportError> {
        let format = self.get(id).ok_or_else(|| ExportError::UnknownFormat(id.to_string()))?;
        let base = clean(doc_name).or_else(|| clean(fallback)).unwrap_or_else(|| UNTITLED.to_string());
        let named = self.with_ext(&base, format.ext);
        Ok(cap(&named, format.ext))
    }

    /// A path picked in the desktop save dialog for format `id`, with the extension rule of
    /// [`Self::file_name`] applied to its file name (the folders are kept as given). When this
    /// changes the path, [`Picked::renamed`] says so: the dialog's overwrite question was about
    /// the other name.
    pub fn picked_path(&self, picked: &str, id: &str) -> Result<Picked, ExportError> {
        let format = self.get(id).ok_or_else(|| ExportError::UnknownFormat(id.to_string()))?;
        let cut = picked.rfind(['/', '\\']).map_or(0, |i| i + 1);
        let (dir, file) = picked.split_at(cut);
        if file.is_empty() || file == "." || file == ".." {
            return Err(ExportError::NoFileName);
        }
        let path = format!("{dir}{}", self.with_ext(file, format.ext));
        let renamed = path != picked;
        Ok(Picked { path, renamed })
    }

    /// [`Self::file_name`] and [`Runtime::save_as_delivery`] together.
    pub fn plan(&self, runtime: Runtime, doc_name: &str, fallback: &str, id: &str) -> Result<SaveAsPlan, ExportError> {
        let file_name = self.file_name(doc_name, fallback, id)?;
        let format = self.get(id).ok_or_else(|| ExportError::UnknownFormat(id.to_string()))?;
        Ok(SaveAsPlan { format, file_name, delivery: runtime.save_as_delivery() })
    }

    /// `name` with extension `want` (the rule in [`Self::file_name`]).
    fn with_ext(&self, name: &str, want: &str) -> String {
        // A dot at 0 starts a dotfile name, not an extension.
        match name.rfind('.').filter(|&i| i > 0) {
            Some(i) => {
                let (stem, ext) = (&name[..i], &name[i + 1..]);
                if ext.eq_ignore_ascii_case(want) {
                    name.to_string()
                } else if self.is_document_ext(ext) {
                    format!("{stem}.{want}")
                } else {
                    format!("{}.{want}", name.trim_end_matches('.'))
                }
            }
            None => format!("{name}.{want}"),
        }
    }
}

/// The last component of `raw`, without control characters and surrounding space; `None` when
/// nothing (or only `.`/`..`) is left.
fn clean(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let kept: String = base.chars().filter(|c| !c.is_control()).collect();
    let trimmed = kept.trim();
    (!trimmed.is_empty() && trimmed != "." && trimmed != "..").then(|| trimmed.to_string())
}

/// `name` (ending in `.{ext}`) cut to [`MAX_FILE_NAME_CHARS`] characters, from the stem.
fn cap(name: &str, ext: &str) -> String {
    if name.chars().count() <= MAX_FILE_NAME_CHARS {
        return name.to_string();
    }
    let tail = ext.chars().count() + 1;
    let stem: String = name.chars().take(MAX_FILE_NAME_CHARS.saturating_sub(tail)).collect();
    format!("{}.{ext}", stem.trim_end())
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
