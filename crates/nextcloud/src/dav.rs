//! Files over WebDAV, with locks.
//!
//! Nextcloud serves each user's files at `/remote.php/dav/files/<user>/`. Listing is a `PROPFIND`
//! with `Depth: 1`; reading and writing are `GET` and `PUT`. Written from RFC 4918 and
//! Nextcloud's published WebDAV documentation.
//!
//! Changed from collab-craft: multistatus is read with `craft-dav` instead of a local quick-xml
//! reader; entries are [`craft_cloud::Entry`]; `create` sends `If-None-Match: *`; `put` and
//! `create` return the ETag of the write itself and fail rather than guess; `get` takes a size
//! cap and returns the ETag; `lock` refresh, `capabilities` and lock-aware `put` are new;
//! `stat`, ranged reads, `mkdir`, `delete` and `exists` were left out.

use craft_cloud::{Entry, EntryKind, LockHolder};
use craft_dav::{Limits, href, parse_multistatus_with, properties, responses};

use crate::error::{Error, Result};
use crate::http::{Http, Method, Response};
use crate::{Account, MAX_LISTING, MAX_LISTING_NODES, MAX_SMALL, encode_path, encode_segment, send};

/// The properties asked for. A small set keeps the answer small on a folder with thousands of
/// entries.
const PROPS: &str = r#"    <d:resourcetype/>
    <d:getcontentlength/>
    <d:getcontenttype/>
    <d:getlastmodified/>
    <d:getetag/>
    <oc:fileid/>
    <oc:permissions/>
    <oc:owner-id/>
    <nc:lock/>
    <nc:lock-owner/>
    <nc:lock-owner-displayname/>
    <nc:lock-owner-type/>"#;

/// The `PROPFIND` request body, asking for [`PROPS`].
fn propfind_body() -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">
  <d:prop>
{PROPS}
  </d:prop>
</d:propfind>"#
    )
}

/// How many results a search asks for.
const SEARCH_LIMIT: u32 = 200;

/// The `SEARCH` request body: files of `user_segment` (already percent-encoded) whose display
/// name contains `text` (already XML-escaped), asking for [`PROPS`].
fn search_body(user_segment: &str, text: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<d:searchrequest xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">
  <d:basicsearch>
    <d:select>
      <d:prop>
{PROPS}
      </d:prop>
    </d:select>
    <d:from>
      <d:scope>
        <d:href>/files/{user_segment}</d:href>
        <d:depth>infinity</d:depth>
      </d:scope>
    </d:from>
    <d:where>
      <d:like>
        <d:prop><d:displayname/></d:prop>
        <d:literal>%{text}%</d:literal>
      </d:like>
    </d:where>
    <d:limit>
      <d:nresults>{SEARCH_LIMIT}</d:nresults>
    </d:limit>
  </d:basicsearch>
</d:searchrequest>"#
    )
}

/// A held WebDAV lock, as returned by [`Files::lock`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    /// The token to send back as `If`, or to `UNLOCK` with.
    pub token: String,
    pub path: String,
    /// The timeout the server granted in its `Timeout` header, or the one asked for when it sent
    /// none it could be read from. [`Files::refresh`] asks for it again.
    pub timeout_secs: u32,
}

/// What the server can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether the server offers WebDAV file locking.
    pub locking: bool,
}

/// File operations against one account.
pub struct Files<'a> {
    account: &'a Account,
}

impl<'a> Files<'a> {
    pub fn new(account: &'a Account) -> Files<'a> {
        Files { account }
    }

    /// Absolute URL for a file under the user's files root, refusing the root itself: nothing is
    /// written to or locked there.
    fn file_url(&self, path: &str) -> Result<String> {
        if encode_path(path)?.is_empty() {
            return Err(Error::malformed("path names no file"));
        }
        self.url(path)
    }

    /// Absolute URL for a path under the user's files root. The root is a collection, so it keeps
    /// its slash: a server redirects it otherwise, and the transport follows no redirect.
    fn url(&self, path: &str) -> Result<String> {
        let rest = encode_path(path)?;
        if rest.is_empty() {
            return Ok(format!("{}/", self.account.files_root()));
        }
        Ok(format!("{}{rest}", self.account.files_root()))
    }

    /// List the direct children of a folder. The folder itself, which the protocol returns first,
    /// is dropped.
    pub fn list<H: Http>(&self, http: &H, path: &str) -> Result<Vec<Entry>> {
        let url = self.url(path)?;
        let req = self
            .account
            .request(Method::Propfind, &url)
            .header("depth", "1")
            .header("content-type", "application/xml; charset=utf-8")
            .header("accept", "application/xml")
            .body(propfind_body().into_bytes());
        let resp = send(http, req, MAX_LISTING)?;

        let own = decode(&href_path(&url));
        self.entries(&resp, Some(own.trim_end_matches('/')))
    }

    /// Search the user's files by display name. Blank `text` returns nothing without a request.
    ///
    /// A `SEARCH` on the DAV root, `d:like` on `d:displayname` with `%text%`, from the user's
    /// files at any depth, at most 200 results. Entries are as [`Files::list`] gives them.
    ///
    /// `text` goes into the `LIKE` as written. Which escape character `d:like` honours is not
    /// confirmed and was not researched, so none is sent: a `%` or `_` in `text` may act as a
    /// wildcard, and a search for `50%` may over-match. See the README's "Not verified" list.
    pub fn search<H: Http>(&self, http: &H, text: &str) -> Result<Vec<Entry>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let body = search_body(&encode_segment(self.account.user()), &craft_xml::esc(text));
        let req = self
            .account
            .request(Method::Search, format!("{}/remote.php/dav/", self.account.base()))
            .header("content-type", "text/xml")
            .header("accept", "application/xml")
            .body(body.into_bytes());
        let resp = send(http, req, MAX_LISTING)?;
        self.entries(&resp, None)
    }

    /// Items other people shared with this user, from the OCS sharing API. Paths are the ones in
    /// this user's own files, with a leading slash as `list` gives them.
    pub fn shared_with_me<H: Http>(&self, http: &H) -> Result<Vec<Entry>> {
        let url = format!("{}/ocs/v2.php/apps/files_sharing/api/v1/shares?shared_with_me=true&format=json", self.account.base());
        let req = self.account.request(Method::Get, url).header("accept", "application/json");
        let resp = send(http, req, MAX_LISTING)?;
        let v: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| Error::malformed(format!("shares: {e}")))?;
        match v.pointer("/ocs/meta/statuscode").and_then(serde_json::Value::as_i64) {
            Some(100 | 200) => {}
            Some(code) => return Err(Error::malformed(format!("shares: OCS status {code}"))),
            None => return Err(Error::malformed("shares: no OCS status")),
        }
        let data = v.pointer("/ocs/data").and_then(serde_json::Value::as_array).ok_or_else(|| Error::malformed("shares: data is not a list"))?;
        // A file shared directly and through a group comes back once per share. Keep the first,
        // and let any share that may write make the entry writable, so access is never lost.
        let mut out: Vec<Entry> = Vec::new();
        for entry in data.iter().filter_map(share_entry) {
            match out.iter_mut().find(|e| e.path == entry.path) {
                Some(first) => first.writable |= entry.writable,
                None => out.push(entry),
            }
        }
        Ok(out)
    }

    /// Entries of a multistatus, skipping the entry whose decoded path is `skip`, the files root
    /// itself, and any href outside the files root, which no path of this account can name.
    ///
    /// An href may be an absolute URI or a path (RFC 4918 §8.3); only its path is read.
    fn entries(&self, resp: &Response, skip: Option<&str>) -> Result<Vec<Entry>> {
        let limits = Limits { max_bytes: MAX_LISTING, max_nodes: MAX_LISTING_NODES, ..Limits::default() };
        let root = parse_multistatus_with(&resp.body, limits).map_err(|e| Error::malformed(e.to_string()))?;
        let files_root = decode(&href_path(&self.account.files_root()));
        let mut out = Vec::new();
        for response in responses(&root) {
            let Some(h) = href(response) else { continue };
            let decoded = decode(&href_path(&h));
            let trimmed = decoded.trim_end_matches('/');
            if skip.is_some_and(|own| trimmed == own) {
                continue;
            }
            let Some(relative) = under(trimmed, &files_root) else { continue };
            if relative.is_empty() {
                continue;
            }
            out.push(entry_from(response, relative));
        }
        Ok(out)
    }

    /// Read a file, refusing a body over `max_bytes`. Returns the bytes and the ETag of that
    /// version, unquoted.
    pub fn get<H: Http>(&self, http: &H, path: &str, max_bytes: usize) -> Result<(Vec<u8>, String)> {
        let req = self.account.request(Method::Get, self.url(path)?);
        // Only the caller's limit applies to a file, not the cap on parsed bodies.
        let resp = send(http, req, max_bytes)?;
        let etag = response_etag(&resp)?;
        Ok((resp.body, etag))
    }

    /// Write a file, replacing it.
    ///
    /// `if_match` is the ETag of the version being replaced; a stale one gives
    /// [`Error::Conflict`]. `lock` is a lock taken by [`Files::lock`]; without it a locked file
    /// gives [`Error::Locked`].
    ///
    /// Returns the ETag of *this* write, read from the PUT response. A follow-up `PROPFIND` would
    /// instead read whatever version exists then, which is somebody else's when they saved in
    /// between. A server that sends no ETag gives `Malformed("no etag")`.
    pub fn put<H: Http>(&self, http: &H, path: &str, bytes: Vec<u8>, if_match: Option<&str>, lock: Option<&Lock>) -> Result<String> {
        let mut req = self.account.request(Method::Put, self.file_url(path)?).header("content-type", "application/octet-stream").body(bytes);
        if let Some(etag) = if_match {
            req = req.header("if-match", quoted(etag));
        }
        if let Some(lock) = lock {
            req = req.header("if", format!("(<{}>)", lock.token));
        }
        response_etag(&send(http, req, MAX_SMALL)?)
    }

    /// Create a file, refusing to replace one that exists.
    ///
    /// `If-None-Match: *` makes this a create rather than an overwrite: a server honouring
    /// RFC 7232 answers 412, which is [`Error::Conflict`], when the name is taken. Returns the new
    /// file's ETag as [`Files::put`] does.
    pub fn create<H: Http>(&self, http: &H, path: &str, bytes: Vec<u8>) -> Result<String> {
        let req = self
            .account
            .request(Method::Put, self.file_url(path)?)
            .header("content-type", "application/octet-stream")
            .header("if-none-match", "*")
            .body(bytes);
        response_etag(&send(http, req, MAX_SMALL)?)
    }

    /// Take an exclusive write lock. `timeout_secs` is a request, and the server may shorten it:
    /// the returned [`Lock`] carries what the server granted. Another holder gives
    /// [`Error::Locked`].
    pub fn lock<H: Http>(&self, http: &H, path: &str, timeout_secs: u32) -> Result<Lock> {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<d:lockinfo xmlns:d="DAV:">
  <d:lockscope><d:exclusive/></d:lockscope>
  <d:locktype><d:write/></d:locktype>
  <d:owner>{}</d:owner>
</d:lockinfo>"#,
            craft_xml::esc(self.account.user())
        );
        let req = self
            .account
            .request(Method::Lock, self.file_url(path)?)
            .header("timeout", format!("Second-{timeout_secs}"))
            .header("content-type", "application/xml; charset=utf-8")
            .body(body.into_bytes());
        let resp = send(http, req, MAX_SMALL)?;
        let token = resp
            .get_header("lock-token")
            .map(|t| t.trim().trim_matches(['<', '>']).to_owned())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::malformed("no Lock-Token header in the lock response"))?;
        let timeout_secs = resp.get_header("timeout").and_then(granted_timeout).unwrap_or(timeout_secs);
        Ok(Lock { token, path: path.to_owned(), timeout_secs })
    }

    /// Extend a held lock by asking for its timeout again.
    pub fn refresh<H: Http>(&self, http: &H, lock: &Lock) -> Result<()> {
        let req = self
            .account
            .request(Method::Lock, self.url(&lock.path)?)
            .header("if", format!("(<{}>)", lock.token))
            .header("timeout", format!("Second-{}", lock.timeout_secs));
        send(http, req, MAX_SMALL)?;
        Ok(())
    }

    /// Release a lock taken by [`Files::lock`].
    pub fn unlock<H: Http>(&self, http: &H, lock: &Lock) -> Result<()> {
        let req = self.account.request(Method::Unlock, self.url(&lock.path)?).header("lock-token", format!("<{}>", lock.token));
        send(http, req, MAX_SMALL)?;
        Ok(())
    }

    /// Ask the server what it offers: `files.locking` of the OCS capabilities.
    pub fn capabilities<H: Http>(&self, http: &H) -> Result<Capabilities> {
        let url = format!("{}/ocs/v1.php/cloud/capabilities?format=json", self.account.base());
        let req = self.account.request(Method::Get, url).header("accept", "application/json");
        let resp = send(http, req, MAX_SMALL)?;
        let v: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| Error::malformed(format!("capabilities: {e}")))?;
        let locking = match v.pointer("/ocs/data/capabilities/files/locking") {
            None | Some(serde_json::Value::Null | serde_json::Value::Bool(false)) => false,
            Some(serde_json::Value::String(s)) => !s.is_empty() && s != "0",
            Some(_) => true,
        };
        Ok(Capabilities { locking })
    }
}

/// Build an [`Entry`] from one OCS share object, or `None` when it has no path.
///
/// OCS permissions are a bitmask: 2 is update, 4 create. A share that omits them is read-only.
fn share_entry(share: &serde_json::Value) -> Option<Entry> {
    let relative = share.get("path")?.as_str()?.trim_matches('/');
    if relative.is_empty() {
        return None;
    }
    // The same form `list` and `search` give, so a path and an id mean the same across sources.
    let path = format!("/{relative}");
    let text = |key: &str| share.get(key).and_then(serde_json::Value::as_str).map(str::to_owned).filter(|t| !t.is_empty());
    let number = |key: &str| share.get(key).and_then(serde_json::Value::as_i64);
    let is_dir = share.get("item_type").and_then(serde_json::Value::as_str) == Some("folder");
    let modified =
        number("item_mtime").or_else(|| number("stime")).and_then(|secs| chrono::DateTime::from_timestamp(secs, 0)).map(|d| d.to_rfc3339());
    let writable = number("permissions").is_some_and(|p| p & if is_dir { 2 | 4 } else { 2 } != 0);
    let room = share
        .get("item_source")
        .and_then(|v| v.as_str().map(str::to_owned).or_else(|| v.as_i64().map(|n| n.to_string())))
        .unwrap_or_else(|| path.clone());
    Some(Entry {
        id: path.clone(),
        room,
        name: path.rsplit('/').next().unwrap_or_default().to_owned(),
        path,
        kind: if is_dir { EntryKind::Folder } else { EntryKind::File },
        size: if is_dir { None } else { share.get("item_size").and_then(serde_json::Value::as_u64) },
        modified,
        mime: text("mimetype"),
        etag: None,
        writable,
        shared_by: text("uid_owner").or_else(|| text("displayname_owner")),
        lock: None,
    })
}

/// `path` relative to `root`, starting with `/` or empty for the root itself, or `None` when
/// `path` is not under `root`. The match ends at a segment boundary, so `/files/al` is not a
/// root of `/files/alice/x`.
fn under<'p>(path: &'p str, root: &str) -> Option<&'p str> {
    let rest = path.strip_prefix(root)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

/// The seconds of a `Timeout` header the server granted (`Second-120`), or `None` for
/// `Infinite` or anything unreadable.
fn granted_timeout(header: &str) -> Option<u32> {
    // RFC 4918 §10.7 allows a list; the first entry is the server's answer.
    let first = header.split(',').next()?.trim();
    let secs = first.get(..7).filter(|p| p.eq_ignore_ascii_case("second-")).and(first.get(7..))?;
    secs.parse().ok()
}

/// The text of a lock holder's name without markup. For a WebDAV lock (`nc:lock-owner-type` 2)
/// Nextcloud 35 names the holder with the `d:owner` content of the LOCK request as it was sent,
/// so `<d:owner><d:href>alice</d:href></d:owner>` comes back as the escaped text
/// `<d:href xmlns:d="DAV:">alice</d:href>`. Seen on workspace.treeo.id, 2026-11-27.
fn markup_free(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_owned()
}

/// Build an [`Entry`] from one `response`, whose decoded path relative to the files root is
/// `relative` (starting with `/`). Every property is optional.
fn entry_from(response: &craft_xml::El, relative: &str) -> Entry {
    let path = relative.to_owned();
    let name = path.rsplit('/').next().unwrap_or_default().to_owned();

    let mut is_dir = false;
    let (mut size, mut mime, mut modified, mut etag, mut file_id, mut permissions, mut owner) = (None, None, None, None, None, String::new(), None);
    let (mut lock, mut lock_owner, mut lock_name, mut lock_type) = (String::new(), None, None, String::new());
    for prop in properties(response) {
        let text = prop.text().trim().to_owned();
        match prop.name.as_str() {
            "d:resourcetype" => is_dir = prop.child("d:collection").is_some(),
            "d:getcontentlength" => size = text.parse().ok(),
            "d:getcontenttype" => mime = Some(text).filter(|t| !t.is_empty()),
            "d:getlastmodified" => modified = chrono::DateTime::parse_from_rfc2822(&text).ok().map(|d| d.to_rfc3339()),
            "d:getetag" => etag = Some(unquote(&text)).filter(|t| !t.is_empty()),
            "oc:fileid" => file_id = Some(text).filter(|t| !t.is_empty()),
            "oc:permissions" => permissions = text,
            "oc:owner-id" => owner = Some(text).filter(|t| !t.is_empty()),
            "nc:lock" => lock = text,
            "nc:lock-owner" => lock_owner = Some(text).filter(|t| !t.is_empty()),
            "nc:lock-owner-type" => lock_type = text,
            "nc:lock-owner-displayname" => lock_name = Some(markup_free(&text)).filter(|t| !t.is_empty()),
            _ => {}
        }
    }
    // `oc:permissions` carries `W` on a writable file and `C` or `K` on a folder that accepts
    // children; a server that omits it is treated as writable, as the launcher's default.
    let writable = if permissions.is_empty() {
        true
    } else if is_dir {
        permissions.contains('C') || permissions.contains('K')
    } else {
        permissions.contains('W')
    };
    // A file shared in carries the owner's id, which is not ours when it was shared.
    let shared_by = if permissions.contains('S') { owner } else { None };
    // Locked only when `nc:lock` says so; an owner prop alone does not make it so.
    // `nc:lock-owner-type` is 0 for a user, 1 for an app (Office or Text, holding it for the user
    // named in `nc:lock-owner`) and 2 for a WebDAV lock, per Nextcloud's developer manual:
    // https://docs.nextcloud.com/server/stable/developer_manual/client_apis/WebDAV/basic.html
    let locked = lock.eq_ignore_ascii_case("1") || lock.eq_ignore_ascii_case("true");
    let lock = locked.then(|| LockHolder { id: lock_owner, name: lock_name, app: lock_type == "1" });
    Entry {
        id: path.clone(),
        // The file id survives a rename; the path does not.
        room: file_id.unwrap_or_else(|| path.clone()),
        name,
        path,
        kind: if is_dir { EntryKind::Folder } else { EntryKind::File },
        size: if is_dir { None } else { size },
        modified,
        mime,
        etag,
        writable,
        shared_by,
        lock,
    }
}

/// The path part of an absolute URL, or the input when it has no scheme.
fn href_path(url: &str) -> String {
    match url.find("://") {
        Some(i) => match url[i + 3..].find('/') {
            Some(j) => url[i + 3 + j..].to_owned(),
            None => "/".to_owned(),
        },
        None => url.to_owned(),
    }
}

/// Percent-decode. An invalid escape is kept as written: a server that emits a bare `%` should
/// not cost the whole listing.
fn decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (bytes.get(i + 1).and_then(|b| hex_val(*b)), bytes.get(i + 2).and_then(|b| hex_val(*b)))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The ETag of a response: `OC-ETag` (Nextcloud) or `ETag`, unquoted.
fn response_etag(resp: &Response) -> Result<String> {
    resp.get_header("oc-etag").or_else(|| resp.get_header("etag")).map(unquote).filter(|e| !e.is_empty()).ok_or_else(|| Error::malformed("no etag"))
}

/// Strip the quotes and any weak marker from an ETag.
fn unquote(etag: &str) -> String {
    etag.trim().trim_start_matches("W/").trim_matches('"').to_owned()
}

/// Put quotes back on for an `If-Match`, unless the caller already did.
fn quoted(etag: &str) -> String {
    if etag.starts_with('"') || etag.starts_with("W/") { etag.to_owned() } else { format!("\"{etag}\"") }
}
