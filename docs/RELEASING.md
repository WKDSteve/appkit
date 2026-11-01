# Releasing

A version of `appkit` is a tag `vX.Y.Z` on `main`. There is no build to publish: the apps depend on
the tag, and `Cargo.lock` of each app pins its commit. The tag covers the Rust crates and the browser
kits (`ui-kit` as `appkit-ui`, `web-kit` as `appkit-web`) alike: an app pins all of them to the same
tag (`pnpm-lock.yaml` holds the commit of each kit), and its CI checks that the pins name one commit.

1. **`main` is green.** The CI of the very commit that gets the tag passed on Linux and Windows,
   with `cargo deny` and `cargo audit`.
2. **The version says what changed.** While the version is `0.x`, a change of anything public
   (a type, a function, a field, a route of the core, the order or the meaning of a check of the
   guard) raises the minor number (`0.1.0` to `0.2.0`); a fix that keeps all of it raises the patch
   number. The crates keep `version = "0.0.0"` in their manifests: the tag is the version.
3. **Tag and push.** `git tag -a vX.Y.Z -m "..."` on the green commit, `git push origin vX.Y.Z`.
4. **A tag is never moved or deleted.** A mistake is fixed in the next version. Each app checks in its
   CI that the commit its `Cargo.lock` pins is the one its tag names now, so a moved tag is noticed.
5. **Every app takes a fix of the guard.** A change to the guard, the pairing, the sessions or the
   limits that closes a hole is taken by every app that uses the kit, not only by the one that found
   it: the change names the apps and their tasks.
6. **Move the apps with one command.** An app pins the tag in several places that must agree (the
   `uses:` lines of its workflows, `tools/release-kit.pin`, and for the hub apps `Cargo.toml`,
   `ui/package.json` and both lock files). `python tools/bump-apps.py vX.Y.Z` edits all of them in
   `ai-desktop`, `docmap-desktop`, `tasks-desktop`, `rules-desktop` and `hub-desktop` (`--apps a,b` for some,
   `--dry-run` to look first, `--commit` to also commit each app; it never pushes), refreshes the lock
   files, and runs each app's own pin check. It refuses a tag that is not on origin, and skips an app
   whose files to edit hold uncommitted work. Then push each app and watch its CI.

   While working on appkit and an app at the same time, do not bump at all: use a `[patch]` in an
   uncommitted `.cargo/config.toml` of the app (README of appkit). Tag and bump when the change is done.
