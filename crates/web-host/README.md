# craft-web-host

The host messages between a Craft app's web build and the page that hosts it, as one contract and one bridge for every app (storytold/craft-libs#358). A page frames the app with `?host=parent`, or runs it directly with `data-host="page"` on its canvas; either way the same messages pass files in, saves out, and unsaved-changes state, so a page can host any of the apps the same way.

The crate has two parts:

- **The contract** (all targets): the message kinds and their `<app>:<kind>` types, the contract version, how the hosting mode is read from the URL and the canvas, and the clean-up of file names a host passes in. Pure, tested natively.
- **The bridge** (`web`, wasm32 only): `Host::connect(app, canvas)` finds the host, `listen` hears its messages, and `post_ready` (or `post_ready_with` for an app with its own Share button), `post_save`, `post_dirty`, `post_failed`, `post_handoff` and `post_share` answer it. `web::download(name, mime, bytes)` downloads a copy in the browser (a `Blob` and a hidden `<a download>`). Only the same-origin parent window is heard and answered; with a parent on another origin the app runs as if unframed.

## Messages

| Kind | Direction | Fields | Meaning |
|---|---|---|---|
| `ready` | app → host | `version`, `appVersion?`, `file?`, `shareButton?` | the app can take a document; `file: true` means it reads a `File`/`Blob` in slices; `shareButton: true` means the app has its own Share button, so the host shows none |
| `host` | host → app | `version`, `handoff?`, `saves?`, `share?` | what the host supports; `handoff: true` means it routes hand-offs; `share: true` means it opens Nextcloud sharing on `share` |
| `open` | host → app | `name`, and `bytes` or `file` + `size` | open a document as File › Open would |
| `save` | app → host | `name`, `bytes`, `id?`, `saveAs?` | Save, Save As or Export: the host stores the file |
| `saved` | host → app | `id`, `name?` | the save with that `id` is stored; the app clears dirty and takes `name` when the host kept it under another name |
| `save-failed` | host → app | `id`, `error` | the host could not store that save; the app stays unsaved and shows `error` |
| `fonts` | host → app | `fonts` (`[{name, url}]`) | same-origin font files to fetch (`web::fetch_font`) and install; at most 64 entries, 20 MB each |
| `dirty` | app → host | `dirty` | unsaved changes appeared or went away |
| `failed` | app → host | `error`, `id?` | a document or a save could not be taken |
| `handoff` | app → host | `name`, and `bytes` or `file` + `size` | this file belongs to another app |
| `handoff-done` | host → app | `name`, `app` | the host gave it to the app that edits it |
| `share` | app → host | none | Share was pressed: open the host's sharing for the open file. Sent only after `host` said `share: true`; otherwise the app says "No Nextcloud connected" (`craft-share`) |

With `?host=parent` a message is an object with `type` (`pdfcraft:open`) and the fields, sent with same-origin `postMessage`; `bytes` go as a transferred `ArrayBuffer`. With `data-host="page"` it is a `CustomEvent` of that type on the canvas, with the fields in `detail`.

### Live collaboration (`craft-live/1`)

Unprefixed, parent mode only, spoken by the Nextcloud page. The host waits for an unprefixed `{type:"ready"}` (`post_live_ready`; the page also posts after the iframe loads), then posts the grant; `listen_live` hears it.

| Type | Direction | Fields |
|---|---|---|
| `live` | host → app | `ticket`, `relayUrl` (absolute `ws(s)://`), `room`, `fileUrl`, `fileName`, `proto: "craft-live/1"` |
| `live-state` | app → host | `state` (`connecting`, `live`, `readonly`, `reconnecting`, `refused`, `broken`: ended for good, reopen), `reason?`, `savedAt?`, `peers` (`[{id, display}]`) |

An unknown `proto` is a typed `LiveError::UnsupportedProto`, never a panic. The ticket is a credential: `LiveMessage`'s `Debug` hides it and the app must not log it. The host sends a fresh `live` before the ticket expires; treat each one as authoritative.

Compatibility with the bridges shipped before this crate:

- `host` may carry `saves: true`: the host stores saves, so the app sends `save` with an `id` (`PendingSaves::start`) and marks the document saved only when `saved` arrives for it (`PendingSaves::finish`). Without `saves` the app keeps its earlier behaviour (download, never marked saved). A live document is saved by its live session, not by `save`.
- Fields marked `?` are optional. A host answers `saved` only for a `save` that carries an `id`; an app that sends none treats its save as done once it is posted.
- A host that never sends `host` gets the behaviour of the earlier bridges.
- `size` beside `file` is for hosts and older bridges; this crate takes a `Blob`'s length from the `Blob` itself.
- A page host may dispatch an event while the app's handler runs (a `saved` answering the `save` it just posted). The bridge holds it and calls the handler again once the running call returns, so messages arrive one at a time and in order.
- `version` is `"1"`. The earlier bridges put the app's own release in it (`"0.14.0"`), so a host treats a `version` it doesn't know as `"1"`. The crate sends the release as `appVersion`.

## Use in an app's web shell

```rust
#[cfg(target_arch = "wasm32")]
if let Some(host) = craft_web_host::web::Host::connect("pdfcraft", &canvas) {
    host.listen(move |message| match message {
        craft_web_host::web::Message::Open { name, data } => {
            // Check data.size() against the app's limit before copying it in.
        }
        craft_web_host::web::Message::Saved { id } => { /* clear dirty for that save */ }
        _ => {}
    });
    let _ = host.post_ready(env!("CARGO_PKG_VERSION"), false);
    // Saves: host.post_save(name, &bytes, None, save_as); unsaved state: host.post_dirty(dirty).
}
```

The page command API is [`craft-web-api`](../web-api/README.md) (storytold/craft-libs#413): `window.<app>.request` and the command functions. It sits beside these messages. Opening, saving, the document name and the dirty flag stay here.
