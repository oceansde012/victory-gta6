# Rules for Claude in this repository

- Everything written to the repository (commits, pull requests, docs, the fork
  tracker, this file) is in English.
- This is a fork of `fxceless/victory-fork` (formerly `fxceless/victory-gta6`), which itself sits on
  `Marvisak/VIctory`. History layout and automatic sync: `docs/upstream-sync.md`.
- Before any task, check that it has not already been done upstream: the
  `upstream-watch` branch plus a search of both upstreams' pull requests and
  issues (steps in `docs/upstream-sync.md`, "Not doing the same work twice").
  Record the task and what the check found in `docs/fork-tracker.md`.
- Upstream reaches `main` only by merge (the "Sync upstream" PR); never rewrite
  `main`'s history.
- XCL (ivall/xcl, GPL-2.0-only): ideas only, never copy its code.
- The game is built and run only on the owner's Windows PC (game on drive B:);
  it cannot run in a cloud Linux container, where `cargo test` works only in part.
- Remotes for local work:
  `git remote add marvisak https://github.com/Marvisak/VIctory`,
  `git remote add fxceless https://github.com/fxceless/victory-fork`.
