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
   number. The crates keep `version = "0.0.0"` in their manifests: the tag is the version. Keep the change
   backward compatible where you can: an addition (`signOut`, the `lang` option of `assemble`) is a minor, and a name that
   goes away stays as a forward to the new one until `rg` finds no app that uses it (the apps sit beside this checkout
   under `E:epos\<app>`); an app drops its own copy only after it took the tag.
3. **Tag and push.** `git tag -a vX.Y.Z -m "..."` on the green commit, `git push origin vX.Y.Z`.
4. **A tag is never moved or deleted.** A mistake is fixed in the next version. Each app checks in its
   CI that the commit its `Cargo.lock` pins is the one its tag names now, so a moved tag is noticed.
5. **Every app takes a fix of the guard.** A change to the guard, the pairing, the sessions or the
   limits that closes a hole is taken by every app that uses the kit, not only by the one that found
   it: the change names the apps and their tasks.
6. **Move the apps with one command.** An app pins the tag in several places that must agree (the
   `uses:` lines of its workflows, `tools/release-kit.pin`, and for the hub apps `Cargo.toml`,
   `ui/package.json` and both lock files; for pult, which has no page, `Cargo.toml` and `Cargo.lock`).
   `python tools/bump-apps.py vX.Y.Z` edits all of them in `ai-desktop`, `docmap-desktop`, `tasks-desktop`,
   `rules-desktop`, `testing-desktop`, `hub-desktop` and `pult` (`--apps a,b` for some,
   `--dry-run` to look first, `--commit` to also commit each app; it never pushes), refreshes the lock
   files, and runs each app's own pin check and, in an app that has one, its lock-age check (`--min-days`,
   default 7): a crate the refresh pulled in younger than that is named and the app is not done; `--hold-back`
   puts it back to the version of appkit's own `Cargo.lock` and checks again. It refuses a tag that is not on
   origin, and skips an app whose files to edit hold uncommitted work. Then push each app and watch its CI.

   A tag that asks an app for more than its pins (a change of its own code, such as `v0.56.0`: the hub is a
   window program) gets an entry in `BY_HAND` of the script. The bump prints it for each app whose pin is
   older ("by hand, not part of the bump commit"); the app makes that change in a commit of its own.

   Other sessions at work in the apps are no reason to wait: `--worktree` (implies `--commit`) makes each
   bump in a throw-away git worktree of the app's `HEAD`, so their uncommitted work neither skips the app nor
   slips into the lock files (`cargo update` resolves the whole workspace, and pnpm runs `--lockfile-only`
   there), and moves the commit into the clone's branch as a fast-forward. When their changes touch the same
   files, or the branch moved meanwhile, the commit waits on `bump/appkit-<tag>` and the app counts as not done.
   A branch with an upstream is compared with it after a `fetch` first. If it is only behind and the clone
   is clean, it is fast-forwarded before the bump. If it is behind with uncommitted changes, or has diverged,
   the bump is built on the upstream's tip, not on the old local state, and waits on `bump/appkit-<tag>`.
   The script names the way onto `main`: `git push origin bump/appkit-<tag>:main` once checked, then
   `git pull --rebase` in the clone once its own work is committed.

   While working on appkit and an app at the same time, do not bump at all: use a `[patch]` in an
   uncommitted `.cargo/config.toml` of the app (README of appkit). Tag and bump when the change is done.
