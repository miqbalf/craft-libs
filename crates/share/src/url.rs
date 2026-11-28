//! The Nextcloud Files URL Share opens on the desktop.
//!
//! Task R1 settled the form. Without a login the desktop knows no file id, so it opens the file's
//! folder: `{server}/index.php/apps/files/?dir=<parent folder>`. `index.php` keeps it working on
//! servers with and without pretty URLs. It never adds `openfile=true`, which can
//! open the file in a viewer or editor instead of showing its sharing.
//!
//! NOTE (file id): if a file id becomes available later (for example from the desktop client's
//! sync journal), `{server}/index.php/f/{fileId}` lands on the file with its Sharing tab open (R1: Files
//! redirects it to `/apps/files/files/{fileId}?dir=…&opendetails=`). Change only this builder.
//! The web host does not use this URL: the Nextcloud page builds its own from the file id.

/// The Files app's path below the server base URL.
pub const FILES_APP_PATH: &str = "/index.php/apps/files/";

/// The Nextcloud Files URL for `remote_path` (a file in the user's files, starting with `/`) on
/// `server` (base URL, a trailing `/` is ignored): `{server}{FILES_APP_PATH}?dir=<parent folder>`.
/// Each folder name is percent-encoded; the `/` between them stays.
pub fn sharing_url(server: &str, remote_path: &str) -> String {
    let server = server.trim_end_matches('/');
    let mut names: Vec<&str> = remote_path.split('/').filter(|n| !n.is_empty()).collect();
    names.pop();
    let dir: String = names.iter().map(|n| format!("/{}", encode(n))).collect();
    let dir = if dir.is_empty() { "/".to_string() } else { dir };
    format!("{server}{FILES_APP_PATH}?dir={dir}")
}

/// `name` percent-encoded as UTF-8, keeping only RFC 3986 unreserved characters.
fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
