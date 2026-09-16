# Project State

<!-- aicontext:generated:start -->
Last synchronized commit: fa29c19
Version: 0.1.0 (Cargo.toml)
Package manager: cargo (Cargo.lock)
Complexity: small
Source files: 48 | LOC: ~6825

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

- CLI: init, scan, sync, status, check, doctor, search, tools plan/status/install/update/uninstall, agent install/uninstall pi, completion
- Skill `aicontext-adopt` (embedded in the binary, installed per agent)
- Versioned JSON contracts in `schemas/`; error envelope with remediation
- 40 tests green (integration per command + unit); no network in tests

## Constraints

- Single-crate binary with internal modules (minimalist decision, user-approved)
- Exact pinned dependencies and committed Cargo.lock
- Managed installs only into the user prefix with prior confirmation; project manifests are never touched
- English for code/comments/identifiers/commits; Spanish for documentation prose
<!-- aicontext:curated:end -->
