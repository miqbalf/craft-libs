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
    Response::new(207, format!(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">{entries}</d:multistatus>"#))
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
        let http = Fake::new(vec![Response::new(207, body)]);
        let err = Files::new(&account()).list(&http, "/").unwrap_err();
        assert!(matches!(err, Error::Malformed(_)), "{err:?}");
    }
}

#[test]
fn path_traversal_is_rejected() {
    let http = Fake::new(Vec::new());
    assert!(matches!(Files::new(&account()).list(&http, "/a/../b"), Err(Error::Malformed(_))));
    assert!(http.requests().is_empty());
}
