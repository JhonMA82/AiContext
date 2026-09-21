# Changelog

Formato basado en Keep a Changelog. Versiones: SemVer.

## [Unreleased]

### Corregido

- `sync` converge en una sola pasada tras cambios de topología: refresca
  el routing de `AGENTS.md` y re-escanea antes de escribir el bloque
  generado (el LOC de Markdown cambiaba bajo sus pies y dejaba el árbol
  stale hasta un segundo `sync`). `tests/routing.rs` lo cubre con
  `routing_row_addition_converges_in_one_sync`.

## [0.3.0] - 2026-09-21

### Agregado

- Compatibilidad Engineering Platform (sin dependencia runtime):
  detección determinista de proyectos Engineering-managed
  (`.engineering/project.json` + `.engineering/project-map.json` con
  `schema_version` conocido; sin hardcodear recetas), precedencia del mapa
  declarado en `scan` (`mode: engineering`,
  `kind: engineering-surface`, objeto `engineering`), referencias compactas
  en `PROJECT_STATE.md`, finding `engineering` en `check` (drift
  filesystem vs declaración), `origin`/`engineering` en `status` y
  diagnóstico advisory en `doctor`. Standalone sigue first-class y sin
  cambios de comportamiento. Skill `aicontext-adopt` con sección
  Engineering-first. Tabla de ownership de `.engineering/` en `README.md`.
  `tests/engineering.rs` cubre los 13 escenarios cross-repo con fixtures
  autocontenidas.

## [0.2.0] - 2026-09-20

### Agregado

- Skill con scope y skew en `doctor` (WU7): `aicontext-adopt` documenta la
  corrida única sobre todos los detectados (orden fijo por path, slices
  reanudables, presupuesto por scope con salida legal a `pending`, un commit
  por subproyecto, summary final). `doctor` compara el `SKILL.md` instalado
  con el embebido (ausente/al día/stale/unmanaged, siempre advisory).
  `tests/adopt_skill.rs` (6 tests) cubre el contenido instalado y los cuatro
  estados del diagnóstico sin que el skew falle nunca.

- `init --recursive` y resolución anidada: por cada subproyecto detectado
  crea su `.engineering/` (manifiesto con `[subproject]` y puntero `parent`
  al root, estado generado, patrones, stub de consistencia; fuente de
  versión detectada para que cada contexto pase su propio `check`) y el
  puntero de contexto en su `AGENTS.md` existente (nunca creado). Todo
  seed-once y byte-idempotente. `resolve_project_root()` sustituye al git
  toplevel directo: el manifiesto más cercano hacia arriba gana.
  `tests/recursive_init.rs` (9 tests) cubre el seed con punteros, la
  idempotencia, el puntero-solo-contexto, la no-creación de `AGENTS.md`,
  la resolución anidada, el `check` por contexto, la adopción grounded y
  el `init` desde un subdirectorio. Limitación conocida: el scan anidado
  aún cuenta archivos repo-wide (el stub se siembra del mismo reporte, así
  que cada contexto converge en su `check`).

- Manifiesto de subproyectos (`aicontext/subprojects/v1`, schema nuevo en
  `schemas/subprojects-v1.json`): `init` siembra `.engineering/subprojects.yml`
  con cada path detectado como `status: pending` (nunca reescribe un archivo
  existente y `sync` tampoco lo toca; el skill llena `purpose`/`status` y
  `sync` proyecta el `purpose` en la tabla de `PROJECT_STATE.md`). `check`
  suma el gate `subprojects`: claves desconocidas y estados fuera del enum
  cerrado (`pending`/`adopted`) fallan con los ofensores exactos; sin
  manifiesto el gate pasa y un schema futuro se omite sin bloquear.
  `check` suma el gate `subprojects routing`: con ≥2 subproyectos el bloque
  de routing debe estar presente, las entradas concilian con la detección
  en ambas direcciones (faltantes y rancias fallan, las rancias listan los
  docs que aún las mencionan) y un `adopted` sin `<path>/.engineering`
  real falla cerrado; `pending` es advisory. `tests/subprojects_gates.rs`
  (9 tests) cubre presencia del bloque, ambas direcciones de la
  conciliación, adopción hueca vs grounded, pending advisory, repo simple
  sin gates y manifiesto ausente.
  `tests/subprojects_manifest.rs` (8 tests) cubre el seed byte-exacto, la
  no-reescritura en `init`/`sync`, la proyección del `purpose`, los rechazos
  (estado, claves, YAML inválido) y la validación contra el schema.

- Búsqueda con scope en monorepos: `search --in <path>` restringe todos los
  backends (tgrep/rg/git grep y ast-grep) a un path repo-relativo existente,
  filtra los hits de knowledge por prefijo y, en `--impact`, intenta la
  consulta de CodeGraph desde el directorio del scope degradando a resultados
  de texto con scope cuando el grafo no puede servir ese directorio. La salida
  `aicontext/search/v1` suma `scope` y `subproject` opcionales (presentes solo
  con la flag) y `--subproject <path>` es `--in` validado contra los
  subproyectos detectados: un path desconocido falla con exit 5 y lista los
  candidatos. `status` reporta `detected/adopted/pending` contando solo
  entradas de `.engineering/subprojects.yml` que matchean un path detectado
  (manifiesto ausente o ilegible ⇒ 0/0 sin error) en la salida humana y en
  `aicontext/status/v1` (schema nuevo en `schemas/status-v1.json`).
  `tests/search_scope.rs` (10 tests) cubre el scope de texto/estructura, la
  equivalencia `--subproject`/`--in`, los errores con candidatos, el
  determinismo byte a byte, la degradación de `--impact`, el cwd de CodeGraph,
  los conteos de `status` y la validación contra los schemas.

- Router de subproyectos en el `AGENTS.md` raíz: con ≥2 subproyectos
  detectados, `init` crea el archivo si falta (bloque de contexto + bloque
  de routing con la tabla `Path/Stack/Manifest/Entry context/Commands/
  Purpose`, ordenada por path, celdas con `|` escapado, espacios/saltos de
  línea colapsados, vacío ⇒ `—` y recorte a 120 chars) y `sync` refresca
  solo los bloques existentes, nunca crea ni inserta. La misma tabla entra
  en el bloque generado de `PROJECT_STATE.md` (única función de render),
  así que cambiar el `purpose` en `.engineering/subprojects.yml` es drift
  que `sync` resuelve. Los bloques viven entre marcadores
  (`aicontext:routing:*`, `aicontext:context:*`): fuera de la región
  marcada no se toca un byte, el puntero legacy sin marcadores se reporta
  (nunca se reescribe ni se duplica) y con <2 subproyectos no se renderiza
  routing. `tests/routing.rs` (9 tests) cubre idempotencia byte a byte,
  inserción tras el primer H1, archivo sin heading, bytes de usuario
  intactos, gate de creación, escapado/recorte, forma estable para el
  linter, drift de `purpose` y migración del puntero legacy.

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
