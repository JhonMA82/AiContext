# Instaladores remotos

Scripts para instalar, actualizar y desinstalar `aicontext` sin clonar el
repo. Todo el estado gestionado vive en el prefijo de usuario y solo se
toca estado owned (misma regla que `aicontext self uninstall --managed`):
nunca archivos ajenos, manifests del proyecto ni `.engineering/`.

## Requisitos

- `sh` POSIX + `curl` para los `.sh` (Linux/macOS).
- PowerShell 5.1+ para los `.ps1` (Windows).
- Sin `sudo` en ningún caso.

## Instalación remota

El binario queda en `<prefijo>/bin` (`~/.local/bin` por defecto): los
scripts pasan al instalador oficial el directorio base y este añade
`bin` él mismo.

Unix (última versión):

```sh
curl -LsSf https://raw.githubusercontent.com/JhonMA82/AiContext/v0.7.0/installers/install.sh | sh
```

Unix (versión fijada):

```sh
curl -LsSf https://raw.githubusercontent.com/JhonMA82/AiContext/v0.7.0/installers/install.sh | sh -s -- --version 0.4.0
```

Windows:

```powershell
irm https://raw.githubusercontent.com/JhonMA82/AiContext/v0.7.0/installers/install.ps1 | iex
```

Origen primario: el instalador oficial de cargo-dist publicado en GitHub
Releases (`aicontext-installer.sh/ps1`, con checksums por target). Si la
descarga falla y hay toolchain Rust, se usa
`cargo install --git https://github.com/JhonMA82/AiContext --locked`
como fallback (el crate no está publicado en crates.io;
`--no-cargo-fallback` lo prohíbe).

Flags comunes (`.sh`): `--version latest|0.4.0|v0.4.0`, `--prefix DIR`,
`--bin-dir DIR`, `--yes` (compat, la instalación nunca pregunta),
`--no-cargo-fallback`, `--help`. Variables: `AICONTEXT_PREFIX`,
`AICONTEXT_INSTALL_DIR`.

## Actualización remota

```sh
curl -LsSf https://raw.githubusercontent.com/JhonMA82/AiContext/v0.7.0/installers/update.sh | sh -s -- --check
curl -LsSf https://raw.githubusercontent.com/JhonMA82/AiContext/v0.7.0/installers/update.sh | sh -s -- --yes
```

- `--check` es solo lectura (delega en `aicontext self update --check`
  cuando el binario existe; nunca cambia nada).
- Sin `--yes` se rehúsa con exit 2 (igual que `self update`).
- Si el binario existe, primero intenta `aicontext self update --yes` y,
  si no completa (p. ej. instalación dist-managed), continúa con el
  instalador oficial remoto.

Windows: `update.ps1` con `-Check` / `-Yes` (`-Version`, `-InstallDir`,
`-NoCargoFallback`).

## Desinstalación remota
```sh
curl -LsSf https://raw.githubusercontent.com/JhonMA82/AiContext/v0.7.0/installers/uninstall.sh | sh -s -- --managed --yes
```

- Requiere `--yes` (exit 2 sin él).
- Sin `--managed` deja el prefijo de herramientas e informa cómo
  removerlo; con `--managed` solo remueve entradas del ownership registry
  (`managed-tools.json`) que vivan dentro del prefijo, más el registry.
- El binario solo se elimina dentro de prefijos owned (`~/.local`,
  `~/.cargo`, `--prefix`); cualquier otra ruta se reporta y se deja.
- El skill solo se toca con manifiesto de ownership
  (`.aicontext-managed.json`); sin manifiesto se deja intacto.

Windows: `uninstall.ps1 -Managed -Yes`.

## Problemas comunes

- **El instalador oficial avisa de comandos eclipsados (`shadowed`)** o
  `aicontext --version` sigue mostrando una versión vieja: hay otro
  binario antes en el `PATH` (p. ej. `~/.cargo/bin/aicontext`). Localízalos
  con `command -v -a aicontext`, elimina el obsoleto y recarga con
  `hash -r`.
- **Binario en `<prefijo>/bin/bin`** (instaladores previos a esta
  corrección): mueve el binario a `<prefijo>/bin` o reejecuta el
  instalador; `uninstall.sh --managed --yes` también limpia ese resto.

## Seguridad

- HTTPS con TLS 1.2+ y `curl --proto '=https'` en los `.sh`.
- Los binarios vienen de GitHub Releases con `.sha256` por artefacto
  (verificados por el instalador oficial); el fallback `cargo install`
  usa `--locked`.
- Revisa siempre el script antes de pipe a shell en máquinas sensibles.
