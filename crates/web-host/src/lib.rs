//! The host messages between a Craft app's web build and the page that hosts it: one contract for
//! the whole family (storytold/craft-libs#358), so each app's bridge is a few calls instead of its
//! own copy.
//!
//! Messages are plain objects with a `type` of `<app>:<kind>` (`cadcraft:open`):
//!
//! | Kind | Direction | Fields |
//! |---|---|---|
//! | `ready` | app → host | `version`, `appVersion?`, `file?` (`true`: it reads a `File`/`Blob` in slices) |
//! | `host` | host → app | `version`, `handoff?` (`true`: the host routes hand-offs), `saves?` (`true`: the host stores saves) |
//! | `open` | host → app | `name`, and `bytes` or `file` + `size` |
//! | `attach-pdf` | host → app | `name`, `bytes`, `page` (from 1), `insertion` ([x,y,z]), `width`, `height` in drawing units |
//! | `save` | app → host | `name`, `bytes`, `id?`, `saveAs?` |
//! | `saved` | host → app | `id`, `name?` (the host stored the save under another name: a conflict copy, or Save As) |
//! | `save-failed` | host → app | `id`, `error` (the host could not store the save) |
//! | `fonts` | host → app | `fonts`: `[{name, url}]`, same-origin font files for the app to fetch and install |
//! | `dirty` | app → host | `dirty` |
//! | `failed` | app → host | `error`, `id?` |
//! | `handoff` | app → host | `name`, and `bytes` or `file` + `size` |
//! | `handoff-done` | host → app | `name`, `app` |
//!
//! A host answers `saved` or `save-failed` only for a `save` that carries an `id`; an app that sends
//! none treats a save as done once it is posted. When `host` has no `saves`, the app keeps its
//! earlier behaviour (a download, never marked saved). With `saves`, the app marks a document saved
//! only when `saved` arrives for its `id`, and stays unsaved and shows the error on `save-failed`.
//! A live document is saved by its live session, not by `save`. A host sends at most
//! [`MAX_FONTS`] fonts in `fonts` (the rest are dropped), each fetched from this origin only and
//! at most [`MAX_FONT_BYTES`] long. Fields marked `?` are optional, so the bridges shipped before
//! this crate already speak version 1. Those bridges put the app's own release in `ready`'s
//! `version` (`"0.14.0"`): a host treats any `version` it doesn't know as `"1"`.
//!
//! The app is hosted when its URL has `?host=parent` (framed by a same-origin page: messages go
//! through `postMessage`) or its canvas has `data-host="page"` (the page runs it directly:
//! messages are `CustomEvent`s on the canvas, the fields in `detail`). This module is the contract
//! itself and builds everywhere; [`web`] (wasm32 only) is the bridge an app's web shell calls.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

pub mod live;

#[cfg(target_arch = "wasm32")]
pub mod web;

/// The contract version an app reports in `ready` and a host in `host`. It changes only when a
/// field changes meaning; new optional fields keep it.
pub const VERSION: &str = "1";

/// Longest file name taken from a host or put in a message, in characters.
pub const MAX_NAME_CHARS: usize = 255;

/// Most entries taken from one `fonts` message; later ones are dropped.
pub const MAX_FONTS: usize = 64;

/// Longest font file an app takes from a host, in bytes (20 MB).
pub const MAX_FONT_BYTES: usize = 20 * 1_000_000;

/// How a page hosts the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `?host=parent`: a same-origin page frames the app; messages go through `postMessage`.
    Parent,
    /// `data-host="page"` on the app's canvas: the page runs the app itself; messages are
    /// `CustomEvent`s on the canvas.
    Page,
}

impl Mode {
    /// The mode for a page URL's query (with or without its leading `?`) and the canvas's
    /// `data-host` attribute. `?host=parent` wins; anything else is not hosted.
    pub fn detect(query: &str, data_host: Option<&str>) -> Option<Self> {
        let query = query.strip_prefix('?').unwrap_or(query);
        if query.split('&').any(|pair| pair == "host=parent") {
            Some(Self::Parent)
        } else if data_host == Some("page") {
            Some(Self::Page)
        } else {
            None
        }
    }
}

/// What an app posts to its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outgoing {
    Ready,
    Save,
    Dirty,
    Failed,
    Handoff,
}

/// What a host sends to an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Incoming {
    Host,
    Open,
    AttachPdf,
    Saved,
    SaveFailed,
    Fonts,
    HandoffDone,
}

impl Outgoing {
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Save => "save",
            Self::Dirty => "dirty",
            Self::Failed => "failed",
            Self::Handoff => "handoff",
        }
    }
}

impl Incoming {
    pub const ALL: [Self; 7] = [Self::Host, Self::Open, Self::AttachPdf, Self::Saved, Self::SaveFailed, Self::Fonts, Self::HandoffDone];

    pub const fn kind(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Open => "open",
            Self::AttachPdf => "attach-pdf",
            Self::Saved => "saved",
            Self::SaveFailed => "save-failed",
            Self::Fonts => "fonts",
            Self::HandoffDone => "handoff-done",
        }
    }

    /// The incoming kind a message `type` names for `app`, or `None` for another app's message or
    /// one this contract doesn't send to apps.
    pub fn parse(app: &str, message_type: &str) -> Option<Self> {
        let kind = message_type.strip_prefix(app)?.strip_prefix(':')?;
        Self::ALL.into_iter().find(|k| k.kind() == kind)
    }
}

/// The `type` of a message: `<app>:<kind>`.
pub fn message_type(app: &str, kind: &str) -> String {
    format!("{app}:{kind}")
}

/// A file name from a host, made safe to show and save under: path separators and control
/// characters dropped, surrounding space trimmed, at most [`MAX_NAME_CHARS`] characters, and
/// `fallback` when nothing is left.
pub fn file_name(raw: Option<&str>, fallback: &str) -> String {
    let base = raw.unwrap_or_default().rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return fallback.to_string();
    }
    trimmed.chars().take(MAX_NAME_CHARS).collect()
}

/// A font the host offers: a display `name` and a same-origin `url` to fetch it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontRef {
    pub name: String,
    pub url: String,
}

/// Whether `url` is on `origin` (`https://host:port`, no trailing slash): an absolute path
/// (`/fonts/a.ttf`, not `//other/…`) or a full URL that starts with the origin and a `/`.
/// Backslashes and control characters are refused, as browsers treat `\` like `/`.
pub fn is_same_origin(url: &str, origin: &str) -> bool {
    if url.is_empty() || url.contains('\\') || url.chars().any(char::is_control) {
        return false;
    }
    if let Some(path) = url.strip_prefix('/') {
        return !path.starts_with('/');
    }
    !origin.is_empty() && url.strip_prefix(origin).is_some_and(|rest| rest.starts_with('/') && !rest.starts_with("//"))
}

/// The usable fonts of a host's list, given as `(name, url)` pairs as read (either may be
/// missing). An entry with no url, a url off `origin`, or an empty name after cleaning with
/// [`file_name`] is dropped; the first [`MAX_FONTS`] entries are looked at, so a longer list is cut.
pub fn font_list<'a>(entries: impl IntoIterator<Item = (Option<&'a str>, Option<&'a str>)>, origin: &str) -> Vec<FontRef> {
    entries
        .into_iter()
        .take(MAX_FONTS)
        .filter_map(|(name, url)| {
            let url = url.filter(|u| is_same_origin(u, origin))?;
            let fallback = url.rsplit('/').next().unwrap_or_default();
            let name = file_name(name, fallback);
            (!name.is_empty()).then(|| FontRef { name, url: url.to_string() })
        })
        .collect()
}

/// A font file of `len` bytes is within [`MAX_FONT_BYTES`] (and not empty).
pub fn font_size_ok(len: f64) -> bool {
    len.is_finite() && len > 0.0 && len <= MAX_FONT_BYTES as f64
}

/// The saves an app has sent and not yet seen answered. An app takes an id from [`Self::start`],
/// posts the save with it, and acts on `saved` or `save-failed` only when [`Self::finish`] says the
/// id was waiting: an answer for an unknown, stale or repeated id changes nothing.
#[derive(Debug, Default)]
pub struct PendingSaves {
    next: u64,
    waiting: Vec<String>,
}

impl PendingSaves {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new id for a save that is about to be posted.
    pub fn start(&mut self) -> String {
        self.next += 1;
        let id = format!("save-{}", self.next);
        self.waiting.push(id.clone());
        id
    }

    /// The host answered `id`: true the first time for an id that was waiting.
    pub fn finish(&mut self, id: &str) -> bool {
        match self.waiting.iter().position(|w| w == id) {
            Some(at) => {
                self.waiting.remove(at);
                true
            }
            None => false,
        }
    }

    /// The post for `id` failed, so no answer will come.
    pub fn cancel(&mut self, id: &str) {
        self.finish(id);
    }

    pub fn is_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }
}

/// Remember `next` before the send. A page host can, while that send is still
/// running, report a different state; that later report must win. A failed send
/// restores the previous state only when nothing else recorded one.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn note_dirty(dirty: &Cell<Option<bool>>, next: bool, post: impl FnOnce() -> bool) {
    if dirty.get() == Some(next) {
        return;
    }
    let previous = dirty.replace(Some(next));
    if !post() && dirty.get() == Some(next) {
        dirty.set(previous);
    }
}

/// Hands the host's messages to the app's handler one at a time. A page host dispatches its events
/// synchronously, so it can answer while the handler runs (a `saved` for the `save` the handler
/// just posted): that message waits here and is handled as soon as the running call returns, never
/// dropped and never given to a second, overlapping call.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct Inbox<M, F> {
    handler: RefCell<F>,
    queue: RefCell<VecDeque<M>>,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl<M, F: FnMut(M)> Inbox<M, F> {
    pub(crate) fn new(handler: F) -> Self {
        Self { handler: RefCell::new(handler), queue: RefCell::new(VecDeque::new()) }
    }

    pub(crate) fn deliver(&self, message: M) {
        self.queue.borrow_mut().push_back(message);
        // Already running: the outer call drains the queue.
        let Ok(mut handler) = self.handler.try_borrow_mut() else { return };
        loop {
            let next = self.queue.borrow_mut().pop_front();
            let Some(message) = next else { break };
            handler(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_switch_wins_over_the_canvas_attribute() {
        assert_eq!(Mode::detect("?host=parent", None), Some(Mode::Parent));
        assert_eq!(Mode::detect("file=a.pdf&host=parent", Some("page")), Some(Mode::Parent));
        assert_eq!(Mode::detect("", Some("page")), Some(Mode::Page));
        assert_eq!(Mode::detect("?host=parents", None), None);
        assert_eq!(Mode::detect("?xhost=parent", None), None);
        assert_eq!(Mode::detect("", Some("frame")), None);
    }

    #[test]
    fn only_this_apps_incoming_kinds_parse() {
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:open"), Some(Incoming::Open));
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:handoff-done"), Some(Incoming::HandoffDone));
        assert_eq!(Incoming::parse("pdfcraft", "cadcraft:open"), None, "another app's message");
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:save"), None, "app-to-host kinds are not incoming");
        assert_eq!(Incoming::parse("pdf", "pdfcraft:open"), None, "an app id that is only a prefix");
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraftopen"), None);
    }

    #[test]
    fn every_kind_round_trips_through_its_type() {
        for kind in Incoming::ALL {
            assert_eq!(Incoming::parse("gridcraft", &message_type("gridcraft", kind.kind())), Some(kind));
        }
        assert_eq!(message_type("cadcraft", Outgoing::Handoff.kind()), "cadcraft:handoff");
    }

    #[test]
    fn host_file_names_are_cleaned() {
        assert_eq!(file_name(Some("plan.dxf"), "Drawing.dxf"), "plan.dxf");
        assert_eq!(file_name(Some("  ../../etc/passwd "), "x"), "passwd");
        assert_eq!(file_name(Some("C:\\Users\\sam\\notes.pdf"), "x"), "notes.pdf");
        assert_eq!(file_name(Some("a\u{0}b\nc.txt"), "x"), "abc.txt");
        assert_eq!(file_name(Some("dir/"), "Drawing.dxf"), "Drawing.dxf");
        assert_eq!(file_name(Some(".."), "x"), "x");
        assert_eq!(file_name(None, "Document.pdf"), "Document.pdf");
        assert_eq!(file_name(Some(&"é".repeat(300)), "x").chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn a_dirty_report_during_the_send_wins_and_a_failed_send_is_forgotten() {
        let dirty = Cell::new(Some(false));
        note_dirty(&dirty, true, || {
            note_dirty(&dirty, false, || true);
            true
        });
        assert_eq!(dirty.get(), Some(false));

        let dirty = Cell::new(None);
        note_dirty(&dirty, true, || false);
        assert_eq!(dirty.get(), None);

        let dirty = Cell::new(Some(true));
        let mut sent = false;
        note_dirty(&dirty, true, || {
            sent = true;
            true
        });
        assert!(!sent);
        assert_eq!(dirty.get(), Some(true));
    }

    #[test]
    fn the_save_and_font_kinds_are_the_hosts_and_parse() {
        assert_eq!(Incoming::parse("wordcraft", "wordcraft:save-failed"), Some(Incoming::SaveFailed));
        assert_eq!(Incoming::parse("wordcraft", "wordcraft:fonts"), Some(Incoming::Fonts));
        assert_eq!(Incoming::parse("wordcraft", "wordcraft:save"), None, "save stays app to host");
    }

    const ORIGIN: &str = "https://cloud.example";

    #[test]
    fn only_urls_on_this_origin_are_same_origin() {
        for ok in ["/fonts/a.ttf", "https://cloud.example/f/a.ttf", "/index.php/apps/x/fonts/Barlow%20Bold.ttf"] {
            assert!(is_same_origin(ok, ORIGIN), "{ok}");
        }
        for bad in [
            "",
            "//evil.example/a.ttf",
            "https://evil.example/a.ttf",
            "https://cloud.example.evil.example/a.ttf",
            "https://cloud.example@evil.example/a.ttf",
            "https://cloud.example",
            "https://cloud.example//evil.example/a.ttf",
            "http://cloud.example/a.ttf",
            "a.ttf",
            "data:font/ttf;base64,AAAA",
            "blob:https://cloud.example/x",
            "/\\evil.example/a.ttf",
            "https://cloud.example/\\evil",
            "/a\n.ttf",
        ] {
            assert!(!is_same_origin(bad, ORIGIN), "{bad:?}");
        }
        assert!(!is_same_origin("https://cloud.example/a.ttf", ""), "no known origin accepts no full URL");
    }

    #[test]
    fn a_font_list_keeps_usable_entries_and_cuts_at_the_cap() {
        let list = font_list(
            [
                (Some("Barlow"), Some("/f/barlow.ttf")),
                (Some("Evil"), Some("https://evil.example/e.ttf")),
                (Some("Nourl"), None),
                (None, Some("/f/Aptos-Regular.ttf")),
                (Some("../../x"), Some("/f/x.ttf")),
            ],
            ORIGIN,
        );
        let names: Vec<_> = list.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Barlow", "Aptos-Regular.ttf", "x"]);

        let many: Vec<String> = (0..MAX_FONTS + 30).map(|i| format!("/f/{i}.ttf")).collect();
        let list = font_list(many.iter().map(|u| (Some("n"), Some(u.as_str()))), ORIGIN);
        assert_eq!(list.len(), MAX_FONTS);
        assert!(list.last().is_some_and(|f| f.url == format!("/f/{}.ttf", MAX_FONTS - 1)), "the first entries are kept");

        // An off-origin entry still counts toward the cap, so a host cannot pad past it.
        let mut mixed: Vec<(Option<&str>, Option<&str>)> = vec![(Some("e"), Some("https://evil.example/e.ttf")); MAX_FONTS];
        mixed.push((Some("late"), Some("/f/late.ttf")));
        assert!(font_list(mixed, ORIGIN).is_empty());
    }

    #[test]
    fn a_font_over_the_byte_cap_is_refused() {
        assert!(font_size_ok(1.0));
        assert!(font_size_ok(MAX_FONT_BYTES as f64));
        assert!(!font_size_ok(MAX_FONT_BYTES as f64 + 1.0));
        assert!(!font_size_ok(0.0));
        assert!(!font_size_ok(f64::NAN));
        assert!(!font_size_ok(f64::INFINITY));
    }

    #[test]
    fn a_save_is_resolved_once_and_only_for_its_own_id() {
        let mut saves = PendingSaves::new();
        let (a, b) = (saves.start(), saves.start());
        assert_ne!(a, b);
        assert!(saves.is_waiting());
        assert!(!saves.finish("other"), "an unknown id changes nothing");
        assert!(saves.finish(&b));
        assert!(!saves.finish(&b), "a repeated answer changes nothing");
        assert!(saves.is_waiting(), "a is still waiting");
        saves.cancel(&a);
        assert!(!saves.is_waiting());
        assert!(!saves.finish(&a), "a cancelled save is never confirmed later");
    }

    #[test]
    fn malformed_message_types_name_no_kind() {
        for bad in
            ["", ":", "pdfcraft", "pdfcraft:", ":open", "pdfcraft:open:extra", "pdfcraft:OPEN", "pdfcraft: open", "pdfcraftx:open", "pdfcraft/open"]
        {
            assert_eq!(Incoming::parse("pdfcraft", bad), None, "{bad:?}");
        }
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:open"), Some(Incoming::Open));
        // The app's own outgoing kinds are not messages to it.
        assert_eq!(Incoming::parse("pdfcraft", "pdfcraft:save"), None);
    }

    #[test]
    fn host_file_names_with_hostile_parts_are_cleaned() {
        // Only control characters, or only separators: nothing is left, so the fallback.
        assert_eq!(file_name(Some("\u{0}\u{7}\n\t"), "Drawing.dxf"), "Drawing.dxf");
        assert_eq!(file_name(Some("///"), "Drawing.dxf"), "Drawing.dxf");
        assert_eq!(file_name(Some("   "), "Drawing.dxf"), "Drawing.dxf");
        // Mixed separators and a control character inside the last part.
        assert_eq!(file_name(Some("a/b\\c\u{1b}[31m.txt"), "x"), "c[31m.txt");
        // Exactly at the limit stays whole; one over is cut; the limit counts characters, not bytes.
        assert_eq!(file_name(Some(&"a".repeat(MAX_NAME_CHARS)), "x").chars().count(), MAX_NAME_CHARS);
        assert_eq!(file_name(Some(&"a".repeat(MAX_NAME_CHARS + 1)), "x").chars().count(), MAX_NAME_CHARS);
        assert_eq!(file_name(Some(&"\u{1F4C4}".repeat(300)), "x"), "\u{1F4C4}".repeat(MAX_NAME_CHARS));
    }

    #[test]
    fn messages_that_arrive_while_the_handler_runs_wait_their_turn() {
        use std::rc::Rc;
        type Shared = Rc<Inbox<&'static str, Box<dyn FnMut(&'static str)>>>;
        let seen = Rc::new(RefCell::new(Vec::new()));
        let inbox: Rc<RefCell<Option<Shared>>> = Rc::new(RefCell::new(None));
        let (log, me) = (seen.clone(), inbox.clone());
        let handler: Box<dyn FnMut(&'static str)> = Box::new(move |m: &'static str| {
            log.borrow_mut().push(format!("start {m}"));
            // The page answers inside the handler, as a synchronous dispatchEvent does.
            if m == "open"
                && let Some(inbox) = me.borrow().clone()
            {
                inbox.deliver("saved");
                inbox.deliver("handoff-done");
            }
            log.borrow_mut().push(format!("end {m}"));
        });
        let shared = Rc::new(Inbox::new(handler));
        *inbox.borrow_mut() = Some(shared.clone());
        shared.deliver("open");
        assert_eq!(*seen.borrow(), ["start open", "end open", "start saved", "end saved", "start handoff-done", "end handoff-done"]);
        shared.deliver("host");
        assert_eq!(seen.borrow().len(), 8, "the inbox is usable again after draining");
    }
}
