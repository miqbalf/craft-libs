//! One error type for the whole crate.
//!
//! Changed from collab-craft: the variants are the ones a launcher acts on differently (see the
//! crate README); the OCS and CardDAV cases are gone with their modules.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// The caller's transport failed. The message is whatever it reported.
    #[error("transport failed: {0}")]
    Transport(String),

    /// 401. The credential was rejected; the user has to sign in again.
    #[error("the server rejected the credential")]
    Unauthorized,

    /// 404.
    #[error("not found")]
    NotFound,

    /// 412. The file changed since the caller read it, or a create found the name taken.
    #[error("the file changed on the server, or already exists")]
    Conflict,

    /// 423. Somebody else holds a lock on the file.
    #[error("the file is locked")]
    Locked,

    /// Any other status the server answered with. `body` is truncated for logs.
    #[error("{method} returned HTTP {status}: {body}")]
    Status { method: &'static str, status: u16, body: String },

    /// The body was not the shape the protocol promises, or a path could not be used safely.
    #[error("malformed: {0}")]
    Malformed(String),
}

impl Error {
    /// Map a non-2xx response to the error a caller acts on.
    pub(crate) fn from_response(method: crate::http::Method, resp: &crate::http::Response) -> Error {
        match resp.status {
            401 => Error::Unauthorized,
            404 => Error::NotFound,
            412 => Error::Conflict,
            423 => Error::Locked,
            status => {
                // A server that answers with a whole HTML error page must not flood a log line.
                let body: String = resp.text().chars().take(400).collect();
                Error::Status { method: method.as_str(), status, body }
            }
        }
    }

    pub(crate) fn malformed(detail: impl Into<String>) -> Error {
        Error::Malformed(detail.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
