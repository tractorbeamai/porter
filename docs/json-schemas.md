# Machine-readable output schemas

Two porter subcommands emit JSON intended for downstream consumption:
`porter status --json` (read by [version.yml](../.github/workflows/version.yml)
and any custom CI you write) and `porter matrix --compact` (read by
[release.yml](../.github/workflows/release.yml) to fan out the build
job). Both are stable surfaces — adding new fields is a minor bump,
removing or renaming is a major.

## `porter status --json`

```sh
$ porter status --json
```

```json
{
  "groups": [
    {
      "name": "sdk",
      "current": "0.5.2",
      "next": "0.5.3",
      "bump": "minor",
      "tags": ["py-sdk/v0.5.3", "ts-sdk/v0.5.3"],
      "changesets": [
        {
          "path": ".changeset/feat-attest.md",
          "bump": "minor",
          "summary": "Add the attest subcommand.",
          "groups": ["sdk"]
        }
      ]
    },
    { "name": "app", "current": "1.2.0", "next": null, "bump": null, "tags": [], "changesets": [] }
  ],
  "pr_title": "Version Packages: 0.5.3"
}
```

| Field                  | Type                                        | When                                  |
| ---------------------- | ------------------------------------------- | ------------------------------------- |
| `groups`               | array of objects                            | always; one per `[[group]]`           |
| `groups[].name`        | string                                      | always                                |
| `groups[].current`     | string (semver)                             | always                                |
| `groups[].next`        | string (semver) or `null`                   | `null` iff the group has no pending changesets |
| `groups[].bump`        | `"patch"` / `"minor"` / `"major"` or `null` | `null` iff the group has no pending changesets |
| `groups[].tags`        | array of strings                            | the tags the group would cut; empty when not bumping |
| `groups[].changesets`  | array of objects (`path`, `bump`, `summary`, `groups`) | the changesets targeting this group |
| `pr_title`             | string or `null`                            | `null` iff no group has pending changesets |

Notes:
- `next` follows the cargo / Changesets pre-1.0 convention. See [How
  `next` is computed](../README.md#how-next-is-computed) in the README.
- `pr_title` is the rolling Version PR title rendered from porter.toml
  `[release].version_pr_title`. It's `null` exactly when nothing is
  releasable, so `version.yml` uses it as the release/skip signal. When a
  single group bumps, `{version}` is filled in; when several do, it falls
  back to the literal stem.
- `summary` is the changeset body verbatim. It's the same string that lands
  in the group's changelog and the GitHub Release notes.

**Consuming this in bash:**

```sh
status=$(porter status --json)
pr_title=$(echo "$status" | jq -r '.pr_title // empty')
if [[ -z "$pr_title" ]]; then
  echo "no pending changesets"
  exit 0
fi
```

`jq -r '.pr_title // empty'` coalesces the `null`-when-nothing-to-release
case; that's the skip check porter's own version.yml uses.

## `porter matrix --compact`

```sh
$ porter matrix --compact
```

```json
{
  "include": [
    {
      "id": "cli-binary-porter-x86_64-unknown-linux-gnu",
      "kind": "cli-binary",
      "name": "porter",
      "group": "default",
      "tag": "v0.5.3",
      "version": "0.5.3",
      "auth_kind": "none",
      "package": "porter-cli",
      "target": "x86_64-unknown-linux-gnu",
      "runner": "ubuntu-latest"
    },
    {
      "id": "cli-binary-porter-aarch64-apple-darwin",
      "kind": "cli-binary",
      "name": "porter",
      "group": "default",
      "tag": "v0.5.3",
      "version": "0.5.3",
      "auth_kind": "none",
      "package": "porter-cli",
      "target": "aarch64-apple-darwin",
      "runner": "macos-14"
    }
  ]
}
```

The output is shaped to plug directly into `strategy.matrix` in a
GitHub Actions workflow:

```yaml
matrix: ${{ fromJSON(needs.tag.outputs.matrix) }}
```

Top-level always has a single `include` key. Each row is a flat object
with common fields plus a kind-specific subset.

**Common fields (every row):**

| Field       | Description                                              |
| ----------- | -------------------------------------------------------- |
| `id`        | Unique row identifier; used as the matrix job name.      |
| `kind`      | `"cli-binary"`, `"oci-image"`, `"helm-chart"`, `"npm-package"`, or `"python-wheel"`. |
| `name`      | The component id (the artifact's public name).           |
| `group`     | The group the component releases in.                     |
| `tag`       | The git tag this artifact publishes under (`<id>/v<version>`, or a custom prefix). |
| `version`   | The bare version (`0.5.3`) — what images/charts are tagged with. |
| `auth_kind` | Registry auth: `"none"`, `"github-token"`, `"basic"`, or `"token"`. |
| `runner`    | GitHub-hosted runner label (`ubuntu-latest`, `macos-14`, etc.). |

**Kind-specific fields:**

| Kind            | Fields                                              |
| --------------- | --------------------------------------------------- |
| `cli-binary`    | `package`, `target`                                 |
| `oci-image`     | `context`, `dockerfile`, `registry`, `platforms`    |
| `helm-chart`    | `chart`, `registry`                                 |
| `npm-package`   | `path`, `registry`                                  |
| `python-wheel`  | `path`                                              |

Registry auth also adds `username_secret`/`password_secret` (basic) or
`token_secret` (token) — names of keys in the workflow's `registry-auth`
JSON secret. Fields that don't apply to a row are omitted (not `null`) so
the JSON stays compact.

**Filtering:** `--kind <name>` restricts the output to one kind.
porter's own self-release workflow uses this to plan only the
cli-binary rows:

```sh
porter matrix --kind cli-binary --compact
```

**Pretty printing:** drop `--compact` for indented output. The
machine-consuming workflows always use `--compact` because it's the
form they expect; humans inspecting the planning step usually want
the indented form.

## Stability

The shape of both outputs is part of porter's public surface. Changes
follow semver:

- **Adding a new field** to either output, or adding a new kind to the
  matrix's enum, is a minor bump.
- **Adding a new kind** to `bump` (e.g. `"prerelease"`) is a minor
  bump, but consumers parsing strictly should accept unknown values
  rather than failing.
- **Removing or renaming any field** is a major bump.
- **Reshaping** (e.g. moving `bump` under `changesets[]`-only, or
  removing the top-level `include` wrapper from the matrix) is a
  major bump.

If your CI parses these outputs with a strict schema, pin to a
specific `setup-porter@<version>` rather than `@v0`.
