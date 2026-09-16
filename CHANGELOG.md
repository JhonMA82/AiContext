# Changelog

Formato basado en Keep a Changelog. Versiones: SemVer.

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
