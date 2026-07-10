---
name: release
description: Cut a release of bun-js-beforeparse. Pick the next semver version from git tags and the Unreleased changelog, update CHANGELOG.md and docs, then commit, tag, and push to trigger the GitHub Actions release workflow.
---

# Release bun-js-beforeparse

Use this skill when the user asks to "release", "cut a release", "publish", "ship a new version", or tag a new version of the project.

The release is fully automated: pushing a `v*.*.*` git tag to `origin` triggers `.github/workflows/release.yml`, which builds 8 platform `.node` binaries and publishes to npm. Your job is only to pick the version, update changelog/docs, and push the tag.

## Preconditions (verify before anything else)

1. Working tree must be clean: `git status --short` should be empty. If not, ask the user to commit/stash first.
2. Current branch must be `main` (or the default release branch). Check with `git rev-parse --abbrev-ref HEAD`.
3. Local and remote must be in sync: `git fetch origin` then confirm `git status` says "up to date with origin/main". If behind/ahead, ask the user to resolve first.

Do not proceed if any precondition fails — releasing from a dirty or unpushed tree creates broken tags.

## Step 1 — Gather version state

Collect and show the user these facts:

```sh
# Current version declared in package.json (no 'v' prefix, e.g. 0.1.0)
node -p "require('./package.json').version"   # or read the JSON directly

# Latest local tag
git describe --tags --abbrev=0 2>/dev/null

# All local tags (sorted)
git tag --list "v*" | sort -V

# Latest remote tag (what has actually been released)
git ls-remote --tags origin "v*" | awk -F/ '{print $3}' | sort -V | tail -5
```

Also read the `[Unreleased]` section of `CHANGELOG.md` to see what changes are pending.

## Step 2 — Check for an existing unreleased tag

Before computing a new version, check whether a tag already exists that has not been released:

- A **local tag** that does **not** appear in `git ls-remote --tags origin` (created locally but never pushed → release never ran).
- A tag that exists on remote but whose version is **greater than** the `package.json` version (prematurely created).

If such a tag exists (e.g. local `v0.2.0` not on remote), **ask the user** whether to reuse that existing version/tag instead of bumping to a new one. Reusing means: set `package.json` to that version, finalize changelog/docs under that version heading, then push the existing tag (do not create a new tag). Default to reusing unless the user says otherwise.

## Step 3 — Determine the next version (when no reusable tag exists)

Base the bump on the `[Unreleased]` changelog content and `git log <latest-tag>..HEAD --oneline`:

- **patch** (x.Y.Z → x.Y.Z+1): only bug fixes / `fix:` commits, no new features, no breaking changes.
- **minor** (x.Y.z → x.Y+1.0): new features / `feat:` commits, backward-compatible additions (new APIs, new test coverage, perf improvements). This is the common case.
- **major** (x.y.z → (x+1).0.0): breaking changes, removed/renamed public APIs, or behavior changes that existing users must adapt to.

If the changes look like a **major** bump, **stop and ask the user for confirmation** before using it. Do not cut a major release unprompted. Patch and minor are fine to propose and proceed.

State the proposed version and the one-line rationale, then proceed (e.g. "Proposed: 0.2.0 (minor — adds async transform support, a new feature, backward compatible)."). You do not need to wait for confirmation on patch/minor unless the user earlier asked to confirm.

**Version format**: `package.json` holds `X.Y.Z` (no `v`); the git tag is `vX.Y.Z` (with `v`). Keep the two in sync.

## Step 4 — Update CHANGELOG.md

`CHANGELOG.md` follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) with three standard subsections: `### Added`, `### Changed`, `### Fixed` (use `### Removed` / `### Deprecated` / `### Security` only if relevant).

1. Convert the current `## [Unreleased]` heading into a dated release heading:
   `## [X.Y.Z] - YYYY-MM-DD` using today's date in ISO format.
2. Insert a fresh `## [Unreleased]` section **above** the new dated one, with empty standard subsections so future edits slot in:
   ```markdown
   ## [Unreleased]

   ### Added

   ### Changed

   ### Fixed
   ```
3. Preserve the existing bullet content under the dated heading unchanged (it was already written). Only reorganize headings.

Do not invent changelog entries that aren't already there — the `[Unreleased]` content is the source of truth. If `[Unreleased]` is empty, ask the user whether there's actually anything to release.

## Step 5 — Update documentation if stale

Check whether docs reflect the pending changes; update only what is now outdated:

- `README.md` — API examples, the "How to publish" section example tag (`git tag v0.1.0` is just an illustration; only update if the canonical example should track the new version), feature lists, platform tables.
- `AGENTS.md` — architectural notes, test counts, described behavior. Update counts/descriptions only if the release changes them (e.g. "13 tests" → new count, new gotchas).

If the docs already reflect the changes, do not edit them. Do not bump version strings that are merely illustrative examples unless they mislead.

## Step 6 — Bump package.json

Set the `version` field in `package.json` to the new `X.Y.Z` (no `v` prefix). Edit only that field.

## Step 7 — Commit

Stage the bumped files (typically `package.json`, `CHANGELOG.md`, and any docs):

```sh
git add package.json CHANGELOG.md README.md AGENTS.md   # only those that changed
git status --short   # confirm no stray .node / target / node_modules files staged
```

Commit using the project's Conventional Commits + gitmoji convention (see `AGENTS.md`):

```
🔧 chore: release vX.Y.Z
```

(No body needed unless a migration note is genuinely useful. Subject capitalized, imperative mood, no trailing punctuation.)

## Step 8 — Tag and push to trigger the release

```sh
git tag vX.Y.Z
git push origin main
git push origin vX.Y.Z
```

Pushing the `v*.*.*` tag is what starts the Release workflow. Confirm the push succeeded.

## Step 9 — Report and verify

Tell the user:
- The released version and what it included (one-line summary from the changelog).
- That the Release workflow is now running on GitHub Actions (multi-platform build + npm publish). Provide the Actions URL: `https://github.com/dianlight/bun-js-beforeparse/actions/workflows/release.yml`.
- Any manual follow-up needed (there usually is none; the workflow handles npm publish + GitHub Release creation automatically). The `NPM_TOKEN` secret must already be set.

Do not amend or delete the tag after pushing — a bad release should be a new version, not a force-push.

## Summary checklist

- [ ] Tree clean, on `main`, in sync with origin
- [ ] Checked for an existing unreleased local tag → asked to reuse if found
- [ ] Computed next version (patch/minor common; major → ask user)
- [ ] `CHANGELOG.md`: dated heading added, fresh `[Unreleased]` on top
- [ ] Docs updated only where stale
- [ ] `package.json` version bumped
- [ ] Commit `🔧 chore: release vX.Y.Z`
- [ ] Tagged `vX.Y.Z` and pushed to origin
- [ ] Reported Actions URL to user