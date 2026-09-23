# Patterns

> Adoption follows the `aicontext-adopt` skill. Status values: preferred | observed | legacy | exception | protected.

## Single-crate binary with internal modules

Status: preferred
Confidence: high
Validated at: 62b85b3

Reference:

- src/main.rs
- src/cli.rs
- src/scan.rs
- src/state.rs
- src/check.rs
- src/tools.rs
- src/tools_install.rs
- src/search.rs
- src/doctor.rs
- src/agent.rs
- src/config.rs
- src/output.rs

Pattern:
CLI parse -> dispatch per command module -> shared helpers (config, output, tools).
Conceptual boundaries live in modules, not in workspace crates.

## Versioned JSON contracts

Status: protected
Confidence: high
Validated at: 62b85b3

Reference:

- schemas/
- src/scan.rs
- src/check.rs
- src/tools.rs
- src/tools_install.rs
- src/search.rs
- src/doctor.rs
- src/agent.rs

Pattern:
every machine-readable output has a versioned schema id (`aicontext/<name>/v1`).
Schemas are public contracts: never change them silently, migrate explicitly.

## Exact pinned dependencies

Status: preferred
Confidence: high
Validated at: 62b85b3

Reference:

- Cargo.toml
- Cargo.lock

Pattern:
exact versions (`=x.y.z`, no ranges) and a committed lockfile.

## Integration tests per command with isolated environments

Status: preferred
Confidence: high
Validated at: 62b85b3

Reference:

- tests/

Pattern:
one integration file per command area; each test builds a unique temp dir
(atomic sequence + thread id) so parallel runs never share state; global
commands (agent, tools install) override HOME per invocation; no network in tests.

## Error envelope with remediation

Status: preferred
Confidence: high
Validated at: 62b85b3

Reference:

- src/output.rs

Pattern:
failures return `aicontext/error/v1` with code, message and the exact
remediation command; exit codes follow the fixed contract (0/1/2/3/4/5).

## Ownership registry for managed state

Status: preferred
Confidence: high
Validated at: 62b85b3

Reference:

- src/tools_install.rs
- src/agent.rs

Pattern:
everything the tool owns is recorded (registry / ownership manifest);
uninstall and update only touch owned entries; foreign files and
user-owned files are reported, never modified blindly.

## Positive conditions over boolean negation

Status: exception
Confidence: medium
Validated at: 62b85b3

Reference:

- src/doctor.rs
- src/search.rs
- src/tools_install.rs

Pattern:
prefer `if present { ... } else { ... }` over `if !present { ... }`.
Reason: the repo linter reports false-positive E0600 errors on valid `!bool`
expressions; `cargo check` stays the real authority for correctness.

## One work unit, one Conventional Commit

Status: preferred
Confidence: high
Validated at: 62b85b3

Reference:

- git log

Pattern:
each reviewable unit ships with its tests and docs in a single commit;
formatting-only changes go in separate `chore` commits.

## Context escalation over provider coupling

Status: preferred
Confidence: high
Validated at: c2d5aa9

Reference:

- src/search.rs
- src/tools.rs
- src/config.rs
- README.md
- skills/aicontext-adopt/SKILL.md

Pattern:
consumer decides whether more context is required;
AiContext owns deterministic backend selection;
graph providers are replaceable;
successful local/simple tasks bypass graph tooling.

## Do not copy

- There is no legacy code yet (greenfield); nothing is marked legacy.
- `.engineering/` generated blocks are owned by `aicontext sync`; curated
  sections are edited by agents with evidence, never overwritten by commands.
