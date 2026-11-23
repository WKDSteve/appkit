# `code-ref/1`

A **code reference** points at code: a repository, a file, a range of lines, and when known the commit, an excerpt, the
DocMap node and the telemetry key. It is what an editor makes when a person marks code, and what a task, DocMap and an
agent then share, so each reads the same fields instead of a ref string of its own. It is data only: it opens and
writes nothing, and the reader decides what to show.

Files here: [`code-ref.schema.json`](code-ref.schema.json) and the goldens in [`examples/`](examples/):
[`marking.json`](examples/marking.json) (a selection in an editor, the working tree was dirty),
[`function.json`](examples/function.json) (a function with its DocMap node and telemetry key),
[`minimal.json`](examples/minimal.json) (file and lines only) and [`task-body.md`](examples/task-body.md) (how it stands
in a task). `tools/check-code-ref.py` is the reference for the rules below, with `tools/test_check_code_ref.py`; apps
port it to their language and test against the same goldens.

## The document

```json
{
  "schema": "code-ref/1",
  "repo": "docmap.nvim",
  "sha": "0123456789abcdef0123456789abcdef01234567",
  "dirty": false,
  "file": "lua/docmap/core/find.lua",
  "lines": [22, 41],
  "lang": "lua",
  "symbol": "docmap.core.find.node",
  "excerpt": "function M.node(ir, name, lua_root)\n  ...",
  "truncated": true,
  "docmap": { "node": "docmap.core.find", "fn": "node" },
  "telemetry": { "namespace": "docmap.nvim", "key": "docmap.core.find.node" },
  "class": "public"
}
```

| Field | | Meaning |
|---|---|---|
| `schema` | required | `code-ref/1` |
| `repo` | required | the name of the repository, never a path: `[A-Za-z0-9][A-Za-z0-9._-]*`, up to 100. The reader finds the checkout by name (DocMap resolves a project by id, name or root; an ambiguous name is refused there) |
| `file` | required | repository-relative, forward slashes, up to 300 (see "Paths") |
| `lines` | required | `[first, last]`, 1-based, both inclusive, `1 <= first <= last <= 10000000`. A single line is `[n, n]`. Always the **whole** range, even when the excerpt is cut |
| `sha` | no | the commit the lines refer to: 40 (or 64) lowercase hex characters, or `null` for "not known" |
| `dirty` | no | `true`: the lines and the excerpt are of the working tree, which differed from `sha`. Absent means `false` |
| `lang` | no | a plain name (`lua`, `rust`, `c++`), up to 32, for the fence of an excerpt |
| `symbol` | no | the name the code goes by in its language (a Lua module path with the function, a Rust path), free text up to 200 |
| `excerpt` | no | the text of the lines, as marked, up to 4096 **UTF-8 bytes** (see "Size") |
| `truncated` | no | `true`: `excerpt` is shorter than `lines`. Absent means `false` |
| `docmap` | no | `{ "node": <module or node id>, "fn": <function name> }`; `node` is what DocMap's lookup takes (a declared module, a node id), `fn` is optional. The IR key of DocMap is `node + "#" + fn` and is **derived**, not stored |
| `telemetry` | no | `{ "namespace": ..., "key": ... }`, both required. The key **as the telemetry recorded it**: it is not derivable from `docmap` (a wrapped module is labelled by the wrap call, `servers.attach`), so it is stored whole or not at all |
| `class` | no | `public` or `internal`. Absent means `internal`; a reader never takes absence for `public` |

Why these fields differ from the first sketch (checked against the readers):

- `tasks.nvim` writes `file:first` today (`range_source`, since `8aa6276` relative to the git root and naming the whole
  range, `file:first-last`): that string is `file + ":" + first + "-" + last` of this document, so a reader that wants
  the old form derives it, and a `:` in `file` is refused so it stays unambiguous. `docmap-desktop` (`open-at.js`) takes
  the same `file:first-last` and refuses a path with a colon, an absolute path or a `..`.
- `docmap.nvim` resolves a name to a node (`core/find.lua`) and joins telemetry by the IR key `node#fn`
  (`core/telemetry_join.lua`), and only for keys the telemetry itself resolved to a module: so `docmap` carries node and
  function, and `telemetry` carries the recorded key, never a guess.
- `class` is optional because a plain marking in the editor knows nothing about it; the safe reading is `internal`.

## Paths

`file` (and `docmap.node`) never leave the repository. The reader refuses: `/x`, `\x`, `C:\x`, `C:/x`, `\\host\x`, `~/x`,
`$HOME/x`, `${HOME}/x`, `%VAR%\x`, a `..` segment, any backslash, any colon (a drive, a stream, a `file:line` pasted in),
a NUL, an empty or `.` segment (`a//b`, `./a`, a trailing `/`). So there is **one** spelling of a path, and two readers
agree on it without normalizing. A path is looked up in the checkout of `repo`; it is never opened without that
check. Nothing in this document holds the path of a person's machine: the repository is named, not located.

## Size

- The whole document is at most **64 KiB** (UTF-8 bytes), measured **before** parsing.
- `excerpt` is at most **4096 UTF-8 bytes**, counted in bytes, not characters (the JSON Schema `maxLength` counts
  characters, so it can only state the upper bound; the byte rule is the one that holds). The excerpt is cut, never the
  mark: cut on a whole character and set `truncated`. `lines` says what the person marked, and an agent reads the rest
  of the range from the repository.

## Versions

- `schema` is `code-ref/<n>`. A higher `n` is refused, with the reason; a lower one does not exist yet.
- Fields nobody knows are ignored. A new field that changes what an existing one means is a new version.

## What the reader checks

A code-ref is **untrusted input**, wherever it came from (a task file, a clipboard, a job). The reader refuses, with one
reason per problem and nothing used:

1. More than 64 KiB, measured before parsing; anything that is not one JSON object.
2. A `schema` other than `code-ref/1` (a higher version included).
3. A missing `repo`, `file` or `lines`; a `repo` that is not a plain name; a `lines` that is not two integers in
   order (a boolean is no integer).
4. A `file` or `docmap.node` that leaves the repository or is not in the one spelling above, or is longer than 300.
5. A `sha` that is not 40 or 64 lowercase hex characters (or `null`); a `dirty` or `truncated` that is not a boolean.
6. A `class` other than `public` or `internal`.
7. Lengths: `symbol` 200, `docmap.fn` 200, `telemetry.namespace` 100, `telemetry.key` 300, `excerpt` 4096 bytes; a
   `docmap` without `node`, a `telemetry` without `namespace` and `key`.
8. A control character other than tab, line break and carriage return in any string, keys included.

The excerpt is code: it may hold `/etc/passwd`, `..` or `~`, and is never checked as a path; it is also never executed
or interpreted, and a reader shows it as text (escaped, in a code fence of `lang`).

## Where it stands in a task

A task holds a code-ref in a **fenced block with the language `code-ref`**, in the body (not the frontmatter), by
convention under `## Kontext`:

````markdown
```code-ref
{ "schema": "code-ref/1", "repo": "docmap.nvim", "file": "lua/docmap/core/find.lua", "lines": [22, 41] }
```
````

- One object per block; several blocks are several references, in the order they stand.
- The fence is three or more backticks and closes with at least as many; the info string is exactly `code-ref`.
  A block of any other language is not read, and a `code-ref` block **inside** another fence is text, not a reference.
  The frontmatter is not body. An unclosed block is dropped.
- The size limit holds per block (the text between the fences). The task file itself is bigger, and has its own limits.
- The block is the only place: tasks.nvim writes it, tasks-desktop, DocMap and ai-desktop read it. ai-desktop hands the
  task as `markdown` of a job, so the reference reaches the agent with no new field in `docs/JOBS.md`.
  `blocks` in `tools/check-code-ref.py` is the reference for finding the blocks.
- A reader that fails a block shows the block as text and uses nothing from it; the other blocks are unaffected.

Privacy: the document names the repository, not a path. In a private area, the writer (tasks.nvim) puts an excerpt in
only after asking, and the reader treats a task with a reference as it treats any private task (ai-desktop:
`private: true`, a local model or a question first). `class` does not replace that: it says what the **code** may be
told to, the area says what the **task** may.

## Porting the check

Lua (tasks.nvim, docmap.nvim), Rust (tasks-desktop, docmap-desktop, ai-desktop) and TypeScript / JavaScript (web pages)
each port `tools/check-code-ref.py` as one function from the bytes to a list of reasons, and a test that reads the
goldens in `examples/` from this repository (a copy pinned by `appkit` tag, as for `wkd-intent/1`): every golden has
no problems, and the refusals of `tools/test_check_code_ref.py` hold. Traps the reference tests name: the size is taken
before parsing, a boolean is no integer, the excerpt is counted in bytes, a deeply nested document does not overflow
the stack, a `code-ref` inside a longer fence is not a block.

## Later

`wkd-intent/1` can carry a code-ref in place of a `finding`, for an action such as "task from a review finding"; its
README says how. That is not part of this version.
