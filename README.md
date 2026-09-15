# AiContext

Capa de inteligencia de repositorio: CLI determinista + skill semántico.
Spec de producto: `AICONTEXT_V0.1_IMPLEMENTATION_SPEC.md` (fuera del repo).

Estado: P0 core en implementación (crate único `aicontext`).

## Uso (P0)

```bash
cargo run -- init [--profile boilerplate] [--non-interactive] [--json]
cargo run -- scan [--json]
cargo run -- sync [--check] [--json]
cargo run -- status [--json]
cargo run -- check [--json]
```

Contratos machine-readable: `schemas/` (`scan-v1`, `check-v1`, `error-v1`).
Fixtures y tests: `fixtures/`, `tests/p0.rs` (`cargo test`).

## Contexto del repositorio

Antes de explorar el repo en profundidad, leer:

- `.engineering/PROJECT_STATE.md`
- `.engineering/PATTERNS.md`

Preferir `aicontext search` (futuro) y las referencias declaradas antes que el escaneo amplio.
Antes de declarar completa una implementación, correr: `aicontext check`.
