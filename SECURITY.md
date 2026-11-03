# Security of the hub

`appkit-hub` is the local web server of an app: it runs on your own machine, and anything on the
machine, and any web page you open in a browser, can try to talk to it. This page says what the
hub does about that, and what it does not. It holds for every app built on it; what an app adds
(its routes, its data) is described by the app.

`<ns>` below is the namespace of the app's routes and `<app>` its name (both from its
`AppIdentity`). The tests named here are in `crates/appkit-testkit/src/security.rs`. They are
written once and run for every app (`security_matrix!`): this repository runs them for `Demo`, a
minimal app, and every app runs them for its own hub.

## What a request has to pass

Every request goes through one guard, in this order. A refusal stops there. An app names paths
(`GuardRules`) and chooses the numbers (`Limits`, `pairing_ttl`); it cannot add, remove or reorder a
step. The numbers on this page are the defaults (`Limits::default()`, a pairing code of 60 s); an app
that changes them says so in its own documentation, and the matrix of `appkit-testkit` runs with the
defaults.

| # | Check | Refusal | Why |
|---|---|---|---|
| 1 | `Host` is exactly `127.0.0.1:PORT`, `localhost:PORT` or `[::1]:PORT` (one header, ASCII) | 403 / 400 | DNS rebinding |
| 2 | `Content-Length` is a number and not over the limit (by default 1 MiB, and 1 KiB for the two pairing routes, which take a few bytes) | 413 / 400 | memory |
| 3 | `Origin`, if sent, is exactly one of the three origins; `Sec-Fetch-Site`, if sent, is `same-origin` or `none` | 403 | cross-site requests |
| 4 | the method is one the route knows (no `OPTIONS`, no CORS) | 405 | no cross-origin access at all |
| 5 | a mutation without `Origin` is a program client: it must carry a credential and must not send `Sec-Fetch-Site` | 403 | a browser always sends `Origin` with a POST |
| 6 | a limit of mutations and of pairing attempts in a window | 429 | guessing |
| 7 | the credential the route needs | 401 | see below |
| 8 | a mutation is `application/json` | 415 | no form posts |

Around that: the hub binds loopback only and refuses any other address; the number of connections
is limited; a client has a few seconds to send its request head; the request line and headers
together are at most 8 KiB.

Every answer carries `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
`Cache-Control: no-store` and `Cross-Origin-Resource-Policy: same-origin`; the page carries a
restrictive `Content-Security-Policy` whose `connect-src` is `'self'`. An app can widen that by exactly one
host with `GuardRules::with_connect_src("https://host")` (today: hub-desktop, `https://api.github.com`, for the
update badge: a read of public release data without token or cookie). The value is checked when the hub is
made and must be `https://` plus a lower-case host name with a dot and an optional port: no wildcard, no
address, no path, no other scheme (a mistake ends the program). The matrix checks that every page's
`connect-src` is `'self'` and at most one such host, and `the_page_may_connect_to` that the named host is the
one in the header.

Routes are classified by their raw path. Everything under `/api/` needs a session unless it is on
the short list below, so a route an app adds is closed until it is opened on purpose; a path with
percent escapes or a trailing slash is not the list's path and needs a session too. A path outside
`/api/` that is not the page is answered "not found" by the guard itself.

## Credentials

| Credential | What it is | What it allows |
|---|---|---|
| none | | the page, `GET /api/<ns>/hello` (name, version, instance id, whether the engine is ready, a short problem code; no data, no path) |
| pairing code | 128 bits, one use, 60 s by default | `POST /auth/exchange` for a session token |
| session token | 256 bits, in the memory of the hub only, a limited number open | the routes of the app |
| control secret | 256 bits, in `control.secret` | `POST /api/control/pair`: mint a pairing code. Nothing else. |

Comparisons are constant time and look at every stored entry. Secrets come from the operating
system's random source and never print themselves. The answer for an unknown, a spent and an
expired code is the same.

### How a browser gets a session

1. A program of the app (`<program> open`) reads `hub.json` and `control.secret` from the config
   folder, checks that the program on the port is this hub (same instance id) before it sends the
   secret anywhere, and asks for a pairing code.
2. It prints `http://127.0.0.1:PORT/#pair=<code>`. The code is in the URL *fragment*: a browser
   does not send it to the server or in a `Referer`.
3. The page takes the code out of the fragment, calls `/auth/exchange` and keeps the token in
   memory.

A pairing code is never handed out by an open endpoint. The only way to get one is to be a program
that can read `control.secret`.

### A new tab of the page

A place of the page (a view, an open item) can stand in the fragment of the address
(`appkit-ui/route`, `appkit-ui/nav`), so that it opens in a new tab, again after a reload, and from a
copied address. It is in the fragment for the same reason the pairing code is: a browser does not send
it to the hub or in a `Referer`. It does end up in the browser's history, with the name of the item it
names; an app puts into a route only what a new tab needs to find the place (an id, the name of a
view), never the content of an item. The pairing code is never part of a route: `takePairingCode`
with `keepRoute` takes the code out and leaves only the place.

The token is kept per tab. A tab the browser opened itself from a link (its context menu, a middle
click the page did not take) starts without one. Instead of a new pairing code it asks the open tabs
of the page on a `BroadcastChannel` (`appkit-ui/handoff`), and one that has a session answers with its
token; the new tab shares that session (no new session is opened, and the limit of the hub is not
touched by tabs). What limits this:

* A broadcast channel reaches only pages of the same origin (`http://127.0.0.1:PORT`) in the same
  browser profile. A script of that origin owns the session anyway (see below).
* A tab answers only while its event stream is open. The hub is then up and holds the port, so the
  page that asks was served by that hub and not by another program that took the port after the hub
  stopped (see below, another process of the same user).
* The question carries 128 random bits and an answer counts only for it; an answer whose token is not
  printable ASCII of the length of a token is not taken. Without an answer in 400 ms the new tab pairs
  the way a first tab does.
* A tab that shares a session shares its limits too: the live connections of a session are counted
  across all its tabs.

### The control secret file

`<config>/<app>/control.secret` is the one thing that lets a local program ask for access. It is
protected by the rights of your account:

* Unix: mode `0600`, set when the file is created and again afterwards.
* Windows: every inherited right is removed and the file is given to the current user alone
  (`icacls /inheritance:r`). The inherited list of a folder is not trusted. If the rights cannot be
  set, the hub does not start.

`hub.json` (app, port, process id, version, instance id) holds no secret. A program believes it only
when the app is its own and the program on the port answers with the same instance id, so a stale
file, or the file of another app, never makes it send the secret anywhere or stop a process.

## What an engine process gets

An engine (a Neovim script the app names) runs with an empty environment plus a short list: `PATH`,
`PATHEXT`, `REPOS_DIR`, `SystemRoot`, `windir`, `TEMP`, `TMP`, `TMPDIR`, `HOME`, `USERPROFILE`, and the
switches the hub sets from its own settings (the vault and the extra areas, under the names of the
app's dialect, and `LIB_NVIM_DIR`). The request goes in on stdin as printable ASCII; nothing a person
typed is put on a command line.

A persistent engine process answers questions as lines (`<id>\t<method>\t<params>`, answered by
`<id>\t<document>`) with the same environment, request and limits. A read that loses its host is
asked again by a process of its own; a write that was sent and not answered is an unknown outcome,
never asked again; a question that does not come back in time kills the host; a host is replaced when
the code of the engine changed; the answer of a question is checked against its id. An app may offer a
switch that turns the host off.

## What this does not protect against

* **Another process of the same user.** It can read `control.secret`, or bind the port while the hub
  is not running. The hub cannot tell that process from the app's own program. Mitigations: the
  secret is only sent to a process that reports the instance id of `hub.json`, tokens are per run,
  and the port is fixed so the origin is stable.
* **A local program that wants to be a nuisance.** The limits protect the hub, not its availability:
  a program on the machine can use up the budget of pairing attempts or of mutations for a while.
  It gains no access by that.
* **An administrator**, who can take ownership of the file.
* **The terminal and the browser history.** The pairing link ends up in the terminal that printed
  it and in the browser's history. That is why the code is single use and lives 60 s (by default).
* **A page script in the app's own origin.** A script in the page would own the session. The policy
  above is the answer; what the app bundles is its own responsibility.

## Dependencies

* `Cargo.lock` is committed and every build uses `--locked`.
* `deny.toml` allows a short list of licenses, crates.io only, no git sources, no wildcard versions,
  and no crate of an app (the kit must never depend on one); `cargo audit` and `cargo deny check` run
  in CI.
* A version that is younger than seven days is not taken (`tools/check-lock-age.py`, also run in CI).
* `unsafe` is denied in the whole workspace. One module allows it: `appkit-hub/src/handles.rs`
  (Windows only, `windows-sys`, the version tokio already brings) clears the inherit flag of the
  standard handles before a hub is started, so the hub does not hold a pipe of its caller. The
  stable standard library cannot start a program without handing on every inheritable handle.
  This replaced the script host (`wscript.exe`) for `start`, which an endpoint protection stops
  when an unsigned program starts it.

## Reporting

This is a personal project. Open an issue without details, or mail the maintainer.
