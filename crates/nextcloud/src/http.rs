//! The transport seam.
//!
//! This crate builds requests and parses responses. It never opens a socket. The caller supplies
//! the transport by implementing [`Http`], which keeps the crate free of an HTTP dependency and
//! lets the same code serve a native app and a WebAssembly build, where the only way out is the
//! browser's own fetch.
//!
//! Changed from collab-craft: the doc comment of `Http`, `Request::max_body`, and `Debug` for
//! `Request` and `Response` written by hand so neither prints a credential.
//!
//! The split follows the one the Craft apps already use for Zotero, where the wire format and the
//! socket live in separate modules.

use std::collections::BTreeMap;
use std::fmt;

/// An HTTP method. WebDAV and CardDAV need more than the browser verbs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
    Propfind,
    Search,
    Report,
    Mkcol,
    Lock,
    Unlock,
    Options,
}

impl Method {
    /// The token that goes on the request line.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Put => "PUT",
            Method::Post => "POST",
            Method::Delete => "DELETE",
            Method::Propfind => "PROPFIND",
            Method::Search => "SEARCH",
            Method::Report => "REPORT",
            Method::Mkcol => "MKCOL",
            Method::Lock => "LOCK",
            Method::Unlock => "UNLOCK",
            Method::Options => "OPTIONS",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A request to send. `url` is absolute, so a transport needs no base of its own.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    /// Header names are lowercase, which keeps lookups and test assertions predictable.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    /// The most response body bytes the caller will accept, when it has a limit. A transport
    /// SHOULD stop reading the body past it and fail with its own error, which reaches the caller
    /// as [`crate::Error::Transport`]; the crate checks the length again after `send` either way.
    pub max_body: Option<usize>,
}

impl fmt::Debug for Request {
    /// The `authorization` header carries the app password, so it is never printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: BTreeMap<&str, &str> =
            self.headers.iter().map(|(k, v)| (k.as_str(), if k == "authorization" { "<redacted>" } else { v.as_str() })).collect();
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .field("max_body", &self.max_body)
            .finish()
    }
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Request {
        Request { method, url: url.into(), headers: BTreeMap::new(), body: Vec::new(), max_body: None }
    }

    /// Set a header. The name is lowercased.
    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Request {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }

    #[must_use]
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Request {
        self.body = body.into();
        self
    }

    /// Set [`Request::max_body`].
    #[must_use]
    pub fn max_body(mut self, max: usize) -> Request {
        self.max_body = Some(max);
        self
    }

    /// Read a header back, by any casing.
    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }
}

/// A response as the transport saw it.
#[derive(Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl fmt::Debug for Response {
    /// The body can carry a credential (a Login Flow poll answer holds the app password), so only
    /// its length is printed; `authorization` and `set-cookie` values are redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: BTreeMap<&str, &str> = self
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), if k == "authorization" || k == "set-cookie" { "<redacted>" } else { v.as_str() }))
            .collect();
        f.debug_struct("Response").field("status", &self.status).field("headers", &headers).field("body_len", &self.body.len()).finish()
    }
}

impl Response {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Response {
        Response { status, headers: BTreeMap::new(), body: body.into() }
    }

    #[must_use]
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Response {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }

    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    /// 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as text, with invalid UTF-8 replaced. Server messages are for humans and for logs,
    /// so a stray byte must not cost us the whole message.
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }
}

/// A transport. One method, because that is all this crate needs.
///
/// An implementation is expected to send `req` as given, follow no redirects, and hand back
/// whatever came off the wire. Do not raise an error for a non-2xx status. The parsers here read
/// status codes themselves, and Nextcloud reports real outcomes through them, 207 Multi-Status
/// above all.
///
/// When [`Request::max_body`] is set, an implementation SHOULD stop reading the body once it is
/// longer than that and return its own error, which the crate reports as
/// [`crate::Error::Transport`]. The crate checks the length again after `send`, but only a
/// transport that stops early keeps an oversized body out of memory.
pub trait Http {
    /// The transport's own failure type, carried through [`crate::Error::Transport`].
    type Error: fmt::Display;

    fn send(&self, req: Request) -> Result<Response, Self::Error>;
}
