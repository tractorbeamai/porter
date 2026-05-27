# Getting started

A worked walkthrough from an empty repo to a shipped GitHub Release.
The example uses a Rust workspace because that's the path porter
itself exercises every day — for non-Rust projects, swap the
component's version source in step 1 and the rest is identical.

**Time budget:** ~20 minutes the first time, almost all of it
clicking through the GitHub App form in step 4. Subsequent repos take
~5 minutes once the App exists.

**Prerequisites:**

- A GitHub repo you can push to and configure rulesets on
  (admin access).
- An org you can create GitHub Apps in (owner access — the App lives
  at the org level, not the repo level).
- `gh` CLI logged in (`gh auth status`).
- `porter` installed locally for `porter add`. From a release tarball
  on the [releases page] or, if you're inside CI, you don't need it
  at all — workflows install it on the fly.

The example assumes `myorg/myrepo`. Substitute your own.

## 1. Add `porter.toml`

At the repo root:

```toml
[changesets]
directory = ".changeset"

# One group, one component: the workspace version is the release line and the
# same release builds the CLI. tag_prefix = "v" keeps bare vX.Y.Z tags (the
# default stem would be `myrepo/v`).
[[group]]
name = "default"
components = [
  { id = "myrepo", type = "cargo-workspace", path = "Cargo.toml", tag_prefix = "v",
    artifact = { kind = "cli-binary", package = "myrepo-cli" } },
]

[release]
changelog = "CHANGELOG.md"
```

If you're not shipping a binary (lib-only crate, internal service,
docs repo), drop the `artifact` — a component can be version-only,
and porter will still maintain versions and changelogs.

Non-Rust analogues for the component's version source:

```toml
# Node
{ id = "myrepo", type = "package-json", path = "package.json" }

# Helm chart
{ id = "myrepo", type = "helm-chart", path = "deploy/chart/Chart.yaml" }

# Anything else (e.g. Terraform pin)
{ id = "myrepo", type = "regex", path = "deploy/main.tf",
  pattern = 'image_tag\s*=\s*"(?P<version>v[0-9.]+)"' }
```

A group can hold several components (e.g. an SDK shipped for two languages);
porter rewrites their version sources in lockstep on every bump and refuses to
proceed if they ever disagree. Separate version lines go in separate
`[[group]]` blocks. See the README "Configuration" section.

Commit it: `git add porter.toml && git commit -m "chore: add porter.toml"`.

## 2. Cut your first changeset

```sh
porter add --bump minor --summary "Initial release."
git add .changeset/ && git commit -m "chore: initial changeset"
```

The bump category is *user-visible impact*, not diff size. For the
first release, `minor` is the conventional choice — it'll produce
`0.1.0` under pre-1.0 semver. See
[How `next` is computed](../README.md#how-next-is-computed) for the
table.

Run `porter status` to sanity-check:

```
porter status
default: 0.0.0 -> 0.1.0 (minor)
  minor  .changeset/initial-release.md  Initial release.
```

## 3. Add a CHANGELOG header

Create an empty `CHANGELOG.md` at the repo root so `porter release
notes` has something to work with on the first release:

```sh
echo "# Changelog" > CHANGELOG.md
git add CHANGELOG.md && git commit -m "chore: seed CHANGELOG"
```

(Not strictly required — `porter version` will create it for you on
the first bump — but having it in the repo from the start avoids a
small surprise diff later.)

## 4. Create and install the porter GitHub App

Follow [`app/README.md`](../app/README.md) for the click-through.
Summary:

1. Org settings → Developer settings → GitHub Apps → **New GitHub
   App**. Copy values from [`app/spec.yml`](../app/spec.yml).
2. Generate a private key on the App's settings page. Download the
   `.pem`.
3. Install the App on `myorg/myrepo`.
4. Add the repo secrets:

   ```sh
   gh secret set PORTER_APP_ID --repo myorg/myrepo --body "<app-id>"
   gh secret set PORTER_APP_PRIVATE_KEY --repo myorg/myrepo \
     < ~/Downloads/myorg-porter.YYYY-MM-DD.private-key.pem
   ```

   `PORTER_APP_ID` is the **App ID** (numeric, top of the App's
   About section). Don't confuse it with the installation ID — see
   [`app/README.md`](../app/README.md#repo-ruleset-the-actual-lockdown)
   for which is which.

5. Lock down the tag namespace:

   ```sh
   GH_TOKEN=$(gh auth token) \
   ORG=myorg \
   REPO=myrepo \
   PORTER_APP_ID=<app-id> \
   tools/install-ruleset.sh
   ```

   After this, only the porter App can push `v*` tags. Verify by
   trying `git tag v0.0.99-test && git push origin v0.0.99-test` —
   it must be rejected.

## 5. Add the version workflow

`.github/workflows/version.yml`:

```yaml
name: version
on:
  push:
    branches: [main]
permissions:
  contents: read
jobs:
  rolling-pr:
    uses: tractorbeamai/porter/.github/workflows/version.yml@v0
    secrets:
      app-id: ${{ secrets.PORTER_APP_ID }}
      app-private-key: ${{ secrets.PORTER_APP_PRIVATE_KEY }}
```

The workflow mints its own App token from these secrets in the job
that uses it — don't mint in a separate job and pass the token in, as
`create-github-app-token` revokes it when that job ends.

Commit it. On the next push to main, this opens a "Version Packages"
PR showing `0.0.0 → 0.1.0` and the rendered changelog entry.

## 6. Add the release workflow

`.github/workflows/release.yml`:

```yaml
name: release
on:
  push:
    branches: [main]
    paths: [CHANGELOG.md]
permissions:
  contents: read
jobs:
  release:
    uses: tractorbeamai/porter/.github/workflows/release.yml@v0
    secrets:
      app-id: ${{ secrets.PORTER_APP_ID }}
      app-private-key: ${{ secrets.PORTER_APP_PRIVATE_KEY }}
```

The `paths: [CHANGELOG.md]` filter is the trigger: merging the
Version PR changes the changelog, which fires this workflow, which
tags and publishes.

Commit it.

## 7. Push and merge

```sh
git push origin main
```

Within a few seconds, the `version` workflow runs and opens a Version
Packages PR. Review the diff (it should be exactly the version bump
and changelog entry), then merge it.

The merge triggers the `release` workflow:

1. `tag` job pushes `v0.1.0` via the App identity.
2. `build` matrix cross-compiles for each declared target.
3. `publish` job creates the GitHub Release with the four tarballs +
   `checksums.txt`.

`gh release view v0.1.0` should show all five assets.

## What you get on every subsequent release

The first release is the only one with manual setup. From here, the
loop is:

1. Author a changeset on any release-worthy PR (`porter add`).
2. Merge to main. The rolling Version PR updates automatically.
3. When you want to cut, merge the Version PR.
4. `release.yml` fires, tags, publishes. Done.

## Recommended additions (opt-in)

Once the core loop is working, these close the remaining ergonomic gaps:

- **PR-status comment.** A sticky comment on every PR showing what
  changesets it adds and the version it would produce on merge. See
  [README → Recommended setup](../README.md#recommended-setup-optional).
- **Auto-merge on label.** If you run policy-bot + bulldozer, copy
  [`.policy.yml`](../.policy.yml) and [`.bulldozer.yml`](../.bulldozer.yml)
  to your repo root. Adapt the approval rules to your trust model.
- **Renovate.** [`renovate.json`](../renovate.json) shows porter's
  setup — extend or replace for your org's conventions.

## When things go wrong

[`docs/runbooks.md`](runbooks.md) covers the failure modes that
actually happen: merge conflicts on the rolling Version PR, mid-flight
release failures, rolling back a bad release, ruleset rejections,
checksum failures.

[releases page]: https://github.com/tractorbeamai/porter/releases
