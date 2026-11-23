---
title: "Example task with two code references"
status: open
---

Why `find.node` misses a namespace.

## Kontext

```code-ref
{
  "schema": "code-ref/1",
  "repo": "docmap.nvim",
  "sha": "0123456789abcdef0123456789abcdef01234567",
  "file": "lua/docmap/core/find.lua",
  "lines": [22, 41],
  "symbol": "docmap.core.find.node",
  "docmap": { "node": "docmap.core.find", "fn": "node" }
}
```

A block of another language is not a reference, whatever it holds:

```json
{ "schema": "code-ref/1", "repo": "x", "file": "/etc/passwd", "lines": [1, 1] }
```

```code-ref
{ "schema": "code-ref/1", "repo": "appkit", "file": "tools/check-wkd-intent.py", "lines": [90, 90] }
```
