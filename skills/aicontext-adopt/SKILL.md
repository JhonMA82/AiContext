---
name: aicontext-adopt
description: Adopt a repository into AIContext persisted context (PROJECT_STATE, PATTERNS, consistency). Use for consolidated repos, forks, external boilerplates, or repos without AIContext. Do not auto-invoke on every task.
---

# aicontext-adopt

Adopt a repository once so future sessions start from persisted context instead of re-exploring. You interpret and classify; the `aicontext` CLI owns every deterministic fact.

## Principles

- **Determinism first.** If a fact can be obtained by a command, run the command. Never guess HEAD, version, scripts, workspaces, or tooling.
- **Persist once.** Expensive conclusions go to `.engineering/` so no session pays for them twice.
- **Progressive disclosure.** Never start with broad exploration. Expand only when evidence requires it.

```text
Level 0  aicontext status
Level 1  .engineering/PROJECT_STATE.md
Level 2  .engineering/PATTERNS.md
Level 3  aicontext search / tgrep / ast-grep
Level 4  CodeGraph (large repos only, when installed)
Level 5  targeted file reads
Level 6  broad exploration, only if evidence is still missing
```

## Flow

1. `aicontext scan --json` — the only starting point. Note complexity profile, package manager, version candidates, docs, CI.
2. If `.engineering/aicontext.toml` is missing, run `aicontext init --non-interactive`. It generates the manifest, `PROJECT_STATE.md` (generated block), `PATTERNS.md` placeholder, and `consistency.yml` stub. It never overwrites curated content.
3. Read minimal root docs only: `README.md`, `AGENTS.md`/`CLAUDE.md` if present, plus whatever `scan` listed under docs. Do not read code yet.
4. Draft the **curated** `PROJECT_STATE.md` section: purpose, current capabilities, constraints, pointers. Mark anything uncertain as `TBD`, never invent it.
5. Find patterns with the cheapest tool that answers the question:
   - `aicontext search <symbol>` (knowledge first, then text).
   - `ast-grep` for structural questions (deprecated APIs, layer violations).
   - CodeGraph only when `scan` classified the repo as large and it is installed.
6. For each candidate pattern, identify 2+ independent references before promoting it. Classify exactly one of:
   - `preferred` — repeated in production code AND backed by docs, an official generator, architecture, or a maintainer. One implementation is never `preferred` on its own.
   - `observed` — repeated but without authoritative backing.
   - `legacy` — superseded; reference it only as "do not copy".
   - `exception` — deliberate, justified deviation; record the reason.
   - `protected` — must not change without an explicit task (scaffolding, contracts).
7. Distinguish production from non-production code. `examples/`, `demo/`, `fixtures/`, `legacy/`, generated output: label them `example/reference/fixture/demo/legacy/generated`. An example is not a preferred pattern until it meets the rule in step 6.
8. Write `PATTERNS.md`: each pattern gets status, confidence, validated-at commit, references, the shape of the pattern, and what not to copy.
9. Propose only **deterministic** consistency rules (commands that exist, files that must exist, ast-grep enforceable invariants). Anything needing human judgment stays out of `consistency.yml`.
10. Run `aicontext check`. Fix drift until it passes, then `aicontext sync` if you touched curated-adjacent facts.
11. End with a short human summary: what was adopted, which references back each preferred pattern, what is still `TBD`, and the `check` result.

## Exploration budget (in order, stop when evidence suffices)

1. `scan --json`
2. root docs
3. `aicontext search` / ast-grep
4. CodeGraph if applicable
5. concrete reference files
6. broad exploration only if evidence is still missing

## Boundaries (never cross)

- Never modify Gentle-AI, Engram memory, SDD/RDD state, branches, or worktrees.
- Never install tools as a side effect; point at `aicontext tools plan` instead.
- Never rewrite the `generated` block of `PROJECT_STATE.md` by hand; `aicontext sync` owns it.
- Never auto-invoke this skill. It runs when the user asks (`/aicontext-adopt`) for a repo worth adopting once.
