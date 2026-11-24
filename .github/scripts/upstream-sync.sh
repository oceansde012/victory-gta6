#!/usr/bin/env bash
# Keeps this fork in step with its two upstreams. Run by .github/workflows/upstream-sync.yml.
#
# 1. Fetches Marvisak/VIctory main and fxceless/victory-fork main.
# 2. If either has commits that our main lacks, merges them (fxceless first,
#    it already sits on top of Marvisak) into the branch `sync/upstream` and
#    keeps ONE pull request "Sync upstream" open against main.
#    On a merge conflict it stops at the conflicting upstream, says so in the
#    PR and labels it `sync-conflict`; the conflict is resolved by hand.
# 3. Writes a list of the upstreams' open and recently merged pull requests to
#    the branch `upstream-watch` (file README.md), so a task can check whether
#    the same work is already in progress upstream.
#
# DRY_RUN=1 does everything locally and pushes nothing.
set -euo pipefail

REPO="${GITHUB_REPOSITORY:-oceansde012/victory-gta6}"
SYNC_BRANCH=sync/upstream
WATCH_BRANCH=upstream-watch
# name|owner/repo ; order = merge order
UPSTREAMS=("fxceless|fxceless/victory-fork" "marvisak|Marvisak/VIctory")
DRY_RUN="${DRY_RUN:-0}"

git config user.name  "upstream-sync[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

for u in "${UPSTREAMS[@]}"; do
  name=${u%%|*}; slug=${u#*|}
  git remote get-url "$name" >/dev/null 2>&1 || git remote add "$name" "https://github.com/$slug"
done
git fetch --no-tags --quiet origin main
git fetch --no-tags --quiet origin "$SYNC_BRANCH" 2>/dev/null || true
for u in "${UPSTREAMS[@]}"; do git fetch --no-tags --quiet "${u%%|*}" main; done

# ---------------------------------------------------------------- 1. sync PR
behind=(); conflict=""
for u in "${UPSTREAMS[@]}"; do
  name=${u%%|*}
  git merge-base --is-ancestor "$name/main" origin/main || behind+=("$name")
done

if [ ${#behind[@]} -eq 0 ]; then
  echo "main already contains both upstreams."
else
  # Skip if the existing sync branch already has exactly what we would merge.
  up_to_date=0
  if git rev-parse --verify -q "origin/$SYNC_BRANCH" >/dev/null; then
    up_to_date=1
    git merge-base --is-ancestor origin/main "origin/$SYNC_BRANCH" || up_to_date=0
    for name in "${behind[@]}"; do
      git merge-base --is-ancestor "$name/main" "origin/$SYNC_BRANCH" || up_to_date=0
    done
  fi

  if [ $up_to_date -eq 1 ]; then
    echo "sync branch already up to date with: ${behind[*]}"
  else
    git checkout --quiet -B "$SYNC_BRANCH" origin/main
    merged=()
    body=$'Automatic merge of upstream changes into `main`. Updated by `.github/workflows/upstream-sync.yml`; do not push to this branch by hand except to resolve a conflict.\n'
    for u in "${UPSTREAMS[@]}"; do
      name=${u%%|*}; slug=${u#*|}
      [[ " ${behind[*]} " == *" $name "* ]] || continue
      # Already brought in by an earlier merge (fxceless contains Marvisak)?
      git merge-base --is-ancestor "$name/main" HEAD && continue
      log=$(git log --no-merges --format="- \`%h\` %s (%an)" "HEAD..$name/main" | head -50)
      if git merge --no-edit --quiet -m "Merge $slug main ($(git rev-parse --short "$name/main"))" "$name/main"; then
        merged+=("$name")
        body+=$'\n'"### From $slug ($(git rev-parse --short "$name/main"))"$'\n'"$log"$'\n'
      else
        git merge --abort
        conflict="$slug"
        body+=$'\n'"### ⚠️ Conflict: $slug main ($(git rev-parse --short "$name/main")) does not merge cleanly"$'\n'
        body+=$'Merge it by hand: `git fetch '"$name"$' && git checkout '"$SYNC_BRANCH"$' && git merge '"$name"$'/main`, resolve, push.\n'"$log"$'\n'
        break
      fi
    done

    if [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ]; then
      echo "Nothing merged cleanly (conflict: $conflict)."
      [ -n "$conflict" ] && echo "::warning::upstream $conflict conflicts with main; nothing could be merged automatically"
    elif [ "$DRY_RUN" = 1 ]; then
      echo "DRY_RUN: would push $SYNC_BRANCH ($(git rev-parse --short HEAD)) merged=${merged[*]} conflict=$conflict"
      printf '%s\n' "$body"
    else
      git push --quiet --force origin "$SYNC_BRANCH"
      pr=$(gh pr list -R "$REPO" --head "$SYNC_BRANCH" --state open --json number --jq '.[0].number // empty')
      if [ -z "$pr" ]; then
        gh pr create -R "$REPO" --base main --head "$SYNC_BRANCH" --title "Sync upstream (Marvisak, fxceless)" --body "$body"
        pr=$(gh pr list -R "$REPO" --head "$SYNC_BRANCH" --state open --json number --jq '.[0].number')
      else
        gh pr edit "$pr" -R "$REPO" --body "$body"
      fi
      if [ -n "$conflict" ]; then
        gh label create sync-conflict -R "$REPO" --color d73a4a --description "Upstream does not merge cleanly" 2>/dev/null || true
        gh pr edit "$pr" -R "$REPO" --add-label sync-conflict || true
      else
        gh pr edit "$pr" -R "$REPO" --remove-label sync-conflict 2>/dev/null || true
      fi
    fi
  fi
fi

# ------------------------------------------------------- 2. upstream watch list
watch=$(mktemp -d)
{
  echo "# Upstream watch"
  echo
  echo "Open and recently merged pull requests in the upstreams. Before starting a task,"
  echo "search this page (and the upstreams' issues) for the same area: if the work is"
  echo "already in progress there, wait for it or cherry-pick it instead of redoing it."
  echo "Updated automatically by \`.github/workflows/upstream-sync.yml\`."
  if [ -n "$conflict" ]; then
    echo
    echo "> ⚠️ **$conflict main does not merge cleanly into our main.** Merge it by hand (see docs/upstream-sync.md)."
  fi
  for u in "${UPSTREAMS[@]}"; do
    name=${u%%|*}; slug=${u#*|}
    echo
    echo "## $slug"
    echo
    echo "main: \`$(git rev-parse --short "$name/main")\` — $(git log -1 --format='%s' "$name/main")"
    if git merge-base --is-ancestor "$name/main" origin/main; then echo "(already in our main)"; else echo "(**not yet in our main**: $(git rev-list --count "origin/main..$name/main") commits)"; fi
    echo
    echo "### Open pull requests"
    echo
    gh api --paginate "repos/$slug/pulls?state=open&per_page=100&sort=updated&direction=desc" \
      --jq '.[] | "- [#\(.number)](\(.html_url)) \(.title) — @\(.user.login)\(if .draft then " (draft)" else "" end)\(if (.labels|length)>0 then " `" + ([.labels[].name]|join("`, `")) + "`" else "" end)"' \
      || echo "_(could not list)_"
    echo
    echo "### Recently merged (last 30)"
    echo
    gh api "repos/$slug/pulls?state=closed&per_page=60&sort=updated&direction=desc" \
      --jq '[.[] | select(.merged_at != null)] | sort_by(.merged_at) | reverse | .[:30][] | "- [#\(.number)](\(.html_url)) \(.title) — merged \(.merged_at[:10])"' \
      || echo "_(could not list)_"
  done
} > "$watch/README.md"

if [ "$DRY_RUN" = 1 ]; then
  echo "DRY_RUN: watch list written to $watch/README.md"; exit 0
fi

wt=$(mktemp -d)
if git fetch --no-tags --quiet origin "$WATCH_BRANCH" 2>/dev/null; then
  git worktree add --quiet "$wt" FETCH_HEAD
else
  git worktree add --quiet --detach "$wt"
  git -C "$wt" checkout --quiet --orphan "$WATCH_BRANCH"
  git -C "$wt" rm -rfq . >/dev/null
fi
cp "$watch/README.md" "$wt/README.md"
git -C "$wt" add README.md
if git -C "$wt" diff --cached --quiet; then
  echo "watch list unchanged"
else
  git -C "$wt" commit --quiet -m "Upstream watch: update"
  git -C "$wt" push --quiet origin "HEAD:refs/heads/$WATCH_BRANCH"
fi
