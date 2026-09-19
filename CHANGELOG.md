# Changelog

Formato basado en Keep a Changelog. Versiones: SemVer.

## [Unreleased]

### Agregado

- Detección determinista de subproyectos en `scan` + contrato
  `aicontext/scan/v2`: `subprojects` (path, name, kind, manifest, manager,
  commands propios, agents_md, initialized, complexity propia) y
  `subprojects_source` (mode, declared, unresolved, containers). Fuentes
  declaradas: `package.json` workspaces, `pnpm-workspace.yaml`, `Cargo.toml`
  `[workspace] members` (reemplaza el centinela `"[workspace]"`, que queda
  solo como fallback), `go.work` `use` y `[subprojects] extra` en
  `aicontext.toml`. Globs no resolubles (`**`, `{}`, `!`, `*` no final) se
  reportan en `unresolved`, nunca se adivinan. Si hay declarados utilizables
  el modo es `declared` y no se escanean contenedores; si no, `containers`
  sobre `apps|services|packages|libs|modules` a profundidad 1; si nada
  califica, `none`. Los fixtures nuevos (`monorepo-full`, `polyglot-monorepo`,
  `apps-no-projects`) y `tests/subprojects.rs` (10 tests) cubren el layout,
  el parseo de `members`, el fallback por contenedores, el marcador
  `AGENTS.md`, la exclusión de `node_modules`, el no-derrame de comandos
  hijos al `commands` raíz y el contrato v2. `scan/v1` queda superado como
  contrato archivado (`schemas/scan-v1.json` intacto).

- Gate de consistencia real en `check` (0.1.1 P0): `commands.documented`
  debe coincidir exactamente con los scripts detectados (faltantes y
  sobrantes fallan; las menciones rancias en docs se reportan con archivo),
  `version.projections` verifica que cada archivo declarado contenga la
  versión resuelta, `protected` exige que cada path exista, y cada regla de
  `.engineering/rules/ast-grep` se ejecuta (`ast-grep scan --rule --json`;
  un match es violación; reglas declaradas sin binario fallan con
  remediation). `init` siembra `documented` desde el scan para que un repo
  fresco pase y el drift posterior falle. 7 regression tests nuevos
  (`tests/check_consistency.rs`).
- Policies de adapters en `check` (punto 2): `adapters.<tool>.policy` en
  `consistency.yml` (`required` bloquea con issues/error/binario faltante,
  `advisory`/ausente es solo evidencia; nombres o valores desconocidos
  fallan cerrado). 8 tests nuevos (`tests/adapters.rs`: 13 en total).
- `self update` / `self uninstall`: `update --check` compara contra
  crates.io con timeout acotado (offline degrada sin colgar); el update
  usa el instalador oficial o `cargo install`; `uninstall --managed --yes`
  remueve solo estado owned (binario en prefijo managed, registry,
  skills con manifiesto) y nunca archivos ajenos. 8 tests nuevos
  (`tests/self_update.rs`, sin red).
- Censo determinístico de tests en el bloque generado de PROJECT_STATE.md:
  `Test functions: N (src: U, tests: I)` contado desde `#[test]` en `.rs`
  trackeados (sin tocar el schema `scan/v1`); el curated ya no mantiene
  cifras volátiles a mano (`40 tests green` eliminado). 2 tests nuevos
  (`tests/test_counts.rs`).
- README: `aicontext search (futuro)` corregido a presente
  (modos text/structure/impact).

## [0.1.0] - 2026-09-16

Primera release: CLI determinista + skill semántico (`aicontext-adopt`).

### Agregado

- Core P0: `init`, `scan`, `sync`, `status`, `check`, `doctor`, `search`
  (single-crate `aicontext`, `schemas/` versionados, `fixtures/` + tests).
- Skill `aicontext-adopt` embebido en el binario (`agent install/uninstall pi`).
- Tool manager P2: `tools plan/status/install/update/uninstall` con registro
  de ownership (solo prefijo de usuario, con confirmación previa).
- `completion` para bash/zsh/fish/powershell.
- Búsqueda por capas: tgrep (literal) -> rg -> git grep, `ast-grep`
  estructural, impacto vía CodeGraph cuando hay índice.
- Adapters de proyecto P4 como evidencia en `check` (nunca bloquean):
  knip, dependency-cruiser, lychee, zizmor.
- Comandos externos con deadline (ningún comando cuelga por un binario wedged).
- `check` con envelope de error y remediation exacta; exit codes por spec §39.

### Corregido

- `search --text` es literal en los tres backends (fixed-strings).
- `doctor` degrada el "No index found" de tgrep a warning de índice corrupto.
- `freshness` compara hechos escaneados, no el puntero del commit de sync.
