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

`search` and `shared_with_me` are tested against in-memory fakes only; neither has been run
against a live Nextcloud. The `SEARCH` body follows Nextcloud's documented `d:basicsearch`, and the
share fields follow the OCS sharing API, so check both on a real server before relying on them.
`search` puts the text into `d:like` as written. The escape behaviour of `d:like` is not
confirmed and was not researched, so none is sent: `%` and `_` in the search text may act as
wildcards and a search for `50%` may over-match. `Entry::lock` comes from `nc:lock`,
`nc:lock-owner`, `nc:lock-owner-displayname` and `nc:lock-owner-type` (1 means an app lock, per
Nextcloud's published WebDAV documentation); not checked against a live server. `shared_with_me`
keeps the first of several shares of one path and makes it writable if any of them is.
Paths from `shared_with_me` carry a leading slash, as `list` and `search` paths do.

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
