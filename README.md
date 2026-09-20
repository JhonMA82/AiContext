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

Contratos machine-readable: `schemas/` (`scan-v2`, `search-v1`,
`status-v1`, `check-v1`, `error-v1`).
`scan --json` emite `aicontext/scan/v2` (supera `scan-v1`, que queda como
contrato archivado) e incluye `subprojects`/`subprojects_source`.
Fixtures y tests: `fixtures/`, `tests/` (`cargo test`).

## Router de subproyectos (monorepos)

En un repo con ≥2 subproyectos detectados, `init` crea `AGENTS.md` si
falta, con dos bloques marcados: el de routing apunta a cada subproyecto
con su contexto de entrada y la tabla de stack, manifest, comandos y
propósito, y el de contexto mantiene el puntero a `.engineering/`. La
misma tabla de subproyectos entra en el bloque generado de
`PROJECT_STATE.md`, así que cambiar el `purpose` declarado en
`.engineering/subprojects.yml` es drift que `sync` resuelve. `sync`
refresca solo bloques existentes (nunca crea el archivo ni inserta
bloques faltantes) y con <2 subproyectos no se renderiza routing. Fuera
de los marcadores no se edita nada: los cambios manuales se preservan y
el puntero legacy sin marcadores se reporta sin reescribirse.

### Manifiesto de subproyectos

`init` siembra `.engineering/subprojects.yml` (`aicontext/subprojects/v1`,
schema en `schemas/subprojects-v1.json`) con cada path detectado como
`status: pending` y `purpose` vacío; el skill de adopción llena el
`purpose` y mueve entradas a `adopted`. El archivo existente **nunca** se
reescribe (ni `init` ni `sync` lo tocan) y `sync` proyecta el `purpose` en
la tabla de `PROJECT_STATE.md`. `check` valida la forma estricta (claves
desconocidas y estados fuera del enum cerrado fallan) y el ciclo de vida:
con ≥2 subproyectos el bloque de routing debe estar presente, las entradas
deben conciliar con la detección en ambas direcciones (faltantes y rancias
fallan) y un `adopted` sin contexto anidado real
(`<path>/.engineering/aicontext.toml`) falla cerrado; `pending` es
advisory. Sin manifiesto no hay nada que validar y un schema futuro se
omite sin bloquear.

### Contextos anidados (`init --recursive`)

```bash
aicontext init --recursive
```

Por cada subproyecto detectado (ordenado por path) crea su propio
`.engineering/` (manifiesto con sección `[subproject]` que apunta al root
padre, estado generado, placeholder de patrones, stub de consistencia) y el
puntero de contexto marcado en su propio `AGENTS.md` — solo si ese archivo
ya existe, nunca se crea. Todo es seed-once: las repeticiones solo
refrescan contenido generado. Además, cada comando resuelve su root al
manifiesto más cercano hacia arriba (`resolve_project_root()`): dentro de
un subproyecto con contexto anidado se opera sobre él, si no sobre el git
toplevel como antes.

### Adopción con scope (skill + `doctor`)

El skill `aicontext-adopt` adopta un monorepo en una corrida sobre todos
los detectados, en orden fijo por path y por slices reanudables: un commit
por subproyecto, presupuesto por scope con salida legal a `pending` y
summary final (adoptados con su propósito, pendientes con lo que les
falta). `doctor` suma el diagnóstico `skill`: compara el `SKILL.md`
instalado con el embebido en el binario (ausente, al día, stale con ambas
versiones, unmanaged) y siempre es advisory — el skew nunca falla.

### Búsqueda con scope

```bash
aicontext search "<query>" --in apps/web [--json]
aicontext search "<query>" --subproject apps/web [--json]
```

`--in <path>` restringe tgrep/rg/git grep y ast-grep a un path
repo-relativo existente (`.` o ausente = repo completo), filtra los hits
de knowledge por prefijo y, con `--impact`, hace correr CodeGraph desde
el directorio del scope: si el grafo no puede servir ese directorio
degrada a resultados de texto con el mismo scope. `--subproject <path>`
es `--in` con validación: el path debe ser uno de los subproyectos
detectados (si no, error con la lista de candidatos) y la salida es
idéntica a `--in` más el campo `subproject`. La salida JSON
(`aicontext/search/v1`) suma `scope` y `subproject` opcionales.

### Estado de adopción

`status` suma una sección `Subprojects: detected: N | adopted: A |
pending: P` (visible cuando hay subproyectos detectados o existe
`.engineering/subprojects.yml`) y el objeto `subprojects`
(`total`/`adopted`/`pending`) en `status --json` (`aicontext/status/v1`).
Solo cuentan las entradas del manifiesto cuyo path coincide con un
subproyecto detectado; sin manifiesto o con un manifiesto ilegible los
conteos son 0/0, sin error.

## Gate de consistencia (`check`)

`.engineering/consistency.yml` son declaraciones que `check` verifica
contra la realidad (no solo schema):

- `commands.documented`: debe coincidir exactamente con los scripts
  detectados. Un script sin declarar falla (hay que declararlo) y una
  entrada sin script falla (hay que removerla); si un doc (`README.md`,
  `AGENTS.md`, `CHANGELOG.md`, `docs/`) todavía menciona el comando
  eliminado, el finding indica el archivo. `init` siembra la lista desde
  el scan.
- `version.projections`: cada archivo listado debe contener literalmente
  la versión resuelta desde `version.source`.
- `protected`: cada path listado debe existir.
- `checks.ast_grep.rules`: cada regla `*.yml` del directorio se ejecuta
  con `ast-grep scan --rule --json`; cualquier match es violación. Sin
  reglas no hay nada que exigir; con reglas pero sin binario, falla con
  remediation (`aicontext tools install ast-grep`).

## Adapters de proyecto (P4)

`check` corre los adapters aplicables. Por defecto son evidencia
(`advisory`: se muestran, nunca bloquean):

- knip (JS/TS con `package.json`): `knip --reporter json`
- dependency-cruiser (JS/TS con config): `depcruise --output-type json`
- lychee (repos docs-heavy): `lychee --offline --format json`
- zizmor (con `.github/workflows`): `zizmor --offline --format=json`

Sin binario o config: nota informativa. `tools plan` recomienda cada
adapter donde aplica; knip y dependency-cruiser son project-local
(nunca se instalan global, AIContext no toca manifests).

### Policies (`required` vs `advisory`)

En `.engineering/consistency.yml` un adapter puede declararse `required`
para que sus findings bloqueen `check`:

```yaml
adapters:
  knip:
    policy: required
  lychee:
    policy: advisory
```

- `advisory` (default, tabla ausente incluida): evidencia que siempre pasa.
- `required`: pasa solo si el adapter se ejecutó con éxito y reportó cero
  issues; issues, error de ejecución o binario/config faltante fallan el
  finding (exit 1) con remediation exacta.
- Nombres de adapter desconocidos o valores distintos de
  `required`/`advisory` fallan cerrado (protección contra typos).

## Self update / uninstall

AIContext también administra su propia instalación:

```bash
aicontext self update --check   # compara versión actual vs última publicada, sin cambiar nada
cargo run -- self update --yes # actualiza (requiere --yes)
aicontext self uninstall --managed --yes  # remueve solo estado owned (requiere --yes)
```

- `self update`: descubre la última versión vía crates.io API con timeout
  acotado (offline degrada a mensaje claro, nunca cuelga); `--check` es
  siempre read-only. La actualización usa el instalador oficial si hay
  evidencia de install cargo-dist, si no `cargo install aicontext --locked`.
- `self uninstall`: remueve solo estado owned — el binario (únicamente
  dentro de un prefijo managed o cargo bin), herramientas registradas en
  el ownership registry y skills con manifiesto de ownership. Sin `--yes`
  se niega con remediation. Nunca toca archivos ajenos, manifests del
  proyecto ni `.engineering/`.

## Contexto del repositorio

Antes de explorar el repo en profundidad, leer:

- `.engineering/PROJECT_STATE.md`
- `.engineering/PATTERNS.md`

Preferir `aicontext search` (modos text/structure/impact) y las referencias declaradas antes que el escaneo amplio.
Antes de declarar completa una implementación, correr: `aicontext check`.

## Release

Releases con `cargo-dist` (tags SemVer, checksums, instaladores shell/powershell,
binarios para Linux/macOS/Windows):

```bash
cargo dist plan    # qué se va a construir
cargo dist build   # artefactos locales en target/distrib/
```

Publicar: `git tag vX.Y.Z && git push origin vX.Y.Z` — CI construye
los 5 targets y publica el GitHub Release con checksums.
Instalación:

```bash
curl -LsSf https://github.com/JhonMA82/AiContext/releases/latest/download/aicontext-installer.sh | sh
```
