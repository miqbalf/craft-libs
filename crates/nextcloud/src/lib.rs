//! Nextcloud for the Craft apps: files over WebDAV, WebDAV locks, and Login Flow v2.
//!
//! The crate builds requests and parses responses. It opens no sockets: the caller supplies a
//! transport by implementing [`http::Http`], so the same code serves a native app and a
//! WebAssembly build, and every test here runs without a network.
//!
//! Extracted from [collab-craft](https://github.com/wdunn001/collab-craft) `crates/nextcloud`
//! (MIT OR Apache-2.0); changed files say so. Multistatus parsing goes through `craft-dav`.
//!
//! ```no_run
//! use craft_nextcloud::{Account, dav::Files};
//! # fn demo<H: craft_nextcloud::http::Http>(http: &H) -> craft_nextcloud::Result<()> {
//! let acct = Account::new("https://cloud.example.net", "alice", "app-password");
//! let files = Files::new(&acct);
//! for entry in files.list(http, "/Documents")? {
//!     println!("{} ({:?} bytes)", entry.name, entry.size);
//! }
//! # Ok(()) }
//! ```
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod dav;
mod error;
pub mod http;
pub mod login;

pub use error::{Error, Result};

use std::fmt;

use http::{Http, Method, Request, Response};

/// Largest listing body (`PROPFIND`, `SEARCH`, shares) the parsers here will walk. A folder of
/// tens of thousands of entries fits; it matches [`MAX_LISTING_NODES`].
pub(crate) const MAX_LISTING: usize = 64 * 1024 * 1024;
/// Most XML nodes in a listing: about 25 per entry at the props asked for, so well over 100,000
/// entries.
pub(crate) const MAX_LISTING_NODES: usize = 4_000_000;
/// Largest body of a small JSON answer (Login Flow, capabilities) or of a write or lock, whose
/// body is not used: a guard against a server that answers a small request with something
/// enormous.
pub(crate) const MAX_SMALL: usize = 1024 * 1024;

/// Where to reach a Nextcloud, and as whom.
///
/// `password` should be an **app password**, as [`login::LoginFlow`] yields. `Debug` is written by
/// hand so the secret cannot reach a log through a derived formatter.
///
/// Changed from collab-craft: the files-root and address-book overrides are gone.
#[derive(Clone)]
pub struct Account {
    base: String,
    user: String,
    password: login::Secret,
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account").field("base", &self.base).field("user", &self.user).field("password", &self.password).finish()
    }
}

impl Account {
    /// `base` is the server root, with or without a trailing slash, as in
    /// `https://cloud.example.net`.
    pub fn new(base: impl Into<String>, user: impl Into<String>, password: impl Into<String>) -> Account {
        let mut base = base.into();
        while base.ends_with('/') {
            base.pop();
        }
        Account { base, user: user.into(), password: login::Secret::new(password) }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// The value for an `Authorization` header: Basic auth, which Nextcloud expects for an app
    /// password.
    pub(crate) fn authorization(&self) -> String {
        let raw = format!("{}:{}", self.user, self.password.expose());
        format!("Basic {}", base64(raw.as_bytes()))
    }

    /// The WebDAV files root for this user, without a trailing slash.
    pub(crate) fn files_root(&self) -> String {
        format!("{}/remote.php/dav/files/{}", self.base, encode_segment(&self.user))
    }

    /// A request with authentication and the headers Nextcloud wants on every call.
    pub(crate) fn request(&self, method: Method, url: impl Into<String>) -> Request {
        Request::new(method, url).header("authorization", self.authorization()).header("ocs-apirequest", "true")
    }
}

/// Percent-encode one path segment.
///
/// Everything outside the unreserved set of RFC 3986 is encoded, so a `/` in the input can never
/// widen a segment into a path.
pub fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(*byte as char),
            other => {
                out.push('%');
                out.push(hex_digit(other >> 4));
                out.push(hex_digit(other & 0x0f));
            }
        }
    }
    out
}

/// Percent-encode a `/`-separated path, segment by segment. Leading, trailing and doubled slashes
/// collapse, and a `.` or `..` segment is rejected: a remote path is built from user input.
pub(crate) fn encode_path(path: &str) -> Result<String> {
    let mut out = String::with_capacity(path.len());
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." {
            return Err(Error::malformed(format!("path has traversal segment {segment:?}")));
        }
        out.push('/');
        out.push_str(&encode_segment(segment));
    }
    Ok(out)
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        // `nibble` is masked to 4 bits by the caller, so this arm covers 10..=15.
        n => (b'A' + (n - 10)) as char,
    }
}

/// Standard base64, with padding.
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied();
        let b2 = chunk.get(2).copied();
        let n = u32::from(b0) << 16 | u32::from(b1.unwrap_or(0)) << 8 | u32::from(b2.unwrap_or(0));
        out.push(char::from(ALPHABET[(n >> 18 & 0x3f) as usize]));
        out.push(char::from(ALPHABET[(n >> 12 & 0x3f) as usize]));
        out.push(if b1.is_some() { char::from(ALPHABET[(n >> 6 & 0x3f) as usize]) } else { '=' });
        out.push(if b2.is_some() { char::from(ALPHABET[(n & 0x3f) as usize]) } else { '=' });
    }
    out
}

/// Send a request and reject a non-2xx status (207 Multi-Status passes). The request tells the
/// transport `max_body`, and a 2xx body over it is refused here as well, in case the transport
/// read it anyway.
pub(crate) fn send<H: Http>(http: &H, req: Request, max_body: usize) -> Result<Response> {
    let method = req.method;
    let resp = fetch(http, req, max_body)?;
    if !resp.is_success() {
        return Err(Error::from_response(method, &resp));
    }
    within(resp, max_body)
}

/// Send `req` with [`Request::max_body`] set, whatever the status that comes back. The caller
/// checks the status, then the length with [`within`].
pub(crate) fn fetch<H: Http>(http: &H, req: Request, max_body: usize) -> Result<Response> {
    http.send(req.max_body(max_body)).map_err(|e| Error::Transport(e.to_string()))
}

/// `resp`, unless its body is over `max_body`.
pub(crate) fn within(resp: Response, max_body: usize) -> Result<Response> {
    if resp.body.len() > max_body {
        return Err(Error::malformed(format!("response body of {} bytes exceeds the {max_body} byte limit", resp.body.len())));
    }
    Ok(resp)
}
