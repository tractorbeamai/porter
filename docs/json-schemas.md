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
  "current": "0.5.2",
  "next": "0.5.3",
  "bump": "minor",
  "pr_title": "Version Packages: 0.5.3",
  "changesets": [
    {
      "path": ".changeset/feat-attest.md",
      "bump": "minor",
      "summary": "Add the attest subcommand."
    }
  ]
}
```

| Field         | Type                            | When                             |
| ------------- | ------------------------------- | -------------------------------- |
| `current`     | string (semver)                 | always                           |
| `next`        | string (semver) or `null`       | `null` iff `changesets` is empty |
| `bump`        | `"patch"` / `"minor"` / `"major"` or `null` | `null` iff `changesets` is empty |
| `pr_title`    | string or `null`                | `null` iff `changesets` is empty |
| `changesets`  | array of objects                | always; empty when none pending  |
| `changesets[].path`    | string (path relative to repo root) | always |
| `changesets[].bump`    | `"patch"` / `"minor"` / `"major"`   | always |
| `changesets[].summary` | string (full body of the changeset) | always |

Notes:
- `next` follows the cargo / Changesets pre-1.0 convention. See [How
  `next` is computed](../README.md#how-next-is-computed) in the README.
- `pr_title` is the rolling Version PR title rendered from porter.toml
  `[release].version_pr_title` (default `Version Packages: {version}`),
  with `{version}`/`{tag}` substituted for the next version. `version.yml`
  uses it as the PR title and bump-commit subject.
- `summary` is the changeset body verbatim, including any literal
  `\n---` lines inside the prose. It's the same string that lands in
  `CHANGELOG.md` and the GitHub Release notes.

**Consuming this in bash:**

```sh
status=$(porter status --json)
next=$(echo "$status" | jq -r '.next // empty')
if [[ -z "$next" ]]; then
  echo "no pending changesets"
  exit 0
fi
```

`jq -r '.next // empty'` is the canonical way to coalesce the
`null`-when-empty case; that's what porter's own version.yml does.

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
      "package": "porter-cli",
      "target": "x86_64-unknown-linux-gnu",
      "runner": "ubuntu-latest"
    },
    {
      "id": "cli-binary-porter-aarch64-apple-darwin",
      "kind": "cli-binary",
      "name": "porter",
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
with `id`, `kind`, `name`, plus a kind-specific subset of fields.

**Common fields (every row):**

| Field    | Description                                              |
| -------- | -------------------------------------------------------- |
| `id`     | Unique row identifier; used as the matrix job name.      |
| `kind`   | `"cli-binary"`, `"oci-image"`, `"helm-chart"`, `"npm-package"`, or `"python-wheel"`. |
| `name`   | The artifact's `name` field from porter.toml.            |
| `runner` | GitHub-hosted runner label (`ubuntu-latest`, `macos-14`, etc.). |

**Kind-specific fields:**

| Kind            | Fields                                              |
| --------------- | --------------------------------------------------- |
| `cli-binary`    | `package`, `target`                                 |
| `oci-image`     | `context`, `dockerfile`, `registry`, `platforms`    |
| `helm-chart`    | `chart`, `registry`                                 |
| `npm-package`   | `path`, `registry`                                  |
| `python-wheel`  | `path`                                              |

Fields that don't apply to a given kind are omitted (not `null`) so
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
