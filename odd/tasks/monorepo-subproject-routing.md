# Feature: monorepo subproject routing (`aicontext init` en raíz)

Rama: `feat/monorepo-subproject-routing` (creada desde `master` @ bfb88d2)
Locator: `odd/tasks/monorepo-subproject-routing.md`
Espejo Engram: topic `odd/monorepo-subproject-routing/tasks` — obs 996 (v3) guardada; el delta posterior
(estrategia de cadena + contrato de WU2 + cierre de WU1) se guardó al volver con el mismo `topic_key`.

## Objetivo

Que `aicontext init` ejecutado en la raíz de un monorepo detecte los subproyectos y
genere un router en `AGENTS.md` que mande al agente al contexto de **un** subproyecto,
en vez de explorar el árbol completo.

## Problema

Hoy `init` escribe `.engineering/` en el git toplevel y no inspecciona la estructura
interna (`src/state.rs:203`). Un layout `apps/desktop|web|mobile` + `services/api`,
cada uno con su `AGENTS.md`, no produce ninguna señal: el agente que aterriza en la
raíz no sabe que hay cuatro proyectos ni cuál le toca.

## Por qué ahora

Es el caso de uso que justifica el producto: evitar exploración repetida. Un monorepo
sin router paga la exploración completa en cada sesión y por cada agente.

## Decisiones tomadas (usuario, cerradas)

| # | Decisión |
| --- | --- |
| 1 | `AGENTS.md` raíz: bloque marcado; `init` lo crea si falta y hay ≥2 subproyectos |
| 2 | Alcance: routing + `init --recursive` (contexto anidado por subproyecto) |
| 3 | Detección: workspaces declarados primero; contenedores acotados solo si no hay declarados |
| 4 | Contrato: `scan/v2` (nuevo schema versionado) |
| 5 | Adopción del skill: todos los subproyectos detectados en una corrida |
| 6 | Patrones: `PATTERNS.md` por subproyecto; la raíz solo transversales |

## Restricciones duras (trampas verificadas en el código)

1. `state.rs:161` `freshness_key()` compara todo el bloque generado menos la línea del
   commit: **toda** salida nueva tiene que ser determinista (orden estable por path, sin
   timestamps, sin paths absolutos, markdown estable para el linter).
2. Los comandos de los hijos **no** pueden entrar en el `commands` de la raíz:
   `check` exige coincidencia exacta (`src/check.rs:97`).
3. `scan::current_dir_root()` es `git rev-parse --show-toplevel`: el contexto anidado
   necesita `resolve_project_root()` (WU5) y una sección `[subproject]` en el manifest.
4. `AGENTS.md` es archivo del usuario: se escribe **solo** entre marcadores, nunca una
   reescritura ciega (`src/agent.rs:167`).
5. Sin dependencias nuevas: pins exactos (`Cargo.toml`). Se usan `serde_json`, `toml`,
   `serde_yaml` y `ignore`, ya presentes.
6. Tests offline (regla del repo). Baseline verificado: 81 tests, 13 suites, `cargo fmt` limpio.

## Fuera de alcance (WU1)

Router renderizado, `subprojects.yml`, gates de `check`, `--recursive`, `search --in`,
reescritura del skill. WU1 es **read-only sobre archivos del usuario**.

## Tareas

Un WU = un work unit = un commit Conventional Commit, con tests y docs junto al comportamiento.

### WU1 — Detección de subproyectos + contrato `scan/v2` (delegado)

- `detect_subprojects(root) -> (Vec<Subproject>, SubprojectsSource)` en `src/scan.rs`.
- Fuentes declaradas: `package.json` (`workspaces` array u `object.packages`),
  `pnpm-workspace.yaml` (`packages:`), `Cargo.toml` (`[workspace] members`, hoy reducido
  al centinela `"[workspace]"`), `go.work` (`use`).
- Expansión de globs: path exacto, o último segmento `*` (`read_dir`); `**`, `{}`, `!`
  y `*` no final van a `unresolved` (nunca se adivinan).
- Si hay declarados utilizables ⇒ `mode = "declared"` y **no** se escanean contenedores.
  Si no ⇒ `mode = "containers"` sobre `apps|services|packages|libs|modules` (profundidad 1).
- `[subprojects] extra` en `.engineering/aicontext.toml` (nueva sección opcional, default
  vacío, retrocompatible) para el caso real "monorepo JS declarado + servicio Rust no declarado".
- Candidato califica con manifest (`package.json`, `Cargo.toml`, `pyproject.toml`, `go.mod`)
  o `AGENTS.md` (`kind = "manifest" | "agents-marker"`). `is_excluded`, symlinks y dot-dirs fuera.
- Campo por subproyecto: `path`, `name`, `kind`, `manifest`, `manager`, `commands` (propios),
  `agents_md`, `initialized` (`<path>/.engineering/aicontext.toml` existe), `complexity` (propia).
- `subprojects_source`: `mode`, `declared`, `unresolved`, `containers`.
- `pub const SCAN_SCHEMA = "aicontext/scan/v2"` en `scan.rs`, usado también por
  `src/tools.rs:241` (hoy duplica el literal). `schemas/scan-v2.json` nuevo;
  `schemas/scan-v1.json` intacto como contrato superado.
- `detect_workspaces("cargo")` devuelve los `members` reales (el centinela queda solo como
  fallback), para que `complexity.workspaces` cuente de verdad.
- `complexity.reason` agrega `; N subprojects` cuando N ≥ 2.
- `commands` de la raíz **no** cambia.

Aceptación: `fixtures/monorepo-full` (layout exacto del usuario: `apps/desktop`, `apps/web`,
`apps/mobile` + `services/api`, con `AGENTS.md` en cada uno) detecta 4 ordenados por path; `fixtures/polyglot-monorepo`
parsea `members` de cargo; `fixtures/apps-no-projects` no detecta nada; `node-single`/`rust-single`
no detectan nada; `node_modules` creado en runtime dentro de un contenedor no se detecta;
los comandos de los hijos no aparecen en el `commands` raíz.

Checks: `cargo fmt --check`, `cargo test --test subprojects`, `cargo test`.

### WU2 — Bloques marcados en `AGENTS.md` + tabla en `PROJECT_STATE.md` (delegado)

Contrato del renderer (decidido; no re-decidir en la delegación):

- Constantes nuevas junto a `GEN_START`/`GEN_END` (`src/state.rs:12`): `ROUTING_START/END`
  (`<!-- aicontext:routing:start|end -->`) y `CONTEXT_START/END` (`<!-- aicontext:context:start|end -->`).
- Funciones: `render_subproject_rows(&[Subproject], Option<&SubprojectsFile>) -> String` (única fuente de
  filas, dos call sites), `render_routing_block(...)`, `render_context_block()`, `extract_block(text, start, end)`,
  `upsert_block(existing, rendered, start, end) -> (String, Placement)`.
- Placement determinista: si existen los marcadores ⇒ reemplazo en el lugar (byte-idéntico si no cambió);
  si hay un heading de nivel 1 (`#`) ⇒ insertar inmediatamente después del primero; si no ⇒ routing al tope, puntero al final.
  Nunca se toca un byte fuera de la región marcada. Escribir solo si el resultado difiere del actual.
- `init` crea `AGENTS.md` **solo** con ≥2 subproyectos y archivo ausente (contenido = bloque de contexto +
  bloque de routing, nada más). `sync` nunca crea: refresca lo que existe y reporta si falta el bloque.
  Con <2 subproyectos no se renderiza routing y un bloque existente queda intacto (lo juzga WU4).
- Fila de tabla: 7 columnas, escapado de `|` ⇒ `\|`, saltos de línea/espacios colapsados, recorte a 120 chars,
  vacío ⇒ `—`, orden por path, paths relativos. Separador `| --- |` (el estilo `|---|---|` lo marca MD060).
  Línea en blanco antes y después de la tabla y antes de la lista `Rules:` (MD032): el markdown generado
  tiene que ser lint-estable porque `freshness_key` compara texto.
- La tabla entra también en el bloque generado de `PROJECT_STATE.md` (mismo `render_subproject_rows`).
  Consecuencia buscada: `purpose` es un hecho proyectado, así que cambiarlo exige `sync`.
- El bloque de `AGENTS.md` **no** está en `freshness_key` (vive en otro archivo): su drift lo detecta WU4
  comparando el bloque real contra `render_routing_block(...)`.
- Migración del puntero viejo sin marcadores (`src/agent.rs:8`): si el texto existe, **no** se reescribe;
  se reporta y el bloque nuevo va a la posición determinista. Advisory en `check` en WU4.

Salida esperada con los datos del fixture `monorepo-full` (preview, no observada todavía):

```md
<!-- aicontext:routing:start -->
## Subprojects — read exactly one

This repository has 4 subprojects. Do not explore the tree broadly.

| Path | Stack | Manifest | Entry context | Commands | Purpose |
| --- | --- | --- | --- | --- | --- |
| apps/desktop | npm | apps/desktop/package.json | apps/desktop/AGENTS.md | dev | — |
| apps/mobile | npm | apps/mobile/package.json | apps/mobile/AGENTS.md | — | — |
| apps/web | npm | apps/web/package.json | apps/web/AGENTS.md | dev, test | — |
| services/api | cargo | services/api/Cargo.toml | services/api/AGENTS.md | test | — |

Rules:

- Identify the subproject that owns your task's files and read only its entry context.
- A file belongs to exactly one subproject; that subproject's `AGENTS.md` governs it.
- Cross-cutting work: this block plus `.engineering/PROJECT_STATE.md`, nothing else.

> Generated by `aicontext sync`. Do not edit inside this block.
<!-- aicontext:routing:end -->
```

Tests de WU2: idempotencia byte a byte (dos corridas), inserción tras H1, append sin marcadores, bytes de
usuario intactos fuera de los marcadores, creación solo con ≥2 y no con 1, escapado de `|`/newline/120 chars,
forma lint-estable (blancos + `| --- |`), drift de `freshness_key` tras cambiar `purpose`, puntero legacy sin
duplicar ni reescribir. Dep: WU1.

### WU3 — `subprojects.yml` (seed, propósito, validación) (delegado)

`init` siembra paths detectados + `status: pending`; el skill llena `purpose`/`status`;
`sync` nunca reescribe; `check` valida clave desconocida y enum cerrado. Dep: WU1, WU2.

### WU4 — Gates de `check` (delegado)

Bloque presente con ≥2 subproyectos, entradas faltantes/rancias (reusar `stale_mentions`),
`status: adopted` sin `<path>/.engineering` real (falla cerrado), advisory por pending. Dep: WU2, WU3.

### WU5 — `init --recursive` + `resolve_project_root()` (delegado)

Contexto anidado por subproyecto + puntero por subproyecto. Dep: WU2.

### WU6 — `search --in/--subproject` + `status` con adopted/pending (delegado)

Hoy `search` es siempre repo-wide (`src/search.rs:78`, `:116`, `:173`, `:237`). Dep: WU1.

### WU7 — Skill `aicontext-adopt` con scope + `doctor` de skew (delegado)

Corrida única sobre todos los detectados, orden fijo por path, presupuesto por scope
con salida legal a `pending`, escritura por slice (reanudable), un commit por subproyecto,
summary final. `doctor` detecta skill instalado viejo vs binario (`src/agent.rs:95`). Dep: WU3, WU4, WU6.

## Rutas y disparadores

| Tarea | Ruta | Disparador |
| --- | --- | --- |
| WU1 | delegado (`gentle-ai-worker`) | writer trigger: 2+ archivos no triviales |
| WU2–WU7 | delegado (`gentle-ai-worker`) | mismo criterio; WU2 puede ser parcialmente inline |
| Verificación | `gentle-ai-verify` según tier de `assess` | verification rule |

## TDD

Modo efectivo: **desconocido** — no hay configuración de proyecto (`.pi/` inexistente, sin
mención de TDD en `.engineering/`). Se aplica la convención del repo: tests junto al
comportamiento en el mismo work unit. Runner exacto: `cargo test`.

## Delivery

Forecast de líneas autoradas: WU1 ~500, WU2 ~250, WU3 ~250, WU4 ~200, WU5 ~300, WU6 ~200,
WU7 ~250 ⇒ ~1950, muy por encima de 400 ⇒ estrategia por defecto `ask-on-risk`: se preguntó una
vez por la estrategia de cadena antes del primer commit y el usuario eligió **`feature-branch-chain`**
(cacheada). Los 3 PRs apuntan a `feat/monorepo-subproject-routing`, se mergean ahí en orden y al
final va un único PR de la rama a `master`. Slices: PR1 = WU1+WU2+WU6, PR2 = WU3+WU4+WU5, PR3 = WU7.

## Progreso

- [x] WU1 detección + `scan/v2` — commit `b5f7347` (`feat: detect monorepo subprojects with scan/v2 contract`)
- [x] WU2 router en `AGENTS.md` — commit `f322801` (`feat: render routing blocks in AGENTS.md and subproject table in PROJECT_STATE`)
- [x] WU3 `subprojects.yml` — commit pendiente (`feat: seed subprojects manifest and validate its shape`)
- [x] WU4 gates de `check` — commit pendiente (`feat: gate subproject routing lifecycle in check`)
- [ ] WU5 `init --recursive`
- [x] WU6 `search` con scope — commit `ca8369a` (`feat: add scoped search and subproject adoption counts`)
- [ ] WU7 skill con scope

## Evidencia de verificación

- Baseline pre-WU1: `cargo test` ⇒ 81 passed (13 suites, 182.90s); `cargo fmt --check` limpio.
- WU1 (`b5f7347`): `cargo test` ⇒ 91 passed (14 suites, 166.56s) — los 10 tests de `tests/subprojects.rs`;
  `cargo fmt --check` limpio; `aicontext check` exit 0 (advisory STALE_PROJECT_STATE, se refresca en la
  chore sync post-cierre). Nota: en el worktree había restos de la rama TUI (`src/tui/`, `tests/tui.rs`,
  wiring en `main.rs`/`cli.rs`/`Cargo.toml`) que **no** entraron al commit; el TUI vive completo en
  `feat/tui-ratatui` (ea72ecd, RDD aprobado). El `.gitignore` ahora cubre `fixtures/**/target/` y
  `fixtures/**/Cargo.lock` (los artefactos de build del fixture cargo quedaron fuera del commit).
- WU2 (`f322801`): `cargo test` ⇒ 100 passed (15 suites, 195.02s) — +9 tests de `tests/routing.rs`
  (idempotencia byte a byte, inserción tras H1, append sin marcadores, bytes de usuario intactos,
  creación solo con ≥2, escape/clip de celdas, forma lint-estable, drift de `freshness_key` por
  `purpose`, puntero legacy reportado sin reescribir). `cargo fmt --check` limpio; verificación
  independiente (`gentle-ai-verify`) PASS sin hallazgos bloqueantes; review nativo
  `review-3c013172aa99e8d1` (tier medium, 1 lente reliability, 5 archivos, 902 líneas): **APPROVED**,
  ack quemado. 4 advisorys informacionales como follow-ups (nunca motivo de re-review):
  `R3-cell-escape-clip` (state.rs:72-76), `R3-check-gate` (state.rs:726), `R3-h1-fence`
  (state.rs:187-197, SUGGESTION), `R3-init-self-stale` (state.rs:646-651) — evaluar dónde aplican
  en WU3/WU4. Resolución del worker sobre el contrato: la preview canónica (6 columnas) gana sobre
  la prosa "7 columnas" (conteo viejo); `SubprojectsFile` minimal tolerante
  (`aicontext/subprojects/v1` placeholder, solo lectura — WU3 adopta/valida).

- WU6 (`ca8369a`): `cargo test` ⇒ 110 passed (16 suites) — los 10 tests de `tests/search_scope.rs`
  (scope de texto/estructura, equivalencia `--subproject`/`--in`, errores con candidatos,
  determinismo byte a byte, degradación de `--impact`, cwd de CodeGraph, conteos de `status`,
  validación contra schemas); `cargo fmt --check` limpio; `aicontext check` exit 0 (mismo
  advisory STALE_PROJECT_STATE, se refresca en la chore sync post-cierre). `schemas/search-v1.json`
  suma `scope`/`subproject` opcionales; `schemas/status-v1.json` nuevo (`aicontext/status/v1`).

- WU3 (seed + validación): `cargo test` ⇒ 118 passed (17 suites) — los 8 tests de
  `tests/subprojects_manifest.rs` (seed byte-exacto, no-reescritura en `init`/`sync`,
  proyección del `purpose`, rechazos de estado/claves/YAML inválido, validación contra
  `schemas/subprojects-v1.json`); `cargo fmt --check` limpio. Lección: el primer `init`
  en un monorepo reporta stale por diseño (crear `AGENTS.md` añade un doc escaneado);
  los tests hacen `init` + `sync` antes de afirmar.

- WU4 (gates de `check`): `cargo test` ⇒ 127 passed (18 suites) — los 9 tests de
  `tests/subprojects_gates.rs` (presencia del bloque, conciliación en ambas direcciones,
  adopción hueca vs grounded, pending advisory, repo simple sin gates, manifiesto ausente);
  `cargo fmt --check` limpio. Pre-verificado: ningún test existente de `check` usa
  monorepos con init (todos usan fixtures simples), sin regresiones.

## Próximo paso

WU4 cerrado: siguiente **WU5** (`init --recursive` + `resolve_project_root()`) para completar
el PR2. Después PR3 = WU7. Los 3 PRs apuntan a `feat/monorepo-subproject-routing`,
se mergean en orden y al final un único PR a `master`.
