#!/usr/bin/env bash
# Install (or update) the repository ruleset that makes the porter App
# the sole identity allowed to push porter's version tags. Idempotent:
# looks up any existing ruleset named "porter-tag-protection" and PUTs to
# it, otherwise POSTs a new one.
#
# Tag patterns: porter cuts one tag per published component. The default
# stem is `<id>/v` (e.g. `py-sdk/v0.4.1`), and a component may override it
# to bare `v` (e.g. `v0.1.0`). The two include patterns below cover both.
# A custom `tag_prefix` that matches neither needs its own pattern added.
#
# Required environment:
#   GH_TOKEN         — token with admin on the target repo
#   ORG              — owning org (e.g. tractorbeamai)
#   REPO             — repo name (e.g. constellation)
#   PORTER_APP_ID    — App ID for the porter App (numeric).
#                      Not the installation ID — for bypass_actors of
#                      actor_type: Integration, GitHub's rulesets API
#                      expects the App ID and rejects installation
#                      IDs with a 422 "Actor integration must be part
#                      of the ruleset source or owner organization".

set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${ORG:?ORG is required}"
: "${REPO:?REPO is required}"
: "${PORTER_APP_ID:?PORTER_APP_ID is required}"

NAME="porter-tag-protection"

read -r -d '' BODY <<JSON || true
{
  "name": "${NAME}",
  "target": "tag",
  "enforcement": "active",
  "bypass_actors": [
    {
      "actor_id": ${PORTER_APP_ID},
      "actor_type": "Integration",
      "bypass_mode": "always"
    }
  ],
  "conditions": {
    "ref_name": {
      "include": ["refs/tags/v*", "refs/tags/*/v*"],
      "exclude": []
    }
  },
  "rules": [
    { "type": "creation" },
    { "type": "update" },
    { "type": "deletion" },
    { "type": "non_fast_forward" }
  ]
}
JSON

api() {
  curl -fsSL \
    -H "Authorization: Bearer ${GH_TOKEN}" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "$@"
}

echo "Looking up existing ruleset on ${ORG}/${REPO}…"
existing=$(api "https://api.github.com/repos/${ORG}/${REPO}/rulesets" \
  | jq -r --arg name "${NAME}" '.[] | select(.name == $name) | .id')

if [[ -n "${existing}" ]]; then
  echo "Updating existing ruleset id=${existing}"
  api -X PUT "https://api.github.com/repos/${ORG}/${REPO}/rulesets/${existing}" \
    -d "${BODY}" \
    | jq '.id, .name, .target, .enforcement'
else
  echo "Creating new ruleset"
  api -X POST "https://api.github.com/repos/${ORG}/${REPO}/rulesets" \
    -d "${BODY}" \
    | jq '.id, .name, .target, .enforcement'
fi

cat <<MSG

Ruleset "${NAME}" is active on ${ORG}/${REPO}.

Verify: as a developer (not the porter App), run
    git tag v0.0.99 && git push origin v0.0.99
The push should be rejected with "rule violations on \`v0.0.99\`".

The only identity that may push v* tags is now the porter App
(App ID ${PORTER_APP_ID}).
MSG
