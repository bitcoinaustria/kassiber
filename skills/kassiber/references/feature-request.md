# Adding a Feature Request

Repeatable playbook for turning a Kassiber product idea into a researched,
PR-ready GitHub issue in `bitcoinaustria/kassiber`. Use it when the user asks
to add, draft, research, or publish a feature request. The user-facing entry
point is the `/feature-request` command.

This is a three-part flow:

1. **Research** the live backlog and the repo truth.
2. **Draft** a clean issue in the house style, with explicit guardrails.
3. **Publish** only after user approval, then preserve the issue as PR context.

---

## Part 1 - Research first

Start with live GitHub state, not memory:

```bash
gh issue list --repo bitcoinaustria/kassiber --state open --limit 100 \
  --json number,title,body,labels,url,updatedAt
```

If a GitHub connector is available, prefer its issue search/read/create tools
over scraping terminal output. Search by the idea's nouns and by adjacent
local-first themes before drafting. Read likely duplicates or dependencies,
then decide one of:

- **Duplicate / amend existing issue** - report the existing issue and suggest a
  comment or refinement instead of opening a new issue.
- **Related but distinct** - draft a new issue with a "Relationship to existing
  issues" section and cross-reference the related numbers.
- **New** - draft a standalone issue and explain why it belongs in the backlog.

Verify claims about current behavior against the repo. Use `rg` first; read the
specific code, tests, docs, or TODO lines that prove the gap. Do not write
"verified" unless you actually inspected the current tree or live GitHub state.
If a useful claim is still uncertain, mark it as an open question.

The current backlog style for moat-building issues is concise but evidence-rich:
outcome-oriented title, `## Summary`, `## Problem / motivation`, `## Proposed
solution`, `## Guardrails / scope`, relationship/cross-ref notes, and a final
effort/category line. Recent examples to refresh live before copying patterns
include the local adversary, egress-auditor, privacy-score, and P2P sync issues.

---

## Part 2 - Draft the issue

Use this skeleton for most feature requests:

```markdown
## Summary

One short paragraph naming the capability and the user-visible outcome.

## Problem / motivation

Why this matters, with the local-first advantage stated concretely rather than
as marketing. If a cloud SaaS would become a honeypot for this workflow, say
what data it would need to ingest or correlate.

## Proposed solution

The smallest useful product shape. Prefer phases for large ideas. Name existing
Kassiber substrates that make the feature plausible.

## Guardrails / scope

- Local-only / no new egress rules.
- Secret, descriptor, xpub, attachment, and AI-provider boundaries.
- Non-goals and anti-patterns.
- Failure/degraded states that should be honest to the user.

## PR context

- Current behavior verified:
- Likely files / surfaces:
- Acceptance checks:
- Non-goals to preserve:

## Relationship to existing issues

- #123 - how it relates.

_Effort: M - Category: local-first moat - cross-ref #123_
```

Keep the issue useful for a future PR:

- Put implementation facts in `## PR context`, not only prose motivation.
- Include file paths or subsystem names when they were verified.
- Include acceptance checks a PR author can turn into tests or manual QA.
- Make guardrails testable: "no new egress", "RAM-only", "local Ollama only",
  "no generic shell/filesystem access", "do not sync derivable state", etc.
- For sensitive full-ledger, source-of-funds, OCR, privacy-graph, tax-report, or
  inheritance surfaces, default to local computation and hard-disable off-device
  AI unless the user explicitly asks for a different product boundary.

Local-first moat framing:

- Prefer "Kassiber already has the user's local ground truth" over vague
  "SaaS can't do this" claims.
- It is fair to call out cloud SaaS honeypots when the feature would otherwise
  require uploading a full ledger, watch-only graph, attachments, source-of-funds
  dossier, tax lots, endpoint exposure, or AI prompt context.
- Do not overclaim for parity work. If cloud tools can do the feature too, frame
  Kassiber's edge as offline cache, user-verifiable egress, auditability, or
  lower leak surface.
- Never propose a Kassiber account, hosted relay, telemetry path, arbitrary file
  read, raw shell, raw wallet config, seed/private-key handling, or broad
  third-party upload as the default answer to a privacy problem.

---

## Part 3 - Publish

Show the draft to the user first unless they explicitly asked to publish without
another stop. Ask for approval with the exact title and body that will be sent.

Publish with either the GitHub connector's issue-create tool or:

```bash
gh issue create --repo bitcoinaustria/kassiber \
  --title "<title>" \
  --body-file /tmp/kassiber-feature-request.md
```

Do not invent labels or milestones. Add labels only if live repo state shows the
exact label already exists and it clearly applies.

After publishing:

- Return the issue URL and number.
- If implementation starts immediately, read the issue body and comments first.
- Use a branch name that keeps the issue visible, e.g.
  `codex/issue-123-<short-slug>`.
- Reference the issue in the PR body with `Refs #123` or `Closes #123` depending
  on whether the PR fully completes it.
- If implementation discovers that the issue spec is wrong or incomplete, update
  the PR notes or add an issue comment rather than letting the durable context
  drift silently.

## Done Check

Before calling the feature request ready:

- Live open issues were searched and likely overlaps were considered.
- Repo-current claims are verified or explicitly marked as open questions.
- The local-first moat is specific, not fluffy.
- Guardrails protect Kassiber's privacy/accounting boundaries.
- The issue includes `## PR context` so a future implementation PR can start
  from the issue instead of reconstructing intent from chat.
