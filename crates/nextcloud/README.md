# craft-nextcloud

A Nextcloud client for the Craft apps: WebDAV files, WebDAV locks and Login Flow v2. It builds
requests and parses responses and opens no socket; the caller implements `http::Http`, so one
code path serves a native app and a WebAssembly build.

- `login::LoginFlow` signs in through the browser and yields an `AppPassword`. The password is a
  `Secret`, whose `Debug` never prints it.
- `dav::Files` lists (`craft_cloud::Entry`), reads (with a size cap and the ETag), writes, creates,
  locks, refreshes and unlocks, searches by name (`search`, a WebDAV `SEARCH`), lists what others
  shared with the user (`shared_with_me`, OCS), and reports whether the server offers locking.
- `Error` has `Conflict` (412), `Locked` (423), `Unauthorized` (401), `NotFound` (404),
  `Transport`, `Malformed`, and `Status` for any other answer.

Two behaviours matter to a document app. `create` sends `If-None-Match: *`, so a name already
taken is a `Conflict`, not a silent overwrite. `put` returns the ETag of the write itself, read
from the PUT response, and is `Malformed("no etag")` when the server sends none, never an ETag
borrowed from a later `PROPFIND`.

## Not verified

Nothing in this crate has run against a live Nextcloud yet. Every test runs against in-memory
fakes that answer the way the public documentation says a server does. In particular, none of
these has been checked on a real server:

- **Login Flow v2**: `LoginFlow::start` and `poll`, including 404 as "not yet".
- **PROPFIND**: the listing body, the properties returned, and absolute-URI versus path hrefs.
- **PUT and `OC-ETag`**: that the PUT response carries the new ETag, as `OC-ETag` or `ETag`.
- **LOCK, refresh and UNLOCK**: the `Lock-Token` and `Timeout` response headers, the refresh
  request, and the `If: (<token>)` header that a lock-aware `put` sends.
- **`capabilities().locking`**: whether `files.locking` is the right capability to read.
- **`search`**: the `SEARCH` body follows Nextcloud's documented `d:basicsearch`. The text goes
  into `d:like` as written. The escape behaviour of `d:like` is not confirmed and was not
  researched, so none is sent: `%` and `_` in the search text may act as wildcards, and a
  search for `50%` may over-match.
- **`shared_with_me`**: the share fields follow the OCS sharing API. It keeps the first of
  several shares of one path, and makes it writable if any of them is. Its paths carry a leading
  slash, as `list` and `search` paths do.
- **The lock-holder props**: `Entry::lock` comes from `nc:lock`, `nc:lock-owner`,
  `nc:lock-owner-displayname` and `nc:lock-owner-type`. The owner-type values are 0 for a user,
  1 for an app (Office or Text) and 2 for WebDAV, per Nextcloud's developer manual,
  <https://docs.nextcloud.com/server/stable/developer_manual/client_apis/WebDAV/basic.html>.

## Size limits

Every request tells the transport the most body bytes the crate will take, in
`Request::max_body`. A transport should stop reading past it, and the crate checks the length
again after the response arrives. The limits are:

- `get`: the caller's `max_bytes`.
- Listings (`list`, `search`, `shared_with_me`): 64 MiB, and at most 4,000,000 XML nodes. That
  is well over 100,000 entries.
- Login Flow, `capabilities`, and the responses to writes and locks: 1 MiB.

## Multistatus parsing

Multistatus is read with `craft-dav` (on `craft-xml`), which covers every property asked for,
including the `nc:` ones (`nc:lock`, `nc:lock-owner`), so this crate has no `quick-xml`
dependency. A DOCTYPE, an over-deep tree or a body that is not UTF-8 is `Malformed`. The `LOCK`
response is not a multistatus, so the lock token is read from the `Lock-Token` header only; a
server that omits it is `Malformed`.

## Origin

Extracted from [wdunn001/collab-craft](https://github.com/wdunn001/collab-craft) `crates/nextcloud`
at `d8cd69cbf9e4ee1950ba1b2dc5d0002eee639e32` (MIT OR Apache-2.0): `http.rs`, `login.rs`, `dav.rs`,
`error.rs` and the `Account` part of `lib.rs`. Sharing, contacts, mail and the provider are not
taken. Each changed file says what changed. See `ATTRIBUTION.md`.
