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

A route an app names with `GuardRules::with_upload_path` takes a file as it is: no limit in row 2, no `application/json` in row 8, and no time limit for the request. Everything else is asked as for any route, a session and the origin first. Such a route has to stream the body to disk and end a transfer that stalls.

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

An app may have one second page that a browser can install (a PWA for a phone), below a prefix of its own:
`GuardRules::with_installable_page("/m", "tasks-mobile-worker")` (today: tasks-desktop, for tasks-mobile). `/m/` and
every path below it are shells: no credential, `GET` only, served by the app's `Pages` by their full path, and a
path the app does not serve is 404 (never the placeholder page). Their policy is the page's plus
`manifest-src 'self'` and `worker-src 'self'`, and `trusted-types` allows one more policy, the named one, next to
Svelte's: registering a service worker is a script-URL sink under `require-trusted-types-for 'script'`, and the page
uses that policy to turn exactly the worker's address into a script URL. Every other path keeps the policy above,
word for word. The prefix is one plain part (`/` + lower-case letters, digits, `-`, not `/api`, `/assets`, `/auth`)
and the name a plain one (not Svelte's, `default` or `none`), checked when the hub is made; a mistake ends the
program. `the_installable_page_has_its_own_policy_and_nothing_more` holds an app to this.

Every page says `microphone=()` in its `Permissions-Policy`. An app may let its installable page, and only that page,
record from the microphone (a voice note): `GuardRules::with_installable_microphone()` (today: tasks-desktop, for
tasks-mobile). Below the prefix the permissions then say `microphone=(self)`, and the policy adds
`media-src 'self' blob:`, so the page can play back the recording it made before it sends it; nothing else differs.
The browser still asks the person, and the desktop page and every other answer keep the microphone off and play
nothing (`the_installable_page_may_record_and_nothing_else`).

The installable page is opened by other apps too: a share target, a link in a mail, a launcher. The browser then sends
`Sec-Fetch-Site: cross-site`, which step 3 refuses everywhere else. Below the installable prefix, and only for the page
itself (`GET`, `Sec-Fetch-Mode: navigate`, `Sec-Fetch-Dest: document`), that is let through: the page is static and
needs no credential, and what it loads and asks afterwards comes from this origin and passes step 3 as usual. A fetch,
a script or a route of data from elsewhere is still refused, and so is the desktop page
(`another_app_may_open_the_installable_page_and_nothing_else`).

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
| device key | ECDSA P-256, made by the page with WebCrypto, non-extractable; the hub keeps the public key in `devices.json` | `POST /auth/device/session`: a session for a signed challenge (only an app that calls `with_devices`) |
| control secret | 256 bits, in `control.secret` | `POST /api/control/pair`: mint a pairing code; `/api/control/devices` and `/api/control/devices/revoke`: list and revoke devices. Nothing else. |

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

### A device that keeps its pairing

A session lives in the memory of the hub and of one page, so a phone that closes the app, or a hub that restarts, would
need a new pairing code each time. An app that calls `GuardRules::with_devices(<settings>/devices.json)` lets a device
pair once instead (`appkit-hub/src/devices.rs`, `appkit-ui/device`):

1. Pairing: the page makes an ECDSA P-256 key pair that WebCrypto keeps non-extractable (a script of the page can sign
   with it, nobody can read or copy it) and sends `POST /auth/device/pair` with the one-time code, a name and the public
   key. The name and the key are checked before the code is spent. The hub writes the device (a random id, the name,
   the public key, when it was paired and last used) to `devices.json` (to a file beside it first, then in place) and
   answers with the id and a first session. The page keeps the id and the key pair in IndexedDB of its origin.
2. Every start: `POST /auth/device/challenge` with the id gives 32 random bytes, good once for 60 s and for that
   device; every id gets one, so the answer does not tell which devices exist. The page signs
   `appkit-device-login/1`, the origin, the id and the challenge, and `POST /auth/device/session` checks the signature
   against the kept key. The origin is the request's, which step 3 has checked: a signature made for a page elsewhere
   opens nothing here. An unknown device, a wrong signature and a spent or old challenge get the same answer.
3. The PC lists and revokes devices with the control secret (`/api/control/devices`, `/api/control/devices/revoke`); a
   revoked device is gone from the file and its open sessions end at once. A session, the phone's own included, cannot
   list or revoke.

All three device routes are classified like `/auth/exchange`: no session, an Origin required, the same limit of
attempts. `devices.json` holds public keys, names and times, no secret; a file that cannot be read stops the start, so
the hub never half forgets whom it trusts. At most 16 devices. The sessions a device opens are tagged with its id, so
that an app can later give each device its own rights.

The file also holds a random id of the hub, made once (a list of an older hub gets one when it is opened, written at
once). The pairing and every challenge name it, and a phone signs nothing for a hub that names another
(`appkit-mobile`, `MSEC-04`): a new hub on the same name or port gets no signature, the phone pairs again.

### A phone through the tailnet or a tunnel

The hub binds loopback only, also for phones. A phone reaches it one of two ways, both ending on the PC
([docs/MOBILE.md](docs/MOBILE.md)):

- **Tailscale Serve** forwards `https://<pc>.<tailnet>.ts.net` to the loopback port. An app that calls
  `GuardRules::with_tailnet_name("<pc>.<tailnet>.ts.net")` (it needs `with_devices`) takes that `Host`, exactly, and the
  origin `https://<pc>.<tailnet>.ts.net`. A request with that host is held to what a device does: the routes of the
  control secret and `/auth/exchange` (a session for a pairing code) are refused (`not_on_tailnet`), and a route below
  `/api/` needs a session of a paired device (`device_required`); a session of a code, made on the PC, does not count
  there. `tailscale funnel` (public) is never the way.
- **An SSH tunnel** from an SSH client on the phone to the same loopback port on the PC (`-L <port>:127.0.0.1:<port>`):
  the hub sees a loopback host, as from the PC itself; the SSH client checks the host key of the PC.

A device reaches only the routes the app released for it (`GuardRules::with_device_route`); every other route below
`/api/` answers it `403 not_for_devices` before it runs, whatever the method, the core's `/api/vaults*` among them.
Every route below `/api/` can read who asks (`Caller`, set by the guard) and gives a device only data of the classes
`public` and `internal` (`appkit-core/src/classes.rs`): no class, or one not known, is `unclassified` and stays on the
PC like `private`.

Each device has a level (`read`, `create`, `change`, `trigger`; a new one gets `create`, or the app's default, never
`trigger`). Each released route names the level it needs, and the guard compares it on every request with the level
the device has now, so a level the PC lowers (`/api/control/devices/level`, control secret) holds for an open session
at once (`403 level_too_low`). A route of the level `trigger` also needs, per request, a signature of the device's
second key (paired as `confirm_key`, kept by the phone behind its biometrics or lock) over
`appkit-device-confirm/1`, the origin, the device, a fresh challenge, the method with the path and query, and the
SHA-256 of the body; the guard reads the body (within the body limit), checks it, and hands the same bytes to the route.
Missing, wrong, spent, or from a device without that key: `403 confirm_required`. A session of the PC is no device and
is held to none of this.

Web Push (`GuardRules::with_push`, `appkit-hub/src/push.rs`): the VAPID key is made once in the settings folder
(`push-key.json`); a file there that is not a key stops the start rather than make a new one. Only a device subscribes
(`/api/push/*`, level `read`), and only with an endpoint the hub may post to: `https`, a public host name, port 443, no
address, no `localhost` and no name of the local network or the tailnet, at most 1 KiB, so a device cannot point the
hub at the PC or the local network. A notice holds the kind of event and at most the id of an entry of a class released
for devices, encrypted for the device (RFC 8291); the VAPID token names the push service's origin and no person. A
revoked device's subscription is forgotten; one the push service calls gone (404, 410) too.

The phone app's shell (`appkit-mobile`) only builds those two kinds of address (an HTTPS tailnet name, or the phone's
own `127.0.0.1`), follows no redirect, uses no proxy of the system, reads every answer under a time and a byte limit,
keeps the session in memory and sends only routes of its own app.

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
  printable ASCII of the length of a token is not taken. Without an answer in 400 ms the new tab offers
  to pair the way a first tab does. A later answer to the same question still counts, with the same
  checks (on a busy machine an open tab can take seconds): the tab then takes the session, unless it
  has paired by then.
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
  A `429` or `503` of crates.io is waited out (its `Retry-After`, at most 120 s, else 10 to 60 s, about four
  minutes in all) and asked again; only then exit 2 (not checked). Publication dates never change and are kept in
  `--cache`, else `$LOCK_AGE_CACHE`, else `~/.cache/lock-age/crates-io.json`, so the jobs on one runner ask once.
* `unsafe` is denied in the whole workspace. One module allows it: `appkit-hub/src/handles.rs`
  (Windows only, `windows-sys`, the version tokio already brings) clears the inherit flag of the
  standard handles before a hub is started, so the hub does not hold a pipe of its caller. The
  stable standard library cannot start a program without handing on every inheritable handle.
  This replaced the script host (`wscript.exe`) for `start`, which an endpoint protection stops
  when an unsigned program starts it.

## Reporting

This is a personal project. Open an issue without details, or mail the maintainer.
