# appkit

The shared base of the desktop apps (tasks-desktop, rules-desktop, testing-desktop, hub-desktop, pult, ai-desktop,
docmap-desktop). Read [README.md](README.md) first: what each crate and `ui-kit` / `web-kit` is, and what goes where.
The kit knows no app; code moves here when a second app needs it, not ahead of time.

## Before you change something

- Security: [SECURITY.md](SECURITY.md). The guard of `appkit-hub` (`crates/appkit-hub/src/guard.rs`) has eight steps
  in a fixed order, and the page gets a strict CSP with Trusted Types. Do not loosen either to make a feature work.
- Processes start only through `appkit_core::child` and `appkit_hub::launch` ([docs/PROCESSES.md](docs/PROCESSES.md)).
- Colours in the CSS of `ui-kit` come from `ui-kit/src/tokens.css` only.
- Releases and how the apps take a tag: [docs/RELEASING.md](docs/RELEASING.md), `tools/bump-apps.py`. An app picks up
  the kit only by its pins (crates, `appkit-ui`, `appkit-web`, release-kit); this file and `.mwgrc` do not reach
  the apps, each app has its own.

## Checks

CI (`.github/workflows/ci.yml`) runs on pushes to `main`. Run what you touched before you push:

- Rust: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
  `cargo test --workspace --all-features --locked`
- Tools and hygiene: `python -m unittest discover tools`, `python tools/check-control-chars.py`,
  `python tools/check-process-starts.py`, `python tools/check-color-literals.py --root ui-kit/src --tokens ui-kit/src/tokens.css`
- `ui-kit`: `pnpm install --frozen-lockfile && pnpm check && pnpm test`
- `web-kit`: `pnpm test && pnpm check` (needs the install of `ui-kit`)

Commits: `type(scope): what changed` (scopes such as `hub`, `ui-kit`, `web-kit`, `release-kit`), a body that says why.

## Web platform

- Browser Support: Baseline `widely` available (`.mwgrc`, `browserslist` in `ui-kit` and `web-kit`). An app built on
  the kit may hold a stricter policy of its own (tasks-desktop: Chrome/Edge 86+, see its README); code in
  `ui-kit` / `web-kit` that an app with such a policy uses must work there too, or come with a fallback.
- Before you write browser code, ask the modern-web guidance for the feature (run it with `DISABLE_TELEMETRY=1`):
  `npx -y modern-web-guidance@latest search "<topic>"`, then `retrieve <id>`. Prefer the platform feature (dialog,
  popover, anchor positioning, `:has()`, container queries) over a script that imitates it, with a fallback when the
  feature is below the target.
- Feature detection, not browser sniffing. Polyfills where browsers still in use need them (rather one too many), run only after the check. Nothing that the CSP (`script-src 'self'`, Trusted Types) would refuse:
  no `innerHTML` with strings, no inline `<script>` or `<style>`, no `eval`.
- `legacy-scan` finds a part of the legacy patterns (0 findings on 2026-11-21; a new finding is a reason to look). The
  scanner is an exact devDependency of `ui-kit` (the pnpm supply-chain policy applies):
  `DISABLE_TELEMETRY=1 pnpm -C ui-kit exec modern-web-guidance legacy-scan src --json`, the same for `../web-kit/src`.
  CI runs it as an informative step in the `ui-kit` and `web-kit` jobs. Raise its version on purpose.
- `.claude/settings.json` sets `DISABLE_TELEMETRY=1` and allows only `npx -y modern-web-guidance@latest *`.
- Rules: the maintainer's ruleset for the web platform (private, so not named or cited here by id).
