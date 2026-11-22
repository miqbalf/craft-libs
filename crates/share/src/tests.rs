//! Every route, with fixture configs (fake servers, users and paths only). The platform is a
//! parameter, so the macOS and Windows rules run on any host.

use std::collections::HashMap;

use crate::config::{ClientConfig, parse};
use crate::{DesktopEnv, NO_NEXTCLOUD, Platform, ShareAction, ShareRoute, config_candidates, desktop_route_in, sharing_url};

/// A machine made of fixtures.
#[derive(Default)]
struct Fake {
    platform: Option<Platform>,
    home: Option<String>,
    vars: HashMap<String, String>,
    files: HashMap<String, String>,
    canonical: HashMap<String, String>,
    xattrs: HashMap<String, String>,
}

impl Fake {
    fn new(platform: Platform, home: &str) -> Self {
        Self { platform: Some(platform), home: Some(home.into()), ..Self::default() }
    }
    fn file(mut self, path: &str, text: &str) -> Self {
        self.files.insert(path.into(), text.into());
        self
    }
    fn var(mut self, key: &str, value: &str) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }
    fn link(mut self, from: &str, to: &str) -> Self {
        self.canonical.insert(from.into(), to.into());
        self
    }
    fn xattr(mut self, dir: &str, value: &str) -> Self {
        self.xattrs.insert(dir.into(), value.into());
        self
    }
}

impl DesktopEnv for Fake {
    fn platform(&self) -> Platform {
        self.platform.unwrap_or(Platform::Linux)
    }
    fn home(&self) -> Option<String> {
        self.home.clone()
    }
    fn var(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }
    fn read_config(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }
    fn canonical(&self, path: &str) -> Option<String> {
        self.canonical.get(path).cloned()
    }
    fn file_provider_domain(&self, dir: &str) -> Option<String> {
        self.xattrs.get(dir).cloned()
    }
}

const LINUX_CFG: &str = "/home/alice/.config/Nextcloud/nextcloud.cfg";

/// R2's classic-sync example: two accounts, a whole-account folder and a subfolder pair.
const CLASSIC: &str = r"[General]
clientVersion=35.0.0

[Accounts]
0\url=https://cloud.example.org/
0\dav_user=alice
0\webflow_user=alice
0\authType=webflow
0\version=13
0\FoldersWithPlaceholders\1\localPath=/home/alice/Nextcloud/
0\FoldersWithPlaceholders\1\targetPath=/
0\FoldersWithPlaceholders\1\paused=false
0\Folders\2\localPath=/home/alice/Projects/
0\Folders\2\targetPath=/Work/Projects
1\url=https://other.example.net/nextcloud
1\dav_user=bob
1\Folders\1\localPath=/home/alice/OtherCloud/
1\Folders\1\targetPath=/
";

fn desktop(server: &str, user: &str, remote_path: &str) -> ShareRoute {
    ShareRoute::NextcloudDesktop { server: server.into(), user: user.into(), remote_path: remote_path.into() }
}

fn linux(cfg: &str) -> Fake {
    Fake::new(Platform::Linux, "/home/alice").file(LINUX_CFG, cfg)
}

// ---- the route and what Share does on it ----

#[test]
fn share_does_only_sharing_or_says_no_nextcloud() {
    assert_eq!(ShareRoute::NextcloudWebHost.action(), ShareAction::PostToHost);
    let route = desktop("https://cloud.example.org", "alice", "/Work/a.docx");
    assert_eq!(route.action(), ShareAction::OpenUrl(sharing_url("https://cloud.example.org", "/Work/a.docx")));
    // No Nextcloud: only the notice, which is the exact catalog key. There is no export action at all.
    assert_eq!(ShareRoute::None.action(), ShareAction::Notice(NO_NEXTCLOUD));
    assert_eq!(NO_NEXTCLOUD, "No Nextcloud connected");
}

#[test]
fn a_web_build_is_connected_only_when_its_host_shares() {
    assert_eq!(ShareRoute::web(true), ShareRoute::NextcloudWebHost);
    assert_eq!(ShareRoute::web(false), ShareRoute::None);
    assert!(ShareRoute::web(true).is_connected());
    assert!(desktop("https://cloud.example.org", "alice", "/a").is_connected());
    assert!(!ShareRoute::None.is_connected());
}

// ---- the config file ----

#[test]
fn the_config_gives_every_account_and_folder() {
    let config = parse(CLASSIC);
    assert_eq!(config.accounts.len(), 2);
    let (a, b) = (&config.accounts[0], &config.accounts[1]);
    assert_eq!((a.url.as_str(), a.user.as_str()), ("https://cloud.example.org", "alice"), "trailing / dropped");
    assert_eq!((b.url.as_str(), b.user.as_str()), ("https://other.example.net/nextcloud", "bob"), "subpath kept");
    let mut pairs: Vec<_> = a.folders.iter().map(|f| (f.local_path.as_str(), f.target_path.as_str())).collect();
    pairs.sort_unstable();
    assert_eq!(pairs, [("/home/alice/Nextcloud/", "/"), ("/home/alice/Projects/", "/Work/Projects")]);
}

#[test]
fn secrets_and_other_keys_are_never_kept() {
    let text = r"[Accounts]
0\url=https://cloud.example.org
0\dav_user=alice
0\password=FAKE-SECRET-1
0\http_oauth_token=FAKE-SECRET-2
0\webflow_pass=FAKE-SECRET-3
0\Folders\1\localPath=/home/alice/Nextcloud
0\Folders\1\journalPath=FAKE-SECRET-4
0\Folders\1\secret=FAKE-SECRET-5
[Proxy]
pass=FAKE-SECRET-6
host=FAKE-SECRET-7
";
    let config = parse(text);
    let kept = format!("{config:?}");
    for n in 1..=7 {
        assert!(!kept.contains(&format!("FAKE-SECRET-{n}")), "secret {n} leaked: {kept}");
    }
    assert_eq!(config.accounts.len(), 1, "the allowed keys still read");
}

#[test]
fn only_the_accounts_section_counts() {
    let text = "[General]\n0\\url=https://evil.example.com\n0\\Folders\\1\\localPath=/home/alice\n";
    assert_eq!(parse(text), ClientConfig::default());
}

#[test]
fn user_falls_back_from_dav_user_to_webflow_user_to_user() {
    let user = |keys: &str| parse(&format!("[Accounts]\n0\\url=https://c.example.org\n{keys}")).accounts[0].user.clone();
    assert_eq!(user("0\\user=u\n0\\webflow_user=w\n0\\dav_user=d\n"), "d");
    assert_eq!(user("0\\user=u\n0\\webflow_user=w\n"), "w");
    assert_eq!(user("0\\user=u\n"), "u");
    assert_eq!(user(""), "", "no user is still an account");
}

#[test]
fn only_web_server_urls_are_taken() {
    for bad in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "ftp://c.example.org",
        "",
        "https://",
        "https://c.example.org/a b",
        "https://c.example.org/\u{7}",
    ] {
        let text = format!("[Accounts]\n0\\url={bad}\n0\\Folders\\1\\localPath=/home/alice/Nextcloud\n");
        assert!(parse(&text).accounts.is_empty(), "{bad:?} must not be a server");
    }
    let text = "[Accounts]\n0\\url=http://192.0.2.7:8080\n";
    assert_eq!(parse(text).accounts.len(), 1, "plain http on a LAN is a server");
}

#[test]
fn qt_value_forms_are_decoded_or_skipped() {
    let text = "\u{feff}[Accounts]\r\n; a comment\r\n0\\url = \"https://cloud.example.org\"\r\n0\\dav_user=@Variant(\\0\\0\\0\\x7f)\r\n\
                0\\Folders\\1\\localPath=C:\\\\Users\\\\alice\\\\Caf\\xe9\r\n\
                0\\Folders\\2\\localPath=@ByteArray(/home/x)\r\n";
    let config = parse(text);
    let account = &config.accounts[0];
    assert_eq!(account.url, "https://cloud.example.org", "BOM, CRLF, spaces and quotes");
    assert_eq!(account.user, "", "a @Variant value is not a user");
    let locals: Vec<_> = account.folders.iter().map(|f| f.local_path.as_str()).collect();
    assert_eq!(locals, ["C:\\Users\\alice\\Café"], "escapes decoded, @ByteArray skipped");
    assert_eq!(account.folders[0].target_path, "/", "no targetPath is the whole account");
}

#[test]
fn hostile_configs_do_not_panic() {
    for text in [
        "[Accounts",
        "[Accounts]\n=",
        "[Accounts]\n\\\\\\=\\x",
        "[Accounts]\n0\\url=\"",
        "[Accounts]\n99999999999999999999\\url=https://a.example.org",
        "[Accounts]\n0\\Folders\\x\\localPath=/a",
        "[Accounts]\n0\\Folders\\1\\localPath=\\xZZZZ\\",
        "[Accounts]\n0\\url=https://a.example.org\n0\\Folders\\1\\localPath=\\x110000",
    ] {
        let _ = parse(text);
    }
}

#[test]
fn file_provider_accounts_have_their_domain_and_no_folders() {
    let text =
        "[Accounts]\n0\\url=https://cloud.example.org\n0\\dav_user=alice\n0\\fileProviderDomainIdentifier=11111111-2222-3333-4444-555555555555\n";
    let account = &parse(text).accounts[0];
    assert_eq!(account.file_provider_domain.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
    assert!(account.folders.is_empty());
}

// ---- where the config lives ----

#[test]
fn the_config_is_looked_for_where_each_platform_keeps_it() {
    let mac = config_candidates(&Fake::new(Platform::MacOs, "/Users/alice"));
    assert_eq!(
        mac[..2],
        [
            "/Users/alice/Library/Containers/com.nextcloud.desktopclient/Data/Library/Preferences/Nextcloud/nextcloud.cfg".to_string(),
            "/Users/alice/Library/Preferences/Nextcloud/nextcloud.cfg".to_string(),
        ],
        "the sandboxed client (33+) first, then the older one"
    );
    let linux = config_candidates(&Fake::new(Platform::Linux, "/home/alice"));
    assert_eq!(linux[0], LINUX_CFG);
    let xdg = config_candidates(&Fake::new(Platform::Linux, "/home/alice").var("XDG_CONFIG_HOME", "/home/alice/.cfg"));
    assert_eq!(xdg[0], "/home/alice/.cfg/Nextcloud/nextcloud.cfg");
    assert!(xdg.contains(&LINUX_CFG.to_string()));
    let relative_xdg = config_candidates(&Fake::new(Platform::Linux, "/home/alice").var("XDG_CONFIG_HOME", "cfg"));
    assert!(!relative_xdg.iter().any(|c| c.starts_with("cfg")), "a relative XDG_CONFIG_HOME is ignored");
    let windows = config_candidates(&Fake::new(Platform::Windows, "C:\\Users\\alice").var("APPDATA", "C:\\Users\\alice\\AppData\\Roaming"));
    assert_eq!(windows, ["C:\\Users\\alice\\AppData\\Roaming\\Nextcloud\\nextcloud.cfg"]);
    assert!(config_candidates(&Fake::default()).is_empty(), "no home, no config");
}

#[test]
fn the_newer_mac_config_wins_over_the_old_one() {
    let entry = |root: &str| format!("[Accounts]\n0\\url=https://{root}.example.org\n0\\Folders\\1\\localPath=/Users/alice/Nextcloud\n");
    let env = Fake::new(Platform::MacOs, "/Users/alice")
        .file("/Users/alice/Library/Preferences/Nextcloud/nextcloud.cfg", &entry("old"))
        .file("/Users/alice/Library/Containers/com.nextcloud.desktopclient/Data/Library/Preferences/Nextcloud/nextcloud.cfg", &entry("new"));
    assert_eq!(desktop_route_in("/Users/alice/Nextcloud/a.docx", &env), desktop("https://new.example.org", "", "/a.docx"));
}

// ---- mapping a local file ----

#[test]
fn a_file_in_a_synced_folder_maps_to_its_remote_path() {
    let env = linux(CLASSIC);
    assert_eq!(desktop_route_in("/home/alice/Nextcloud/Report.docx", &env), desktop("https://cloud.example.org", "alice", "/Report.docx"));
    assert_eq!(
        desktop_route_in("/home/alice/Projects/Q3/Plan 2.xlsx", &env),
        desktop("https://cloud.example.org", "alice", "/Work/Projects/Q3/Plan 2.xlsx"),
        "targetPath joined with the nested remainder"
    );
    assert_eq!(
        desktop_route_in("/home/alice/OtherCloud/deck.pptx", &env),
        desktop("https://other.example.net/nextcloud", "bob", "/deck.pptx"),
        "the second account"
    );
}

#[test]
fn a_file_outside_every_folder_has_no_nextcloud() {
    let env = linux(CLASSIC);
    for outside in [
        "/home/alice/Documents/a.docx",
        "/home/alice/Nextcloud2/a.docx",
        "/home/alice/NextcloudX",
        "/home/alice/Nextcloud/../Documents/a.docx",
        "relative/Nextcloud/a.docx",
        "",
    ] {
        assert_eq!(desktop_route_in(outside, &env), ShareRoute::None, "{outside:?}");
    }
}

#[test]
fn no_client_config_means_no_nextcloud() {
    let env = Fake::new(Platform::Linux, "/home/alice");
    assert_eq!(desktop_route_in("/home/alice/Nextcloud/a.docx", &env), ShareRoute::None);
    let empty = linux("[Accounts]\n");
    assert_eq!(desktop_route_in("/home/alice/Nextcloud/a.docx", &empty), ShareRoute::None);
}

#[test]
fn the_deepest_folder_wins_across_accounts() {
    let text = r"[Accounts]
0\url=https://cloud.example.org
0\dav_user=alice
0\Folders\1\localPath=/home/alice/Nextcloud
0\Folders\1\targetPath=/
1\url=https://other.example.net
1\dav_user=bob
1\Folders\1\localPath=/home/alice/Nextcloud/Shared/Team/
1\Folders\1\targetPath=/Team
";
    let env = linux(text);
    assert_eq!(desktop_route_in("/home/alice/Nextcloud/Shared/Team/a.docx", &env), desktop("https://other.example.net", "bob", "/Team/a.docx"));
    assert_eq!(desktop_route_in("/home/alice/Nextcloud/Shared/b.docx", &env), desktop("https://cloud.example.org", "alice", "/Shared/b.docx"));
}

#[test]
fn linux_paths_are_case_sensitive() {
    let env = linux(CLASSIC);
    assert_eq!(desktop_route_in("/home/alice/nextcloud/a.docx", &env), ShareRoute::None);
}

#[test]
fn mac_paths_match_in_any_case_and_keep_the_files_own_case() {
    let text = "[Accounts]\n0\\url=https://cloud.example.org\n0\\dav_user=alice\n0\\Folders\\1\\localPath=/Users/alice/Nextcloud/\n0\\Folders\\1\\targetPath=/Ünïcode\n";
    let env = Fake::new(Platform::MacOs, "/Users/alice").file("/Users/alice/Library/Preferences/Nextcloud/nextcloud.cfg", text);
    assert_eq!(desktop_route_in("/USERS/Alice/NEXTCLOUD/Sub/Doc.DOCX", &env), desktop("https://cloud.example.org", "alice", "/Ünïcode/Sub/Doc.DOCX"));
}

#[test]
fn windows_paths_match_by_drive_and_any_separator_and_case() {
    let text = r"[Accounts]
0\url=https://cloud.example.org
0\dav_user=alice
0\FoldersWithPlaceholders\1\localPath=C:/Users/alice/Nextcloud/
0\FoldersWithPlaceholders\1\targetPath=/
0\FoldersWithPlaceholders\1\virtualFilesMode=wincfapi
1\url=https://other.example.net
1\dav_user=bob
1\Folders\1\localPath=//fileserver/share/Team
1\Folders\1\targetPath=/Team
";
    let env = Fake::new(Platform::Windows, "C:\\Users\\alice")
        .var("APPDATA", "C:\\Users\\alice\\AppData\\Roaming")
        .file("C:\\Users\\alice\\AppData\\Roaming\\Nextcloud\\nextcloud.cfg", text);
    let want = desktop("https://cloud.example.org", "alice", "/Sub/a.docx");
    assert_eq!(desktop_route_in("C:\\Users\\Alice\\nextcloud\\Sub\\a.docx", &env), want);
    assert_eq!(desktop_route_in("c:/users/alice/Nextcloud/Sub/a.docx", &env), want);
    assert_eq!(desktop_route_in("\\\\?\\C:\\Users\\alice\\Nextcloud\\Sub\\a.docx", &env), want, "a canonicalized verbatim path");
    assert_eq!(desktop_route_in("D:\\Users\\alice\\Nextcloud\\Sub\\a.docx", &env), ShareRoute::None, "another drive");
    assert_eq!(desktop_route_in("C:\\Users\\alice\\Nextcloud2\\a.docx", &env), ShareRoute::None);
    assert_eq!(
        desktop_route_in("\\\\fileserver\\share\\Team\\x.xlsx", &env),
        desktop("https://other.example.net", "bob", "/Team/x.xlsx"),
        "a UNC share"
    );
    assert_eq!(desktop_route_in("\\\\?\\UNC\\fileserver\\share\\Team\\x.xlsx", &env), desktop("https://other.example.net", "bob", "/Team/x.xlsx"));
    assert_eq!(desktop_route_in("Users\\alice\\Nextcloud\\a.docx", &env), ShareRoute::None, "relative");
}

#[test]
fn a_symlink_into_a_synced_folder_counts_as_inside() {
    let env = linux(CLASSIC).link("/home/alice/Desktop/link.docx", "/home/alice/Nextcloud/Real.docx");
    assert_eq!(desktop_route_in("/home/alice/Desktop/link.docx", &env), desktop("https://cloud.example.org", "alice", "/Real.docx"));
    // And a synced folder that is itself reached through a link.
    let env = linux(CLASSIC).link("/home/alice/Projects/", "/data/projects");
    assert_eq!(desktop_route_in("/data/projects/a.md", &env), desktop("https://cloud.example.org", "alice", "/Work/Projects/a.md"));
}

#[test]
fn the_folder_itself_maps_to_its_target() {
    let env = linux(CLASSIC);
    assert_eq!(desktop_route_in("/home/alice/Projects", &env), desktop("https://cloud.example.org", "alice", "/Work/Projects"));
}

// ---- macOS File Provider ----

const DOMAIN: &str = "11111111-2222-3333-4444-555555555555";
const MAC_CFG: &str = "/Users/alice/Library/Containers/com.nextcloud.desktopclient/Data/Library/Preferences/Nextcloud/nextcloud.cfg";
const CLOUD: &str = "/Users/alice/Library/CloudStorage/Nextcloud-cloud.example.org-alice";

fn file_provider() -> Fake {
    let text = format!("[Accounts]\n0\\url=https://cloud.example.org\n0\\dav_user=alice\n0\\fileProviderDomainIdentifier={DOMAIN}\n");
    Fake::new(Platform::MacOs, "/Users/alice").file(MAC_CFG, &text)
}

#[test]
fn a_file_provider_file_maps_through_its_domain_id() {
    let env = file_provider().xattr(CLOUD, &format!("com.nextcloud.desktopclient.FileProviderExt/{DOMAIN}"));
    assert_eq!(desktop_route_in(&format!("{CLOUD}/Docs/Q3/a.docx"), &env), desktop("https://cloud.example.org", "alice", "/Docs/Q3/a.docx"));
    let upper = file_provider().xattr(CLOUD, &format!("com.nextcloud.desktopclient.FileProviderExt/{}", DOMAIN.to_uppercase()));
    assert_eq!(desktop_route_in(&format!("{CLOUD}/a.docx"), &upper), desktop("https://cloud.example.org", "alice", "/a.docx"), "uuid case");
}

#[test]
fn a_stale_or_foreign_file_provider_folder_has_no_nextcloud() {
    let stale = file_provider().xattr(CLOUD, "com.nextcloud.desktopclient.FileProviderExt/99999999-0000-0000-0000-000000000000");
    assert_eq!(desktop_route_in(&format!("{CLOUD}/a.docx"), &stale), ShareRoute::None, "a domain the config doesn't name");
    let none = file_provider();
    assert_eq!(desktop_route_in(&format!("{CLOUD}/a.docx"), &none), ShareRoute::None, "no attribute");
    let deeper = file_provider().xattr(&format!("{CLOUD}/Docs"), &format!("x/{DOMAIN}"));
    assert_eq!(desktop_route_in(&format!("{CLOUD}/Docs/a.docx"), &deeper), ShareRoute::None, "only a folder directly in CloudStorage is a domain");
    let elsewhere = file_provider().xattr("/Users/alice/Documents", &format!("x/{DOMAIN}"));
    assert_eq!(desktop_route_in("/Users/alice/Documents/a.docx", &elsewhere), ShareRoute::None, "outside CloudStorage");
}

#[test]
fn the_file_provider_is_only_asked_on_a_mac() {
    let text = format!("[Accounts]\n0\\url=https://cloud.example.org\n0\\fileProviderDomainIdentifier={DOMAIN}\n");
    let env = linux(&text).xattr("/home/alice/Library/CloudStorage/NC", &format!("x/{DOMAIN}"));
    assert_eq!(desktop_route_in("/home/alice/Library/CloudStorage/NC/a.docx", &env), ShareRoute::None);
}

// ---- the sharing URL ----

#[test]
fn the_sharing_url_opens_the_files_folder() {
    assert_eq!(sharing_url("https://cloud.example.org/", "/Work/Q3 report.docx"), "https://cloud.example.org/apps/files/?dir=/Work");
    assert_eq!(sharing_url("https://other.example.net/nextcloud", "/a.docx"), "https://other.example.net/nextcloud/apps/files/?dir=/");
    assert_eq!(sharing_url("https://cloud.example.org", "/"), "https://cloud.example.org/apps/files/?dir=/");
}

#[test]
fn the_sharing_url_encodes_every_path_part() {
    let url = sharing_url("https://cloud.example.org", "/Ünï & co/50%+#?=x/a b.docx");
    assert_eq!(url, "https://cloud.example.org/apps/files/?dir=/%C3%9Cn%C3%AF%20%26%20co/50%25%2B%23%3F%3Dx");
    assert_eq!(sharing_url("https://cloud.example.org", "//a//b.docx"), "https://cloud.example.org/apps/files/?dir=/a", "empty parts dropped");
}

#[test]
fn the_sharing_url_never_opens_the_file_itself() {
    // R1: `openfile=true` can open the file in a viewer or editor instead of its sharing.
    let url = sharing_url("https://cloud.example.org", "/Work/openfile=true.docx");
    assert!(!url.contains("openfile"), "{url}");
}

#[test]
fn a_route_survives_json() {
    for route in [ShareRoute::NextcloudWebHost, desktop("https://cloud.example.org", "alice", "/a.docx"), ShareRoute::None] {
        let json = serde_json::to_string(&route).expect("serialises");
        assert_eq!(serde_json::from_str::<ShareRoute>(&json).expect("parses"), route);
    }
}
