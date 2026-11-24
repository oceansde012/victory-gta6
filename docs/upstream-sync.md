# Keeping up with upstream

## Where the history comes from

```
Marvisak/VIctory  main ──●──●──●──(ffe310e)──●──●…        upstream, GPL-3.0-or-later
                                     \
fxceless/victory-fork main            ●──●──●──●──●──●──●  7 linear commits on top of ffe310e
                                                         \
oceansde012/victory-gta6 main                             (our work)
```

- fxceless's fork was renamed from `fxceless/victory-gta6` to
  `fxceless/victory-fork` on 2026-11-24 (GitHub redirects the old URL; the
  scripts use the new one). It is the code behind its author's Rumble streams.
- `fxceless/victory-fork` is an ordinary linear branch, not a squash over a
  rewritten history: its first commit `578d459` ("this fork's changes on top of
  VIctory main (ffe310e)") squashes fxceless's early changes, followed by six
  regular commits (including PR #1 by ZazaJr24). Marvisak's history below them
  is intact, so **both upstreams merge into our `main` with a plain
  `git merge`**: no rebase, no cherry-pick.
- When this fork was created, our `main` was `fxceless/main` (`1684989`), so it
  already contained Marvisak up to `ffe310e`.
- Merge order: fxceless first (it sits on Marvisak), then Marvisak (whatever
  landed upstream after the commit fxceless is based on).
- **Never rewrite our history** (no rebasing `main` onto upstream): merges
  only, otherwise every upstream update breaks open branches.

## Automation (every 10 minutes)

`.github/workflows/upstream-sync.yml` runs `.github/scripts/upstream-sync.sh`:

1. Fetches `main` of both upstreams.
2. If either has commits our `main` lacks, merges them into the branch
   `sync/upstream` and keeps **one** pull request "Sync upstream (Marvisak,
   fxceless)" open against `main` (updated in place, never duplicated). Its
   description lists the incoming commits.
3. On a conflict the merge stops at the conflicting upstream, the PR gets the
   `sync-conflict` label, and a ⚠️ note appears at the top of the
   `upstream-watch` branch. Resolve by hand (or ask Claude):
   ```
   git fetch origin && git fetch marvisak && git fetch fxceless
   git checkout -B sync/upstream origin/main
   git merge fxceless/main   # then: git merge marvisak/main
   # resolve, git commit, git push -f origin sync/upstream
   ```
4. Refreshes the **`upstream-watch`** branch (file `README.md`): open and the
   last 30 merged pull requests of both upstreams, and how many commits of each
   are not yet in our `main`. It commits only when the list actually changed.

If neither upstream moved, a run pushes nothing.

Required repository settings: Settings → Actions → General → Workflow
permissions: "Read and write permissions" and "Allow GitHub Actions to create
and approve pull requests".

GitHub limits: `*/10` cron is best effort and runs are often 5–20 minutes late
under load; scheduled workflows are disabled on forks until Actions are enabled
in the Actions tab; GitHub disables the schedule after 60 days without commits.
A PR opened with `GITHUB_TOKEN` does not trigger `CI`: re-run it by hand. The
repository is public, so Actions minutes are free.

Manual run: Actions → Upstream sync → Run workflow.

## Not doing the same work twice

Before any task (code, fix, optimisation):

1. Open the `upstream-watch` branch and search it for the task's area
   (shaders, ray tracing, shadows, barriers, pipelines, frame rate, ...).
2. Search the issues and pull requests of both upstreams, closed ones included:
   `https://github.com/Marvisak/VIctory/pulls?q=<terms>` and
   `https://github.com/fxceless/victory-fork/pulls?q=<terms>`.
3. If something turns up:
   - merged upstream → wait for the sync PR (or merge it right away);
   - open pull request → wait, or take its commits early
     (`git fetch marvisak pull/<N>/head:up-<N> && git cherry-pick ...`) and record
     that in `docs/fork-tracker.md` so the later upstream merge is not a surprise;
   - discussed in an issue → take its conclusions into account.
4. Only if nothing exists, do the work ourselves and record the task in
   `docs/fork-tracker.md`.

Code from Marvisak and fxceless may be taken (both GPL-3.0-or-later, like this
fork). **XCL (ivall/xcl, GPL-2.0-only) is a source of ideas only: never copy
its code.**
