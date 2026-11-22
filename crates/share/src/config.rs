//! The Nextcloud desktop client's `nextcloud.cfg` (Qt `QSettings` .ini), read for the few keys
//! Share needs and nothing else.
//!
//! Only these `[Accounts]` keys are ever kept (`N` is the account, `M` the folder index):
//! `N\url`, `N\dav_user`, `N\webflow_user`, `N\user`, `N\fileProviderDomainIdentifier`,
//! `N\Folders\M\localPath`, `N\Folders\M\targetPath`, `N\FoldersWithPlaceholders\M\localPath`
//! and `N\FoldersWithPlaceholders\M\targetPath`. Every other key is dropped before its value is
//! even decoded, so passwords, tokens or proxy settings an older client may keep in the file never
//! reach memory the app holds on to. Nothing here logs.
//!
//! The file's layout is not an official contract of the client (its docs say it is not for manual
//! editing); the keys above come from public forum posts and a real install.

use std::collections::BTreeMap;

/// Longest config file read, in bytes. A real one is a few kilobytes.
pub const MAX_CONFIG_BYTES: u64 = 1 << 20;

/// Highest account or folder index taken; a larger one is ignored.
const MAX_INDEX: u32 = 9_999;

/// The accounts in a client config, in index order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientConfig {
    pub accounts: Vec<Account>,
}

/// One account of the desktop client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Server base URL, `http(s)://`, no trailing `/`; may have a subpath.
    pub url: String,
    /// The WebDAV user id (`dav_user`, else `webflow_user`, else `user`); empty when none is set.
    pub user: String,
    /// macOS File Provider domain id (`fileProviderDomainIdentifier`), when the account uses it.
    pub file_provider_domain: Option<String>,
    /// The classic and virtual-files synced folders, in index order.
    pub folders: Vec<SyncFolder>,
}

/// A local folder the client syncs with a remote folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFolder {
    /// The local folder as the client wrote it (forward slashes, even on Windows).
    pub local_path: String,
    /// The remote folder in the user's files; `/` for the whole account.
    pub target_path: String,
}

/// The accounts in `text`, a `nextcloud.cfg`. Never fails: what can't be read is left out. An
/// account without a usable `http(s)://` server URL is dropped, and so is a folder without a
/// `localPath`.
pub fn parse(text: &str) -> ClientConfig {
    let mut raw: BTreeMap<u32, RawAccount> = BTreeMap::new();
    let mut in_accounts = false;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_accounts = line == "[Accounts]";
            continue;
        }
        if !in_accounts || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        // The key decides first: a value is decoded only for an allowed key.
        let Some(slot) = Slot::of(key.trim()) else { continue };
        let Some(value) = decode(value.trim()) else { continue };
        let account = raw.entry(slot.account()).or_default();
        match slot {
            Slot::Url(_) => account.url = Some(value),
            Slot::DavUser(_) => account.dav_user = Some(value),
            Slot::WebflowUser(_) => account.webflow_user = Some(value),
            Slot::User(_) => account.user = Some(value),
            Slot::Domain(_) => account.domain = Some(value),
            Slot::Folder { kind, folder, local, .. } => {
                let pair = account.folders.entry((kind, folder)).or_default();
                if local {
                    pair.0 = Some(value);
                } else {
                    pair.1 = Some(value);
                }
            }
        }
    }
    let accounts = raw
        .into_values()
        .filter_map(|a| {
            let url = server_url(a.url.as_deref()?)?;
            let user = a.dav_user.or(a.webflow_user).or(a.user).unwrap_or_default();
            let folders = a
                .folders
                .into_values()
                .filter_map(|(local, target)| {
                    let local_path = local.filter(|l| !l.is_empty())?;
                    let target_path = target.filter(|t| !t.is_empty()).unwrap_or_else(|| "/".to_string());
                    Some(SyncFolder { local_path, target_path })
                })
                .collect();
            let file_provider_domain = a.domain.filter(|d| !d.is_empty());
            Some(Account { url, user, file_provider_domain, folders })
        })
        .collect();
    ClientConfig { accounts }
}

#[derive(Default)]
struct RawAccount {
    url: Option<String>,
    dav_user: Option<String>,
    webflow_user: Option<String>,
    user: Option<String>,
    domain: Option<String>,
    /// (classic 0 / placeholders 1, folder index) -> (localPath, targetPath)
    folders: BTreeMap<(u8, u32), (Option<String>, Option<String>)>,
}

/// An allowed key, by account.
enum Slot {
    Url(u32),
    DavUser(u32),
    WebflowUser(u32),
    User(u32),
    Domain(u32),
    Folder { account: u32, kind: u8, folder: u32, local: bool },
}

impl Slot {
    fn of(key: &str) -> Option<Self> {
        let parts: Vec<&str> = key.split('\\').collect();
        let account = index(parts.first()?)?;
        Some(match *parts.get(1..)? {
            ["url"] => Self::Url(account),
            ["dav_user"] => Self::DavUser(account),
            ["webflow_user"] => Self::WebflowUser(account),
            ["user"] => Self::User(account),
            ["fileProviderDomainIdentifier"] => Self::Domain(account),
            [group, folder, field] => {
                let kind = match group {
                    "Folders" => 0,
                    "FoldersWithPlaceholders" => 1,
                    _ => return None,
                };
                let local = match field {
                    "localPath" => true,
                    "targetPath" => false,
                    _ => return None,
                };
                Self::Folder { account, kind, folder: index(folder)?, local }
            }
            _ => return None,
        })
    }

    fn account(&self) -> u32 {
        match *self {
            Self::Url(a) | Self::DavUser(a) | Self::WebflowUser(a) | Self::User(a) | Self::Domain(a) => a,
            Self::Folder { account, .. } => account,
        }
    }
}

fn index(text: &str) -> Option<u32> {
    if text.is_empty() || text.len() > 4 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|i| *i <= MAX_INDEX)
}

/// A QSettings .ini value as text: surrounding quotes dropped, `\\`, `\"`, `\t`, `\n`, `\r`, `\0`
/// and `\xNNNN` (up to four hex digits) decoded. `None` for typed values (`@Variant(...)`,
/// `@ByteArray(...)`, anything starting with `@`) and for a broken escape.
fn decode(value: &str) -> Option<String> {
    let value = match value.strip_prefix('"') {
        Some(rest) => rest.strip_suffix('"')?,
        None => value,
    };
    if value.starts_with('@') {
        return None;
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            't' => out.push('\t'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            'x' => {
                let mut code = 0u32;
                let mut digits = 0;
                while digits < 4
                    && let Some(d) = chars.peek().and_then(|c| c.to_digit(16))
                {
                    code = code * 16 + d;
                    digits += 1;
                    chars.next();
                }
                if digits == 0 {
                    return None;
                }
                out.push(char::from_u32(code)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// A server base URL Share may open: `http://` or `https://`, a host, no space or control
/// character; the trailing `/` dropped. Anything else (`javascript:`, `file:`) is refused, so a
/// tampered config can't make Share open something that isn't a web server.
fn server_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end_matches('/');
    let rest = trimmed.strip_prefix("https://").or_else(|| trimmed.strip_prefix("http://"))?;
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() || trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(trimmed.to_string())
}
