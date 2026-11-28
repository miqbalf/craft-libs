//! Local paths as text, compared by component, with each platform's rules. Paths are handled as
//! strings (not `std::path::Path`) so the Windows and macOS rules are tested on any host.

use serde::{Deserialize, Serialize};

/// The desktop platform whose path rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    /// Case-sensitive, `/` only.
    Linux,
    /// Case-insensitive (the default file system), `/` only; File Provider folders.
    MacOs,
    /// Case-insensitive, `\` or `/`, drive letters and UNC shares, `\\?\` verbatim paths.
    Windows,
}

impl Platform {
    /// The platform this build runs on (anything not macOS or Windows uses the Linux rules).
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    /// Whether names that differ only in case are the same file.
    pub fn case_insensitive(self) -> bool {
        matches!(self, Self::MacOs | Self::Windows)
    }

    /// The form of one name used to compare it.
    pub(crate) fn fold(self, name: &str) -> String {
        if self.case_insensitive() { name.to_lowercase() } else { name.to_string() }
    }
}

/// An absolute path split into its root (`/`, `C:`, `//`) and its names, with empty names and `.`
/// dropped and `..` applied. `None` for a relative or empty path, or one that climbs above its root.
pub(crate) fn components(platform: Platform, path: &str) -> Option<Vec<String>> {
    let (root, rest) = match platform {
        Platform::Linux | Platform::MacOs => ("/".to_string(), path.strip_prefix('/')?.to_string()),
        Platform::Windows => windows_root(&path.replace('\\', "/"))?,
    };
    let mut parts = vec![root];
    for name in rest.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                if parts.len() == 1 {
                    return None;
                }
                parts.pop();
            }
            _ => parts.push(name.to_string()),
        }
    }
    Some(parts)
}

/// The root and the rest of a Windows path already written with `/`: `C:/…`, a UNC share
/// `//server/share/…` (its root is `//`, so the server and share compare as names), or the
/// verbatim forms `//?/C:/…` and `//?/UNC/server/share/…` that canonicalizing gives.
fn windows_root(path: &str) -> Option<(String, String)> {
    if let Some(rest) = path.strip_prefix("//?/UNC/") {
        return Some(("//".to_string(), rest.to_string()));
    }
    let path = path.strip_prefix("//?/").unwrap_or(path);
    if let Some(rest) = path.strip_prefix("//") {
        return (!rest.is_empty()).then(|| ("//".to_string(), rest.to_string()));
    }
    let mut chars = path.chars();
    let (drive, colon, slash) = (chars.next()?, chars.next()?, chars.next()?);
    if !drive.is_ascii_alphabetic() || colon != ':' || slash != '/' {
        return None;
    }
    Some((format!("{drive}:"), chars.as_str().to_string()))
}

/// Whether `prefix` is `path` or one of its folders, by whole components.
pub(crate) fn starts_with(platform: Platform, path: &[String], prefix: &[String]) -> bool {
    prefix.len() <= path.len() && path.iter().zip(prefix).all(|(a, b)| a == b || platform.fold(a) == platform.fold(b))
}
