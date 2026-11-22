//! Desktop: is the open file in a folder the Nextcloud desktop client syncs, and if so which
//! server, user and remote path is it.

use std::path::Path;

use crate::ShareRoute;
use crate::config::{Account, ClientConfig, MAX_CONFIG_BYTES, SyncFolder, parse};
use crate::path::{Platform, components, starts_with};

/// What [`desktop_route_in`] needs from the machine. [`SystemEnv`] is the real one; tests use
/// fixtures with fake values.
pub trait DesktopEnv {
    /// Whose path rules apply.
    fn platform(&self) -> Platform;
    /// The user's home folder.
    fn home(&self) -> Option<String>;
    /// An environment variable (`APPDATA`, `XDG_CONFIG_HOME`).
    fn var(&self, key: &str) -> Option<String>;
    /// The text of the config file at `path`, or `None` when it is missing, unreadable, not UTF-8
    /// or longer than [`crate::config::MAX_CONFIG_BYTES`].
    fn read_config(&self, path: &str) -> Option<String>;
    /// `path` with symlinks resolved, or `None` when that fails.
    fn canonical(&self, path: &str) -> Option<String>;
    /// macOS: the `com.apple.file-provider-domain-id` extended attribute of folder `dir`.
    fn file_provider_domain(&self, dir: &str) -> Option<String>;
}

/// The machine this app runs on.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnv;

impl DesktopEnv for SystemEnv {
    fn platform(&self) -> Platform {
        Platform::current()
    }

    fn home(&self) -> Option<String> {
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        self.var(key)
    }

    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }

    fn read_config(&self, path: &str) -> Option<String> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
            return None;
        }
        std::fs::read_to_string(path).ok()
    }

    fn canonical(&self, path: &str) -> Option<String> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        std::fs::canonicalize(path).ok()?.into_os_string().into_string().ok()
    }

    fn file_provider_domain(&self, dir: &str) -> Option<String> {
        file_provider_domain(dir)
    }
}

/// The File Provider domain attribute through the system's `xattr` tool (std has no extended
/// attribute call and the crate takes no other dependency).
#[cfg(target_os = "macos")]
fn file_provider_domain(dir: &str) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/xattr").args(["-p", "com.apple.file-provider-domain-id", dir]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty() && value.len() <= 512).then(|| value.to_string())
}

#[cfg(not(target_os = "macos"))]
fn file_provider_domain(_dir: &str) -> Option<String> {
    None
}

/// Where the client's config may be, most likely first, for `env`'s platform:
/// - macOS: the sandboxed client (33.0.0 and newer), then the older location;
/// - Linux: `$XDG_CONFIG_HOME` (when absolute), `~/.config`, then the Flatpak and Snap homes
///   (those two are inferred, not in the client's docs);
/// - Windows: `%APPDATA%`.
pub fn config_candidates(env: &dyn DesktopEnv) -> Vec<String> {
    const FILE: &str = "Nextcloud/nextcloud.cfg";
    let home = env.home();
    let mut out = Vec::new();
    match env.platform() {
        Platform::MacOs => {
            if let Some(home) = home {
                out.push(format!("{home}/Library/Containers/com.nextcloud.desktopclient/Data/Library/Preferences/{FILE}"));
                out.push(format!("{home}/Library/Preferences/{FILE}"));
            }
        }
        Platform::Linux => {
            if let Some(xdg) = env.var("XDG_CONFIG_HOME").filter(|x| x.starts_with('/')) {
                out.push(format!("{}/{FILE}", xdg.trim_end_matches('/')));
            }
            if let Some(home) = home {
                let plain = format!("{home}/.config/{FILE}");
                if !out.contains(&plain) {
                    out.push(plain);
                }
                out.push(format!("{home}/.var/app/com.nextcloud.desktopclient.nextcloud/config/{FILE}"));
                out.push(format!("{home}/snap/nextcloud-desktop-client/current/.config/{FILE}"));
            }
        }
        Platform::Windows => {
            if let Some(appdata) = env.var("APPDATA") {
                out.push(format!("{}\\{}", appdata.trim_end_matches('\\'), FILE.replace('/', "\\")));
            }
        }
    }
    out
}

/// The Share route for a document saved at `local_path` on this machine: the desktop client's
/// account and remote path when the file is in a folder it syncs, else [`ShareRoute::None`].
/// Reads the client's config (allowed keys only, see [`crate::config`]) each call.
pub fn desktop_route(local_path: &Path) -> ShareRoute {
    match local_path.to_str() {
        Some(path) => desktop_route_in(path, &SystemEnv),
        None => ShareRoute::None,
    }
}

/// [`desktop_route`] against `env`:
/// 1. the first readable config of [`config_candidates`];
/// 2. the synced folder (`Folders` or `FoldersWithPlaceholders`) that holds the file, by whole
///    path components (case-insensitively on macOS and Windows), the deepest one when several
///    do; the file and each folder are compared with symlinks resolved where that works;
/// 3. on macOS, else the File Provider domain folder (a folder directly in
///    `~/Library/CloudStorage`) whose `com.apple.file-provider-domain-id` ends in an account's
///    `fileProviderDomainIdentifier`.
pub fn desktop_route_in(local_path: &str, env: &dyn DesktopEnv) -> ShareRoute {
    let platform = env.platform();
    let Some(text) = config_candidates(env).iter().find_map(|p| env.read_config(p)) else { return ShareRoute::None };
    let config = parse(&text);
    let file = env.canonical(local_path).unwrap_or_else(|| local_path.to_string());
    let Some(file) = components(platform, &file) else { return ShareRoute::None };

    let mut best: Option<(usize, &Account, &SyncFolder)> = None;
    for account in &config.accounts {
        for folder in &account.folders {
            let local = env.canonical(&folder.local_path).unwrap_or_else(|| folder.local_path.clone());
            let Some(local) = components(platform, &local) else { continue };
            if starts_with(platform, &file, &local) && best.is_none_or(|(len, ..)| local.len() > len) {
                best = Some((local.len(), account, folder));
            }
        }
    }
    if let Some((len, account, folder)) = best {
        let target = folder.target_path.split(['/', '\\']).filter(|n| !n.is_empty());
        let rest = file.iter().skip(len).map(String::as_str);
        return route(account, target.chain(rest));
    }
    if platform == Platform::MacOs {
        return file_provider_route(env, &config, &file);
    }
    ShareRoute::None
}

fn file_provider_route(env: &dyn DesktopEnv, config: &ClientConfig, file: &[String]) -> ShareRoute {
    let platform = Platform::MacOs;
    let Some(cloud) = env.home().and_then(|home| components(platform, &format!("{home}/Library/CloudStorage"))) else {
        return ShareRoute::None;
    };
    if file.len() <= cloud.len() || !starts_with(platform, file, &cloud) {
        return ShareRoute::None;
    }
    let domain_dir = format!("/{}", file.iter().skip(1).take(cloud.len()).map(String::as_str).collect::<Vec<_>>().join("/"));
    let Some(value) = env.file_provider_domain(&domain_dir) else { return ShareRoute::None };
    let uuid = value.rsplit('/').next().unwrap_or_default();
    let account =
        config.accounts.iter().find(|a| a.file_provider_domain.as_deref().is_some_and(|d| !uuid.is_empty() && d.eq_ignore_ascii_case(uuid)));
    match account {
        Some(account) => route(account, file.iter().skip(cloud.len() + 1).map(String::as_str)),
        None => ShareRoute::None,
    }
}

fn route<'a>(account: &Account, names: impl Iterator<Item = &'a str>) -> ShareRoute {
    let remote_path = format!("/{}", names.collect::<Vec<_>>().join("/"));
    ShareRoute::NextcloudDesktop { server: account.url.clone(), user: account.user.clone(), remote_path }
}
