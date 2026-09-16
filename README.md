# AiContext

Capa de inteligencia de repositorio: CLI determinista + skill semántico.
Spec de producto: `AICONTEXT_V0.1_IMPLEMENTATION_SPEC.md` (fuera del repo).

Estado: P0–P4 implementados (crate único `aicontext` + skill `aicontext-adopt`).

## Uso (P0)

```bash
cargo run -- init [--profile boilerplate] [--non-interactive] [--json]
cargo run -- scan [--json]
cargo run -- sync [--check] [--json]
cargo run -- status [--json]
cargo run -- check [--json]
```

Contratos machine-readable: `schemas/` (`scan-v1`, `check-v1`, `error-v1`).
Fixtures y tests: `fixtures/`, `tests/` (`cargo test`).

## Adapters de proyecto (P4)

`check` corre los adapters aplicables como evidencia (nunca bloquean):

- knip (JS/TS con `package.json`): `knip --reporter json`
- dependency-cruiser (JS/TS con config): `depcruise --output-type json`
- lychee (repos docs-heavy): `lychee --offline --format json`
- zizmor (con `.github/workflows`): `zizmor --offline --format=json`

Sin binario o config: nota informativa. `tools plan` recomienda cada
adapter donde aplica; knip y dependency-cruiser son project-local
(nunca se instalan global, AIContext no toca manifests).

## Contexto del repositorio

Antes de explorar el repo en profundidad, leer:

- `.engineering/PROJECT_STATE.md`
- `.engineering/PATTERNS.md`

Preferir `aicontext search` (futuro) y las referencias declaradas antes que el escaneo amplio.
Antes de declarar completa una implementación, correr: `aicontext check`.
