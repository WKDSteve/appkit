# The phone apps of the families

Each app family (testing, tasks, ...) has a phone app of its own. What they all need between the phone and the hub on
the PC is here, once: the way to the data, pairing, the rights of a device, the data classes and push. The rules are
the ruleset `mobile` of the maintainer's rules (`MOB-*`, `MSEC-*`, `AND-*`); this page says how the kit meets them.

## The three ways to the data

| Way | What it is | Read | Create, change | Trigger | Push | Offline |
|---|---|---|---|---|---|---|
| Tailscale Serve | `https://<pc>.<tailnet>.ts.net`; `tailscale serve` forwards it to the loopback port of the hub | yes | yes | with a confirmation on the phone | yes | the kept copy |
| SSH tunnel | an SSH client on the phone forwards `127.0.0.1:<port>` to the same port on the PC | yes | yes | with a confirmation on the phone | no (the tunnel is down when the app is) | the kept copy |
| Vault on the phone | a folder cloned with Git on the phone (an export of the app), read by the app | yes | no | no | no | always |

- The hub stays on loopback in every way (`SRV-01`): no LAN address, no port forward on the router, no Funnel.
- Tailscale Serve gives a real certificate for the tailnet name, so the phone speaks HTTPS. The name is in public
  Certificate Transparency logs: give the PC a neutral name.
- Two hubs on one PC: one on Serve's 443 (`<pc>.<tailnet>.ts.net`), the other on another HTTPS port
  (`tailscale serve --bg --https=8443 http://127.0.0.1:<its port>`), named with it: `<pc>.<tailnet>.ts.net:8443` as
  its tailnet name, in its pairing link and on the phone. Never `:443` (a browser leaves the default port out).
- The SSH tunnel is plain HTTP to the phone's own loopback, carried by SSH; the SSH client checks the host key. Use the
  hub's own port on both ends: the guard takes `127.0.0.1:<its port>` as the host.
- A cloned vault is read only. The hub is not involved, so the data classes are the clone's: a vault (or an export)
  with private parts is never cloned onto a phone, and the app refuses one that says it holds any.

## Behind one interface

A phone app is a Tauri app. Its page never speaks to the hub itself:

```
page (Svelte)  --ipcFetch-->  shell (Rust, appkit-mobile)  --HTTPS or tunnel-->  hub (appkit-hub, loopback)
```

- `appkit-mobile` (Rust): `way` (the ways and the pairing link), `link` (pairing, the device login, every request
  with a time limit and a byte limit; the session in memory; only routes of the app), `state` (the kept pairing,
  versioned, written atomically, removed wholly on unpairing).
- `tauri-plugin-appkit` (`mobile-plugin/`, Rust and Kotlin, a Cargo project of its own): the Android side. The device
  keys in the Android Keystore (a login key, and a trigger key that signs only after the biometrics or the screen lock,
  bound by a `CryptoObject`, `AND-05`), the QR code of a pairing (ZXing: no network, no analytics, unlike ML Kit), the
  folder of a cloned vault (Storage Access Framework, read only, bounded), and UnifiedPush (the endpoint of a distributor
  app, a notice with the words the app gave, never the content of a message). The page reaches none of it: the shell
  calls it from Rust.
- `appkit-ui/mobile` (TypeScript): `ipcFetch(bridge)` makes a `fetch` out of the bridge to the shell, so the client of
  the app (`HubClient` and what extends it) works unchanged; `Way`, `WAY_ABILITIES` and `nextStep` for what the page
  shows about the connection (`MUI-05`, `MOB-09`).
- `appkit-ui/queue` (TypeScript): the queue of writes made offline, each with `if_match`; a conflict keeps the hub's
  current version beside the item and is never sent again; a write whose answer was lost is never sent again by itself
  (`MOB-07`).

## Data classes, fail closed

The hub cuts, never the phone (`MOB-02`):

- A route reaches a device only when the app released it: `GuardRules::with_device_route(path)`. Every other route
  below `/api/` answers a device `403 not_for_devices` before it runs, the core's choice of vault among them. An app
  that releases nothing gives a device nothing.
- A released route reads who asks (`Caller::Pc` or `Caller::Device(id)`, set by the guard) and gives a device only data
  of the classes `public` and `internal` (`DataClass`, `Caller::may_see`). An area without a class, or with one the
  code does not know, is `unclassified` and stays on the PC like `private`. What is made of several things has the
  highest class of them (`classes::highest`).
- The phone keeps only what the hub sent it (`appkit-mobile::cache`), so its copy holds no private data either; a
  device the hub does not take any more forgets its copy.
- A vault cloned on the phone has no hub in front of it. The phone reads only an export the engine wrote in its export
  form (the data classes asked, private and unclassified left out); a raw vault, or an export that does not say its
  classes were asked, is refused. Never clone a vault with private parts onto a phone.

## Rights of a device

Each device has a level, which the hub checks on every request (`MOB-03`):

| Level | May |
|---|---|
| `read` | read what is released for devices |
| `create` | also create (a new entry, an answer); the default of a new device |
| `change` | also change (a status, a field) |
| `trigger` | also start something on the PC, each time confirmed on the phone |

- The app names the level of each route it releases: `GuardRules::with_device_route(path, Level::Read)`. A device below
  it gets `403 level_too_low`. An app may raise the default of a new device to `change`
  (`with_device_default_level`); `trigger` is never a default.
- The PC sets the level of a device (`/api/control/devices/level` with the control secret, the command line below);
  an open session follows at once.
- A trigger carries the confirmation of that very request: a second key of the phone, kept behind its biometrics or
  its lock (Android Keystore with user authentication, `AND-05`), signs the method, the path with its query and the
  SHA-256 of the body with a fresh challenge of the hub (`x-device-confirm: <challenge>.<signature>`). Without it, with
  a wrong or spent one, or from a phone paired without that key: `403 confirm_required`. The phone shows the prompt
  before it signs; cancelling sends nothing (`appkit-mobile`: `Link::trigger`).
- A page installed on the phone (a PWA, `appkit-ui/device`) has such a key too, made at pairing and non-extractable,
  but not behind the biometrics: it signs only after the person tapped to confirm on the page (`confirmHeader`), which
  keeps a stolen session token from triggering, not a script of the page itself (SECURITY.md). A page paired before
  it had the key pairs again to trigger (`canConfirm` tells it).
- Never from a phone, at any level, because no app releases a route for it: trusting a commit of somebody else
  (`testing ci trust`), pushing, deleting for good, any command of the person's choice. Each app's tests prove that its
  released routes hold none of these.

## Push

The hub sends Web Push itself (`appkit-hub::push`, RFC 8030, 8291, 8292): no Firebase project, no account at any push
provider.

- The hub makes a VAPID key once and keeps it in its settings folder (`push-key.json`, never in a repository). A device
  subscribes with its public half (`GET /api/push/key`) for kinds of events (`POST /api/push/subscribe` with the
  endpoint, its `p256dh` key, its `auth` secret and the kinds; `POST /api/push/unsubscribe`). Only a device does; a
  session of the PC gets `403 device_required`. Revoking a device forgets its subscription.
- The hub posts only to an `https` endpoint on a public host name: no address, no `localhost`, no name of the local
  network or the tailnet, no other port than 443 (it would otherwise post to whatever a device names).
- A notice carries the kind of event and at most the id of an entry whose class is released for devices
  (`Push::notify(kind, Some((id, class)), urgent)`): `{"kind":"ci_red","id":"lib.nvim"}` or `{"kind":"ci_red","id":null}`.
  The phone fetches the rest over its paired way (`PWA-06`). The text is encrypted for the device (aes128gcm), but the
  push service is somebody else's server all the same.
- Where the endpoint comes from: a browser's `PushManager` in an installed page (Android Chrome; iOS only for a page on
  the home screen), or, for a Tauri app on Android (whose WebView has no Web Push), a UnifiedPush distributor app on the
  phone (for example Sunup, which uses Mozilla's push service, or ntfy). Both speak this format. Without either, there is
  no push, and the app says so once (`MOB-09`).
- An app turns on the sending with the feature `push-net` of `appkit-hub` and `GuardRules::with_push`.

## On the PC: the command line

An app's hub offers (with `appkit-hub::phone`):

```
<program> phone pair [--tailnet <pc>.<tailnet>.ts.net]   a one-time pairing link and its QR code (60 s)
<program> phone list                                     the paired devices: id, name, level, confirmation key
<program> phone level <id> read|create|change|trigger    set the level of a device
<program> phone revoke <id>                              forget a device; its sessions end at once
```

Without `--tailnet` the link names the loopback port, for a phone behind an SSH tunnel.

## Pairing

The PC shows a pairing link, as a QR code or as text:

```
https://<pc>.<tailnet>.ts.net/#appkit-pair=1&app=<namespace>&code=<32 hex digits>
http://127.0.0.1:<port>/#appkit-pair=1&app=<namespace>&code=<32 hex digits>
```

The code is good once, for 60 s, and stands in the fragment (no server sees it). The phone checks that the hub there is
of its app (`hello`), sends its public key (and the public key that confirms triggers) with the code, and keeps the
device id and the id of the hub. The keys are the platform's (Android Keystore) and never leave the phone. Every start
of the app signs a challenge of the hub; a hub that names another id gets no signature.

## Losing a phone

1. On the PC: revoke the device (the list of devices of the app's hub); its sessions end at once.
2. In the Tailscale admin console: remove the phone from the tailnet.
3. What was on the phone: the kept copy of data classes released for phones only, no secret that opens anything
   without the phone's lock.
