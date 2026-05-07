# Porter GitHub App

A single-purpose GitHub App that holds the only identity allowed to push
release tags (`v*`) on a porter-managed repository. Pattern borrowed from
[Palantir's Autorelease][autorelease]. Once installed and the matching
ruleset is in place, every release tag in the repo's history demonstrably
originates from porter — humans can't `git tag && git push` to bypass it.

## Install

1. **Create the App in your org**:

   ```sh
   # The manifest endpoint accepts a JSON payload with the shape gh's
   # `app create-from-manifest` produces from app/manifest.yml.
   gh api --method POST \
     /organizations/$ORG/settings/apps/from-manifest \
     --input app/manifest.yml
   ```

   Or use the GitHub UI: **Org settings → Developer settings → GitHub
   Apps → New GitHub App → "Create from a manifest"**, paste the
   manifest, accept.

2. **Generate a private key** for the App and download the `.pem`.

3. **Install** the App on the repo(s) you want porter to manage. From
   the App's settings page → "Install App" → choose the repo.

4. **Add repo secrets**:

   ```sh
   gh secret set PORTER_APP_ID --body "<app-id>" --repo $ORG/$REPO
   gh secret set PORTER_APP_PRIVATE_KEY --body "$(cat porter.private-key.pem)" --repo $ORG/$REPO
   ```

5. **Wire the workflows**: each `version.yml` / `release.yml` consumer
   exchanges these secrets for an installation token before calling
   porter's reusable workflows. A typical wrapper:

   ```yaml
   jobs:
     release:
       runs-on: ubuntu-latest
       steps:
         - id: app-token
           uses: actions/create-github-app-token@v2
           with:
             app-id: ${{ secrets.PORTER_APP_ID }}
             private-key: ${{ secrets.PORTER_APP_PRIVATE_KEY }}

       # then call porter's reusable workflow with secrets:
       #   uses: tractorbeamai/porter/.github/workflows/release.yml@v0
       #   secrets:
       #     app-token: ${{ steps.app-token.outputs.token }}
   ```

6. **Lock down tag pushes**: install the ruleset (the next section).
   Until you do, the App is just a release author; humans can still
   bypass it.

## Repo ruleset (the actual lockdown)

`tools/install-ruleset.sh` posts a [GitHub repository ruleset][rulesets]
that:

- Targets `refs/tags/v*` on push.
- Sets `enforcement: active`.
- Names the porter App installation as the sole `bypass_actor`.
- Rejects any tag-creation/update push from non-bypass identities.

Run it after the App is installed:

```sh
GH_TOKEN=$(gh auth token) \
ORG=tractorbeamai \
REPO=porter \
PORTER_APP_INSTALLATION_ID=12345678 \
tools/install-ruleset.sh
```

The installation ID is shown in the App's installation settings page
URL: `…/installations/<id>`.

## Verify the boundary

The verification step in the plan: as a developer, attempt
`git tag v0.0.99 && git push origin v0.0.99`. The push must be rejected
with a ruleset violation message. Only the porter App's installation
token can create or move `v*` refs.

[autorelease]: https://blog.palantir.com/how-palantir-secures-source-control-105c49079eae
[rulesets]: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets
