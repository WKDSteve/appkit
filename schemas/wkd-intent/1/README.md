# `wkd-intent/1`

An **intent** is what one app hands to another: "create a task from this finding", "show this rule". It has fixed
actions, never free operations, and it writes nothing by itself: the target shows its own form, and the target's engine
writes after a person confirmed. `testing.nvim` and the apps around it produce intents; no one of them writes into
another's vault.

Files here: [`wkd-intent.schema.json`](wkd-intent.schema.json), one golden example per action in
[`examples/`](examples/). `tools/check-wkd-intent.py` is the reference for the rules below, with
`tools/test_check_wkd_intent.py`; apps port it to their language and test against the same examples.

## The document

```json
{
  "schema": "wkd-intent/1",
  "action": "tasks.new",
  "mode": "form",
  "source": { "app": "testing", "finding": "0123456789abcdef" },
  "finding": { "schema": "testing-finding/1", "...": "..." },
  "target": null
}
```

| Action | Who takes it | `finding` | `target` | `mode` |
|---|---|---|---|---|
| `tasks.new` | tasks | required | `null` | `form`, `ai-draft`, `agent` |
| `tasks.show` | tasks | `null` | `{ "id": "<area>/<slug>" }` | `form` |
| `rules.new` | rules | required | `null` | `form`, `ai-draft`, `agent` |
| `rules.show` | rules | `null` | `{ "id": "<RULE-ID>" }` | `form` |
| `ai.handoff` | ai | required | `null` | `form`, `ai-draft`, `agent` |

`form` opens the target's form filled in. `ai-draft` has the target draft the text first, then its form. `agent` hands the
work to an agent of the target; a person still confirms what is written.

`finding` is `testing-finding/1` (`schemas/testing-finding/1` in testing.nvim). The bigger AI bundle `testing-handoff/1` is
too big for this document and travels as a file.

## Versions

- `schema` is `wkd-intent/<n>`. A higher `n` is refused, with the reason; a lower one does not exist yet.
- Fields nobody knows are ignored. A new field that changes what an action means is a new version.
- A new action is not a new version: a target that does not know it refuses it, as any action it did not offer.

## Handshake

Every target names what it takes where it already introduces itself (`call hello` of the engine, the `/health`-side
document of a hub):

```json
{ "intents": { "schema": "wkd-intent/1", "actions": ["tasks.new", "tasks.show"], "modes": ["form", "ai-draft"] } }
```

A sender offers only what the target named. A target that names nothing takes nothing. The sender's list is a courtesy;
the check below happens on the target's side either way.

## What the target checks

An intent is **untrusted input**, wherever it came from. The target refuses, with one reason per problem and nothing
written:

1. More than 64 KiB (UTF-8 bytes), measured **before** parsing.
2. Anything that is not one JSON object of this schema: a higher version, a missing field, a `finding` where an action has
   none, a mode the action does not allow.
3. An action this target did not offer in its handshake.
4. A `class` other than `public` or `internal`.
5. An absolute path or a way out of the repository in `finding.where.file`, `finding.repro` or `target.id`: `/x`, `\x`,
   `C:\x`, `C:/x`, `\\host\x`, `~/x`, `$HOME/x`, a `..` segment. Paths in an intent are repository-relative.
6. Lengths: `finding.title` 120, `finding.excerpt` 4096, `finding.repro` 1000, `finding.where.file` and `target.id` 300,
   `source.app` 64. Control characters other than tab and line break are refused in every field.
7. A `finding.run.url` that does not start with `https://github.com/`.

The form of the target then applies the limits of its own door on what the person submits (tasks: title 300, summary
1000). `target.id` is looked up, never opened as a path.

## How it travels

| From | To | Transport |
|---|---|---|
| Neovim | same editor | a call to the target's engine (`:Tasks new` filled in, the rules form, a context source of ai.nvim); without the plugin the headless CLI |
| Command line | any engine | `testing act`, through the target's door; without `--yes` a dry run |
| Desktop app | a hub on loopback | `POST /intent` with the hub's token, the hub passes it to its panel; inside the hub `navigate(link)` |
| Desktop app | no hub running | clipboard, as from a browser |
| Browser page | any app | **clipboard** on a click, or **a file** the person saves; the target has an "insert intent" field |

A target reads the clipboard only through a paste field the person pastes into: the kit forbids `clipboard-read`, and that
stays. A URL form (`wkd://...`) is deliberately not part of this version. It would need a protocol handler, a registry
change on the machine, and it could carry ids only.
