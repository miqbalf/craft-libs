//! Behaviour of the Nextcloud client against an in-memory server that records what it was sent.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::convert::Infallible;

use craft_nextcloud::dav::Files;
use craft_nextcloud::http::{Http, Method, Request, Response};
use craft_nextcloud::login::LoginFlow;
use craft_nextcloud::{Account, Error};

/// Answers from a queue, in order, and keeps every request.
struct Fake {
    answers: RefCell<VecDeque<Response>>,
    seen: RefCell<Vec<Request>>,
}

impl Fake {
    fn new(answers: Vec<Response>) -> Fake {
        Fake { answers: RefCell::new(answers.into()), seen: RefCell::new(Vec::new()) }
    }
    fn requests(&self) -> Vec<Request> {
        self.seen.borrow().clone()
    }
}

impl Http for Fake {
    type Error = Infallible;
    fn send(&self, req: Request) -> Result<Response, Infallible> {
        self.seen.borrow_mut().push(req);
        Ok(self.answers.borrow_mut().pop_front().unwrap_or_else(|| Response::new(500, "no answer queued")))
    }
}

fn account() -> Account {
    Account::new("https://cloud.example.net/", "u", "pw-secret")
}

fn multistatus(entries: &str) -> Response {
    Response::new(
        207,
        format!(
            r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">{entries}</d:multistatus>"#
        ),
    )
}

fn item(href: &str, props: &str) -> String {
    format!("<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>")
}

#[test]
fn create_sends_if_none_match_star() {
    let http = Fake::new(vec![Response::new(201, "").header("ETag", "\"A\"")]);
    let etag = Files::new(&account()).create(&http, "/n.docx", b"x".to_vec()).unwrap();
    assert_eq!(etag, "A");
    let req = &http.requests()[0];
    assert_eq!(req.method, Method::Put);
    assert_eq!(req.get_header("if-none-match"), Some("*"));

    let taken = Fake::new(vec![Response::new(412, "")]);
    let err = Files::new(&account()).create(&taken, "/n.docx", Vec::new()).unwrap_err();
    assert!(matches!(err, Error::Conflict), "{err:?}");
}

#[test]
fn put_returns_etag_from_put_response() {
    // The second answer is what a later PROPFIND would say: somebody else saved in between.
    let http = Fake::new(vec![Response::new(204, "").header("ETag", "\"B\""), multistatus(&item("/x", "<d:getetag>\"C\"</d:getetag>"))]);
    let etag = Files::new(&account()).put(&http, "/a.docx", b"x".to_vec(), Some("A"), None).unwrap();
    assert_eq!(etag, "B");
    let seen = http.requests();
    assert_eq!(seen.len(), 1);
    assert!(seen.iter().all(|r| r.method != Method::Propfind));
    assert_eq!(seen[0].get_header("if-match"), Some("\"A\""));
}

#[test]
fn put_without_an_etag_is_malformed() {
    let http = Fake::new(vec![Response::new(204, "")]);
    let err = Files::new(&account()).put(&http, "/a", Vec::new(), None, None).unwrap_err();
    assert!(matches!(err, Error::Malformed(m) if m == "no etag"));
}

#[test]
fn put_with_stale_if_match_is_conflict() {
    let http = Fake::new(vec![Response::new(412, "")]);
    let err = Files::new(&account()).put(&http, "/a", Vec::new(), Some("old"), None).unwrap_err();
    assert!(matches!(err, Error::Conflict), "{err:?}");
}

#[test]
fn statuses_map_to_the_errors_a_caller_acts_on() {
    for (status, ok) in [(401, "Unauthorized"), (404, "NotFound"), (423, "Locked")] {
        let http = Fake::new(vec![Response::new(status, "")]);
        let err = Files::new(&account()).put(&http, "/a", Vec::new(), None, None).unwrap_err();
        assert!(format!("{err:?}").starts_with(ok), "{status}: {err:?}");
    }
}

#[test]
fn segments_are_percent_encoded() {
    let acct = account();
    let files = Files::new(&acct);

    let http = Fake::new(vec![multistatus("")]);
    files.list(&http, "/Ünïcode #1/a b%.docx").unwrap();
    assert_eq!(http.requests()[0].url, "https://cloud.example.net/remote.php/dav/files/u/%C3%9Cn%C3%AFcode%20%231/a%20b%25.docx");

    let folder = item("/remote.php/dav/files/u/%C3%9Cn%C3%AFcode%20%231/", "<d:resourcetype><d:collection/></d:resourcetype>");
    let listed = item(
        "/remote.php/dav/files/u/%C3%9Cn%C3%AFcode%20%231/a%20b%25.docx",
        "<d:resourcetype/><d:getcontentlength>5</d:getcontentlength><d:getetag>\"e1\"</d:getetag>",
    );
    let http = Fake::new(vec![multistatus(&format!("{folder}{listed}"))]);
    let entries = files.list(&http, "/Ünïcode #1").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a b%.docx");
    assert_eq!(entries[0].path, "/Ünïcode #1/a b%.docx");
    assert_eq!(entries[0].size, Some(5));
    assert_eq!(entries[0].etag.as_deref(), Some("e1"));
}

#[test]
fn list_drops_the_folder_itself_and_reads_properties() {
    let own = item("/remote.php/dav/files/u/Docs/", "<d:resourcetype><d:collection/></d:resourcetype>");
    let file = item(
        "/remote.php/dav/files/u/Docs/r.docx",
        "<d:resourcetype/><d:getlastmodified>Fri, 20 Nov 2026 10:00:00 GMT</d:getlastmodified><oc:fileid>77</oc:fileid><oc:permissions>SG</oc:permissions><oc:owner-id>bob</oc:owner-id>",
    );
    let http = Fake::new(vec![multistatus(&format!("{own}{file}"))]);
    let entries = Files::new(&account()).list(&http, "/Docs").unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!((e.path.as_str(), e.room.as_str()), ("/Docs/r.docx", "77"));
    assert!(!e.writable);
    assert_eq!(e.shared_by.as_deref(), Some("bob"));
    assert!(e.modified.as_deref().is_some_and(|m| m.starts_with("2026-11-20T10:00:00")));
}

#[test]
fn get_refuses_a_body_over_the_limit() {
    let http = Fake::new(vec![Response::new(200, vec![0u8; 10]).header("etag", "\"E\"")]);
    assert!(matches!(Files::new(&account()).get(&http, "/a", 5), Err(Error::Malformed(_))));
    let http = Fake::new(vec![Response::new(200, vec![0u8; 10]).header("etag", "\"E\"")]);
    let (body, etag) = Files::new(&account()).get(&http, "/a", 10).unwrap();
    assert_eq!((body.len(), etag.as_str()), (10, "E"));
}

#[test]
fn get_limit_is_the_callers_not_a_hidden_cap() {
    let forty_mib = 40 * 1024 * 1024;
    let http = Fake::new(vec![Response::new(200, vec![7u8; forty_mib]).header("etag", "\"E\"")]);
    let (body, _) = Files::new(&account()).get(&http, "/big", 100 * 1024 * 1024).unwrap();
    assert_eq!(body.len(), forty_mib);
    let http = Fake::new(vec![Response::new(200, vec![7u8; forty_mib]).header("etag", "\"E\"")]);
    assert!(matches!(Files::new(&account()).get(&http, "/big", 30 * 1024 * 1024), Err(Error::Malformed(_))));
}

#[test]
fn lock_refresh_unlock_use_the_token() {
    let http = Fake::new(vec![Response::new(200, "").header("Lock-Token", "<opaquelocktoken:t1>"), Response::new(200, ""), Response::new(204, "")]);
    let acct = account();
    let files = Files::new(&acct);
    let lock = files.lock(&http, "/a.docx", 300).unwrap();
    assert_eq!(lock.token, "opaquelocktoken:t1");
    files.refresh(&http, &lock).unwrap();
    files.unlock(&http, &lock).unwrap();
    let seen = http.requests();
    assert_eq!(seen[1].get_header("if"), Some("(<opaquelocktoken:t1>)"));
    assert_eq!(seen[2].get_header("lock-token"), Some("<opaquelocktoken:t1>"));
}

#[test]
fn capabilities_read_files_locking() {
    let on = Fake::new(vec![Response::new(200, r#"{"ocs":{"data":{"capabilities":{"files":{"locking":"1.0"}}}}}"#)]);
    assert!(Files::new(&account()).capabilities(&on).unwrap().locking);
    let off = Fake::new(vec![Response::new(200, r#"{"ocs":{"data":{"capabilities":{"files":{}}}}}"#)]);
    assert!(!Files::new(&account()).capabilities(&off).unwrap().locking);
}

const FLOW_START: &str =
    r#"{"poll":{"token":"tok","endpoint":"https://cloud.example.net/login/v2/poll"},"login":"https://cloud.example.net/login/v2/flow/x"}"#;

#[test]
fn login_flow_polls_until_done() {
    let http = Fake::new(vec![
        Response::new(200, FLOW_START),
        Response::new(404, ""),
        Response::new(200, r#"{"server":"https://cloud.example.net","loginName":"alice","appPassword":"app-pw"}"#),
    ]);
    let flow = LoginFlow::start(&http, "https://cloud.example.net/").unwrap();
    assert_eq!(flow.login_url(), "https://cloud.example.net/login/v2/flow/x");
    assert!(flow.poll(&http).unwrap().is_none());
    let done = flow.poll(&http).unwrap().unwrap();
    assert_eq!(done.login_name, "alice");
    assert_eq!(done.app_password.expose(), "app-pw");
    assert_eq!(http.requests()[1].url, "https://cloud.example.net/login/v2/poll");
}

#[test]
fn debug_never_prints_password() {
    let http =
        Fake::new(vec![Response::new(200, FLOW_START), Response::new(200, r#"{"server":"s","loginName":"alice","appPassword":"hunter2-app"}"#)]);
    let flow = LoginFlow::start(&http, "https://cloud.example.net").unwrap();
    let app = flow.poll(&http).unwrap().unwrap();
    assert!(!format!("{app:?}").contains("hunter2-app"));
    assert!(!format!("{flow:?}").contains("tok\""));
    assert!(!format!("{:?}", Account::new("https://x", "u", "pw-secret")).contains("pw-secret"));

    // A recorded request carries the Authorization header, which holds the app password.
    let http = Fake::new(vec![Response::new(201, "").header("ETag", "\"A\"")]);
    let acct = Account::new("https://x", "u", "pw-secret");
    Files::new(&acct).create(&http, "/a", Vec::new()).unwrap();
    let req = &http.requests()[0];
    assert!(req.get_header("authorization").is_some_and(|h| h.starts_with("Basic ")));
    let shown = format!("{req:?}");
    let encoded = req.get_header("authorization").unwrap_or_default().trim_start_matches("Basic ").to_owned();
    assert!(!shown.contains("pw-secret") && !shown.contains(&encoded), "{shown}");
}

/// The hostile-input sweep: each answer is an error, never a panic.
#[test]
fn hostile_multistatus_is_malformed() {
    let deep = format!("<d:multistatus xmlns:d=\"DAV:\">{}{}</d:multistatus>", "<a>".repeat(64), "</a>".repeat(64));
    let bodies: Vec<Vec<u8>> = vec![
        br#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY e "boom">]><d:multistatus xmlns:d="DAV:"><d:response>&e;</d:response></d:multistatus>"#
            .to_vec(),
        deep.into_bytes(),
        vec![b'<', b'd', 0xff, 0xfe, b'>'],
        b"<html>not dav</html>".to_vec(),
        b"<d:multistatus xmlns:d=\"DAV:\"><d:response>".to_vec(),
    ];
    for body in bodies {
        let http = Fake::new(vec![Response::new(207, body.clone())]);
        let err = Files::new(&account()).list(&http, "/").unwrap_err();
        assert!(matches!(err, Error::Malformed(_)), "{err:?}");
        // A SEARCH answer goes through the same parser.
        let http = Fake::new(vec![Response::new(207, body)]);
        let err = Files::new(&account()).search(&http, "x").unwrap_err();
        assert!(matches!(err, Error::Malformed(_)), "{err:?}");
    }
}

#[test]
fn hostile_ocs_body_is_malformed() {
    let bodies: Vec<Vec<u8>> = vec![
        br#"{"ocs":{"meta":{"statuscode":200},"data":[{"path":"/a""#.to_vec(),
        b"<html>not json</html>".to_vec(),
        br#"{"ocs":{"meta":{"statuscode":200},"data":"nope"}}"#.to_vec(),
        br#"{"ocs":{"data":[]}}"#.to_vec(),
        br#"{"ocs":{"meta":{"statuscode":403},"data":[]}}"#.to_vec(),
        vec![b'{', 0xff, 0xfe],
    ];
    for body in bodies {
        let http = Fake::new(vec![Response::new(200, body)]);
        let err = Files::new(&account()).shared_with_me(&http).unwrap_err();
        assert!(matches!(err, Error::Malformed(_)), "{err:?}");
    }
}

fn body_text(req: &Request) -> String {
    String::from_utf8_lossy(&req.body).into_owned()
}

#[test]
fn search_sends_basicsearch_scoped_to_user_files() {
    let acct = Account::new("https://cloud.example.net/", "alice smith", "pw-secret");
    let http = Fake::new(vec![multistatus("")]);
    Files::new(&acct).search(&http, "report").unwrap();
    let req = &http.requests()[0];
    assert_eq!(req.method, Method::Search);
    assert!(req.url.ends_with("/remote.php/dav/"), "{}", req.url);
    assert_eq!(req.get_header("content-type"), Some("text/xml"));
    let body = body_text(req);
    assert!(body.contains("d:basicsearch") && body.contains("d:like"));
    assert!(body.contains("%report%"), "{body}");
    // The scope carries the encoded user name, never the raw space.
    assert!(body.contains(&format!("/files/{}", craft_nextcloud::encode_segment("alice smith"))), "{body}");
    assert!(!body.contains("alice smith"));
    // The same properties `list` asks for.
    for prop in ["d:getetag", "oc:fileid", "oc:permissions", "oc:owner-id"] {
        assert!(body.contains(prop), "{prop}");
    }
}

#[test]
fn search_maps_results_like_list() {
    let folder = item("/remote.php/dav/files/u/Docs/Plans/", "<d:resourcetype><d:collection/></d:resourcetype>");
    let file = item(
        "/remote.php/dav/files/u/Docs/Plans/plan%20b.docx",
        "<d:resourcetype/><d:getcontentlength>9</d:getcontentlength><d:getetag>\"e2\"</d:getetag>",
    );
    let listing = Fake::new(vec![multistatus(&format!("{folder}{file}"))]);
    let searched = Fake::new(vec![multistatus(&format!("{folder}{file}"))]);
    let acct = account();
    let files = Files::new(&acct);
    let found = files.search(&searched, "plan").unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].path.as_str(), found[0].kind), ("/Docs/Plans", craft_cloud::EntryKind::Folder));
    assert_eq!((found[1].path.as_str(), found[1].name.as_str()), ("/Docs/Plans/plan b.docx", "plan b.docx"));
    assert_eq!(found[1].size, Some(9));
    // Parsed exactly as a listing of the parent would parse the same responses.
    let listed = files.list(&listing, "/Docs/Plans").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], found[1]);
}

#[test]
fn search_escapes_xml_in_text() {
    let text = "a<b&\"c";
    let http = Fake::new(vec![multistatus("")]);
    Files::new(&account()).search(&http, text).unwrap();
    let body = body_text(&http.requests()[0]);
    assert!(body.contains(&format!("%{}%", craft_xml::esc(text))), "{body}");
    assert!(!body.contains("a<b"), "{body}");
}

#[test]
fn blank_search_sends_nothing() {
    for text in ["", "   ", "\t\n"] {
        let http = Fake::new(Vec::new());
        assert!(Files::new(&account()).search(&http, text).unwrap().is_empty());
        assert!(http.requests().is_empty());
    }
}

const SHARES: &str = r#"{"ocs":{"meta":{"status":"ok","statuscode":200},"data":[
  {"id":"1","item_type":"file","path":"/Shared/report.docx","uid_owner":"bob","displayname_owner":"Bob B","mimetype":"application/vnd.openxmlformats-officedocument.wordprocessingml.document","item_size":1234,"stime":1700000000,"item_mtime":1700000500,"item_source":4711,"permissions":19},
  {"id":"2","item_type":"folder","path":"/Shared/Team","uid_owner":"carol","displayname_owner":"Carol","mimetype":"httpd/unix-directory","permissions":1}
]}}"#;

#[test]
fn shared_with_me_maps_ocs_shares() {
    let http = Fake::new(vec![Response::new(200, SHARES)]);
    let entries = Files::new(&account()).shared_with_me(&http).unwrap();
    let req = &http.requests()[0];
    assert_eq!(req.method, Method::Get);
    assert!(req.url.contains("/ocs/v2.php/apps/files_sharing/api/v1/shares"), "{}", req.url);
    assert!(req.url.contains("shared_with_me=true") && req.url.contains("format=json"));
    assert_eq!(req.get_header("ocs-apirequest"), Some("true"));

    assert_eq!(entries.len(), 2);
    let (file, folder) = (&entries[0], &entries[1]);
    assert_eq!(file.path, "/Shared/report.docx");
    assert_eq!(file.name, "report.docx");
    assert_eq!(file.kind, craft_cloud::EntryKind::File);
    assert_eq!(folder.kind, craft_cloud::EntryKind::Folder);
    assert_eq!(folder.name, "Team");
    assert_eq!(file.shared_by.as_deref(), Some("bob"));
    assert_eq!(folder.shared_by.as_deref(), Some("carol"));
    assert_eq!(file.size, Some(1234));
    assert_eq!(folder.size, None);
    assert!(file.mime.as_deref().is_some_and(|m| m.contains("wordprocessingml")));
    assert!(file.modified.as_deref().is_some_and(|m| m.starts_with("2023-11-14T")));
    // Read-only share versus one that may be updated.
    assert!(file.writable && !folder.writable);
    // The room is stable across a rename, so it is the file id, not the path.
    assert_eq!(file.room, "4711");
}

#[test]
fn shared_path_and_id_match_list_form() {
    let http = Fake::new(vec![Response::new(200, SHARES)]);
    let shared = Files::new(&account()).shared_with_me(&http).unwrap();
    let listed_item = item("/remote.php/dav/files/u/Shared/report.docx", "<d:resourcetype/>");
    let http = Fake::new(vec![multistatus(&format!(
        "{}{listed_item}",
        item("/remote.php/dav/files/u/Shared/", "<d:resourcetype><d:collection/></d:resourcetype>")
    ))]);
    let listed = Files::new(&account()).list(&http, "/Shared").unwrap();
    assert_eq!(shared[0].path, listed[0].path);
    assert_eq!(shared[0].id, listed[0].id);
}

#[test]
fn shared_with_me_unauthorized() {
    let http = Fake::new(vec![Response::new(401, "")]);
    let err = Files::new(&account()).shared_with_me(&http).unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");
}

#[test]
fn path_traversal_is_rejected() {
    let http = Fake::new(Vec::new());
    assert!(matches!(Files::new(&account()).list(&http, "/a/../b"), Err(Error::Malformed(_))));
    assert!(http.requests().is_empty());
}

#[test]
fn lock_reads_holder_and_kind() {
    let own = item("/remote.php/dav/files/u/Docs/", "<d:resourcetype><d:collection/></d:resourcetype>");
    let docs = "/remote.php/dav/files/u/Docs";
    let named = item(
        &format!("{docs}/a.docx"),
        "<d:resourcetype/><nc:lock>1</nc:lock><nc:lock-owner>alice</nc:lock-owner><nc:lock-owner-displayname>Alice Example</nc:lock-owner-displayname><nc:lock-owner-type>0</nc:lock-owner-type>",
    );
    let nobody = item(&format!("{docs}/b.docx"), "<d:resourcetype/><nc:lock>1</nc:lock>");
    let upper = item(&format!("{docs}/c.docx"), "<d:resourcetype/><nc:lock>TRUE</nc:lock><nc:lock-owner>alice</nc:lock-owner>");
    let app = item(&format!("{docs}/d.docx"), "<d:resourcetype/><nc:lock>1</nc:lock><nc:lock-owner-type>1</nc:lock-owner-type>");
    let off = item(&format!("{docs}/e.docx"), "<d:resourcetype/><nc:lock>0</nc:lock><nc:lock-owner>alice</nc:lock-owner>");
    let empty = item(&format!("{docs}/f.docx"), "<d:resourcetype/><nc:lock></nc:lock><nc:lock-owner>alice</nc:lock-owner>");
    let absent = item(&format!("{docs}/g.docx"), "<d:resourcetype/><nc:lock-owner>alice</nc:lock-owner>");
    let http = Fake::new(vec![multistatus(&format!("{own}{named}{nobody}{upper}{app}{off}{empty}{absent}"))]);
    let e = Files::new(&account()).list(&http, "/Docs").unwrap();
    let lock = |i: usize| e[i].lock.clone();
    let l = lock(0).unwrap();
    assert_eq!((l.id.as_deref(), l.name.as_deref(), l.app), (Some("alice"), Some("Alice Example"), false));
    // Locked with nobody named is still locked.
    assert_eq!(lock(1), Some(craft_cloud::LockHolder::default()));
    assert_eq!(lock(2).and_then(|l| l.id).as_deref(), Some("alice"));
    assert!(lock(3).is_some_and(|l| l.app && l.id.is_none()));
    // An owner prop alone does not make a file locked.
    assert_eq!((lock(4), lock(5), lock(6)), (None, None, None));
    let body = body_text(&http.requests()[0]);
    assert!(body.contains("nc:lock-owner-displayname") && body.contains("nc:lock-owner-type"));
}

#[test]
fn search_reads_lock_too() {
    let locked = item(
        "/remote.php/dav/files/u/a.docx",
        "<d:resourcetype/><nc:lock>1</nc:lock><nc:lock-owner-displayname>Alice Example</nc:lock-owner-displayname>",
    );
    let http = Fake::new(vec![multistatus(&locked)]);
    let entries = Files::new(&account()).search(&http, "a").unwrap();
    assert_eq!(entries[0].lock.as_ref().and_then(|l| l.name.as_deref()), Some("Alice Example"));
}

#[test]
fn a_file_shared_directly_and_through_a_group_is_listed_once() {
    let share =
        |path: &str, owner: &str, perms: u32| format!(r#"{{"path":"{path}","item_type":"file","uid_owner":"{owner}","permissions":{perms}}}"#);
    let body = |shares: Vec<String>| format!(r#"{{"ocs":{{"meta":{{"statuscode":200}},"data":[{}]}}}}"#, shares.join(","));
    let list = |shares: Vec<String>| Files::new(&account()).shared_with_me(&Fake::new(vec![Response::new(200, body(shares))])).unwrap();

    // Read-only first, writable second: still writable, and the first sharer is kept.
    let entries = list(vec![share("/Shared/r.docx", "bob", 1), share("/Shared/other.docx", "bob", 1), share("/Shared/r.docx", "carol", 19)]);
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["/Shared/r.docx", "/Shared/other.docx"]);
    assert!(entries[0].writable && !entries[1].writable);
    assert_eq!(entries[0].shared_by.as_deref(), Some("bob"));

    // Writable first, read-only second: stays writable.
    let entries = list(vec![share("/Shared/r.docx", "bob", 19), share("/Shared/r.docx", "carol", 1)]);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].writable);
}
