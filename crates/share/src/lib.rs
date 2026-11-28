//! Where a Craft app's Share button goes. Share does one thing: it opens Nextcloud's own sharing
//! for the open file. With no Nextcloud connected it says [`NO_NEXTCLOUD`] and does nothing else:
//! it never exports, mails, or shows a share dialog of the app's own.
//!
//! - **Web inside Nextcloud** ([`ShareRoute::NextcloudWebHost`]): the page hosting the app
//!   (`?host=parent`) said `share: true` in `<app>:host`; Share posts `<app>:share {}` to it
//!   (`craft-web-host`'s `Host::post_share`) and the page opens its sharing.
//! - **Desktop** ([`ShareRoute::NextcloudDesktop`]): the open file sits in a folder the Nextcloud
//!   desktop client syncs ([`desktop_route`]); Share opens [`sharing_url`] in the default browser.
//! - **Anything else** ([`ShareRoute::None`]): the app shows [`NO_NEXTCLOUD`], translated in its
//!   own catalog.
//!
//! Save As = Export lives in [`export`]: one format list per app, the file-naming rule, and the
//! delivery per platform (a download on the web, framed or not; the save dialog on the desktop).
//!
//! The crate depends on std and serde only: the apps link it from the miqbalf craft-libs fork next
//! to their storytold `craft-*` pins, so it must not pull in a second copy of any `craft-*` crate.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod config;
mod desktop;
pub mod export;
mod path;
mod url;

pub use desktop::{DesktopEnv, SystemEnv, config_candidates, desktop_route, desktop_route_in};
pub use path::Platform;
pub use url::{FILES_APP_PATH, sharing_url};

use serde::{Deserialize, Serialize};

/// What Share says when it has nowhere to go. The apps translate it in their own catalogs (this
/// exact English text is the key).
pub const NO_NEXTCLOUD: &str = "No Nextcloud connected";

/// Where Share goes for the open document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShareRoute {
    /// The Nextcloud page hosting the web build answered `share: true`: post `<app>:share {}` to it.
    NextcloudWebHost,
    /// The file is in a folder the Nextcloud desktop client syncs. `server` is the account's base
    /// URL without a trailing `/` (it may have a subpath), `user` the WebDAV user id (empty when the
    /// client's config names none), `remote_path` the file's path in that user's files, starting
    /// with `/`.
    NextcloudDesktop { server: String, user: String, remote_path: String },
    /// No Nextcloud connected: show [`NO_NEXTCLOUD`] and do nothing else.
    None,
}

/// What the app does when Share is pressed. There is deliberately no export or download here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareAction {
    /// Post `<app>:share {}` to the hosting page (`craft-web-host`'s `Host::post_share`).
    PostToHost,
    /// Open this URL in the default browser (Nextcloud Files with the file's sharing).
    OpenUrl(String),
    /// Show this text (translated by the app) in the status bar or a toast. Nothing else happens.
    Notice(&'static str),
}

impl ShareRoute {
    /// The route of a web build: the hosting page's `<app>:host` carried `share: true`, or not.
    pub fn web(host_share: bool) -> Self {
        if host_share { Self::NextcloudWebHost } else { Self::None }
    }

    /// Whether a Nextcloud is connected for this document.
    pub fn is_connected(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// What Share does on this route.
    pub fn action(&self) -> ShareAction {
        match self {
            Self::NextcloudWebHost => ShareAction::PostToHost,
            Self::NextcloudDesktop { server, remote_path, .. } => ShareAction::OpenUrl(sharing_url(server, remote_path)),
            Self::None => ShareAction::Notice(NO_NEXTCLOUD),
        }
    }
}

#[cfg(test)]
mod export_tests;
#[cfg(test)]
mod tests;
