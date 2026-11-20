//! Signing in from a desktop app, through Login Flow v2.
//!
//! The app asks the server to begin a login and gets back a URL plus a poll token. The user opens
//! the URL in a browser and authenticates there; the app polls until the server hands back an app
//! password. Any login backend works unchanged, and the app never sees the account password.
//!
//! Nothing here sleeps or opens a socket. [`LoginFlow::poll`] answers "not yet" and leaves the
//! waiting to the caller.
//!
//! Changed from collab-craft: only the flow remains; `LoginFlow::start`, `login_url` and the
//! redacting [`Secret`] are new, the poll treats 404 as pending only, and both
//! answers are size-capped.
//!
//! Written from Nextcloud's published Login Flow v2 documentation.

use std::fmt;

use crate::error::{Error, Result};
use crate::http::{Http, Method, Request};

/// A string that never prints. `Debug` shows a placeholder, so a derived formatter cannot leak it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The secret itself. Call this only where it goes on the wire or into a keystore.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// What a finished login yields. Device-scoped and separately revocable, never the account
/// password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPassword {
    /// The server the credential belongs to, as the server itself named it.
    pub server: String,
    /// The login name to authenticate as. Not necessarily the display name.
    pub login_name: String,
    pub app_password: Secret,
}

/// A login the user has not finished yet.
#[derive(Clone, Debug)]
pub struct LoginFlow {
    login_url: String,
    endpoint: String,
    token: Secret,
}

impl LoginFlow {
    /// Ask the server to begin a login. `base` is the server root, such as
    /// `https://cloud.example.com`.
    pub fn start<H: Http>(http: &H, base: &str) -> Result<LoginFlow> {
        let url = format!("{}/index.php/login/v2", base.trim_end_matches('/'));
        let resp = crate::send(http, Request::new(Method::Post, url), crate::MAX_SMALL)?;
        let v = json(&resp.body)?;
        let login_url = string(&v, "login")?;
        let poll = v.get("poll").ok_or_else(|| Error::malformed("login flow: no poll object"))?;
        Ok(LoginFlow { login_url, endpoint: string(poll, "endpoint")?, token: Secret::new(string(poll, "token")?) })
    }

    /// Open this in a browser. The user authenticates there.
    pub fn login_url(&self) -> &str {
        &self.login_url
    }

    /// Check once whether the user has finished.
    ///
    /// `Ok(None)` means not yet, which the server reports as 404 for as long as the login is
    /// outstanding. The caller decides how long to keep asking; Nextcloud expires a flow after
    /// about twenty minutes. The server hands the credential over exactly once.
    pub fn poll<H: Http>(&self, http: &H) -> Result<Option<AppPassword>> {
        let req = Request::new(Method::Post, self.endpoint.clone())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("token={}", crate::encode_segment(self.token.expose())).into_bytes());
        // Not crate::send: here a 404 is the ordinary answer rather than a failure.
        let resp = crate::fetch(http, req, crate::MAX_SMALL)?;
        if resp.status == 404 {
            return Ok(None);
        }
        if !resp.is_success() {
            return Err(Error::from_response(Method::Post, &resp));
        }
        let resp = crate::within(resp, crate::MAX_SMALL)?;
        let v = json(&resp.body)?;
        Ok(Some(AppPassword {
            server: string(&v, "server")?,
            login_name: string(&v, "loginName")?,
            app_password: Secret::new(string(&v, "appPassword")?),
        }))
    }
}

fn json(body: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(body).map_err(|e| Error::malformed(format!("login flow: {e}")))
}

fn string(v: &serde_json::Value, key: &str) -> Result<String> {
    v.get(key).and_then(serde_json::Value::as_str).map(str::to_owned).ok_or_else(|| Error::malformed(format!("login flow: no {key}")))
}
