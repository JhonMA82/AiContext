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
