# Native context entry for Codex

This Git-tracked adapter also serves Codex tasks in other repositories when the
machine's `~/.codex/AGENTS.md` points here. Use the checkout containing this file
as the ontology root. Its `scripts/brain.sh` enters that checkout; a built binary,
local `.env`, and the configured PostgreSQL store are required. The README owns
installation, recovery, and the expected store ID.

- At the start of each task, send `{"op":"identity"}` to `bash scripts/brain.sh context`
  from this checkout and compare `store_id` with the README's local store ID.
  Then raw-read `{"op":"read","scope":"profile","path":"preferences/agent-operating-preferences.md"}`.
  Read the complete successful UTF-8 output, including frontmatter. A failed or
  truncated read and a parsed search/document preview do not load a policy.
- For a review or evaluation that may produce findings, also raw-read the exact
  `profile` rule `rules/common-review-quality.md`. Follow the baseline rule's
  task-specific reads without treating this bootstrap as a grant to `personal`
  or `work`.
- Enter broader native context only for explicitly requested durable context or
  cross-repository personal/work evidence, a career task needing stored experience,
  or a local adapter that requires it. For applications, résumés, portfolios, or
  case studies, read `vault/personal/index.md` and follow its routing. Read
  `profile/preferences/context-vault-operating-model.md` for source ownership,
  native writing, and cleanup; use its Harness boundary for agent-authored native
  changes. Keep every lookup scoped and do not transmit context through external
  tools or networks without the user's instruction for that exact transfer.
- After the first failed native read, stop further reads and run
  `python3 scripts/connection.py check --target database` from this checkout once.
  A `docker_permission` result is a caller permission limit: retry the same scoped
  read and check with permitted escalation, or report access unverified. For an
  actual connection failure, follow the README's existing recovery path. Wrong
  store, missing material, pending apply, and unavailable projection are not DB
  outages; do not repair or reimport them.
- Repository-local `AGENTS.md` files still add instructions for their own trees.
  Logical `vault/...` paths identify native scoped materials, not old filesystem
  files. Reading the baseline alone does not start Harness or grant other scopes.
