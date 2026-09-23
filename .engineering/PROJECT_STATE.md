# Project State

<!-- aicontext:generated:start -->
Last synchronized commit: 9ea4f9c
Version: 0.6.1 (Cargo.toml)
Package manager: cargo (Cargo.lock)
Complexity: small
Source files: 94 | LOC: ~19044
Test functions: 200 (src: 13, tests: 187)

Important paths:

- README.md
- CHANGELOG.md
- AGENTS.md
<!-- aicontext:generated:end -->

<!-- aicontext:curated:start -->
## Purpose

Deterministic repository-intelligence layer: a global Rust CLI that discovers facts,
manages tooling, keeps verifiable project context and detects drift — plus a semantic
skill for agents. It replaces repetitive exploration, never the development workflow
(Gentle-AI), the memory (Engram) or the review framework (RDD).

## Current capabilities

- CLI: init, scan, sync, status, check, doctor, search, tools plan/status/install/update/uninstall, agent install/uninstall pi|opencode, completion
- Skill `aicontext-adopt` (embedded in the binary, installed per agent: pi, opencode)
- Versioned JSON contracts in `schemas/`; error envelope with remediation
- Integration tests per command plus unit tests; exact test-function counts are a scanned fact in the generated block

## Constraints

- Single-crate binary with internal modules (minimalist decision, user-approved)
- Exact pinned dependencies and committed Cargo.lock
- Managed installs only into the user prefix with prior confirmation; project manifests are never touched
- English for code/comments/identifiers/commits; Spanish for documentation prose
- Tests run fully offline (no network in tests)
<!-- aicontext:curated:end -->
