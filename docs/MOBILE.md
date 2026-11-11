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
- `appkit-ui/mobile` (TypeScript): `ipcFetch(bridge)` makes a `fetch` out of the bridge to the shell, so the client of
  the app (`HubClient` and what extends it) works unchanged; `Way`, `WAY_ABILITIES` and `nextStep` for what the page
  shows about the connection (`MUI-05`, `MOB-09`).
- `appkit-ui/queue` (TypeScript): the queue of writes made offline, each with `if_match`; a conflict keeps the hub's
  current version beside the item and is never sent again; a write whose answer was lost is never sent again by itself
  (`MOB-07`).

## Pairing

The PC shows a pairing link, as a QR code or as text:

```
https://<pc>.<tailnet>.ts.net/#appkit-pair=1&app=<namespace>&code=<32 hex digits>
http://127.0.0.1:<port>/#appkit-pair=1&app=<namespace>&code=<32 hex digits>
```

The code is good once, for 60 s, and stands in the fragment (no server sees it). The phone checks that the hub there is
of its app (`hello`), sends its public key with the code, and keeps the device id and the id of the hub. The key is the
platform's (Android Keystore) and never leaves the phone. Every start of the app signs a challenge of the hub; a hub
that names another id gets no signature.

## Losing a phone

1. On the PC: revoke the device (the list of devices of the app's hub); its sessions end at once.
2. In the Tailscale admin console: remove the phone from the tailnet.
3. What was on the phone: the kept copy of data classes released for phones only, no secret that opens anything
   without the phone's lock.
