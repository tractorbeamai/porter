#!/usr/bin/env bash
# Install (or update) the repository ruleset that makes the porter App
# the sole identity allowed to push `v*` tag refs. Idempotent: looks up
# any existing ruleset named "porter-tag-protection" and PUTs to it,
# otherwise POSTs a new one.
#
# Required environment:
#   GH_TOKEN                       — token with admin on the target repo
#   ORG                            — owning org (e.g. tractorbeamai)
#   REPO                           — repo name (e.g. constellation)
#   PORTER_APP_INSTALLATION_ID     — installation ID for the porter App on this repo

set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${ORG:?ORG is required}"
: "${REPO:?REPO is required}"
: "${PORTER_APP_INSTALLATION_ID:?PORTER_APP_INSTALLATION_ID is required}"

NAME="porter-tag-protection"

read -r -d '' BODY <<JSON || true
{
  "name": "${NAME}",
  "target": "tag",
  "enforcement": "active",
  "bypass_actors": [
    {
      "actor_id": ${PORTER_APP_INSTALLATION_ID},
      "actor_type": "Integration",
      "bypass_mode": "always"
    }
  ],
  "conditions": {
    "ref_name": {
      "include": ["refs/tags/v*"],
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

The only identity that may push v* tags is now installation
${PORTER_APP_INSTALLATION_ID} of the porter App.
MSG
