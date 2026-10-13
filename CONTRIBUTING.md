# Contributing

Issues and pull requests are welcome, in English or Russian. Code comments and commit messages are in English.

## Setup

You need Rust 1.88 or newer, `clang`, `make`, `gettext`, `pkg-config`, the PipeWire headers and, for the
script tests, `node`. KDE Plasma 6 with PipeWire to try it out.

```sh
./install.sh               # build and install into ~/.local, enable the script
./install.sh --uninstall   # remove
```

Running `./install.sh` again rebuilds, reinstalls, restarts the service and reloads the script.
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the parts fit together.

## Checks

```sh
make check          # service unit tests
make check-script   # KWin script logic against a fake KWin (needs node)
cargo clippy --manifest-path service/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path service/Cargo.toml --check
```

CI runs the same checks on every pull request.

Useful while testing:

```sh
journalctl --user -u muteonminimized -f        # service log: Muting / Unmuting
journalctl --user -f | grep -i kwin            # script output (console.log)
pw-dump                                        # PipeWire nodes and their properties
pw-cli enum-params <node> Props                # a node's Props
```

Calling the service with `busctl` is of limited use: `busctl` leaves the bus right away and the service then
unmutes everything. Call it from a temporary KWin script instead
(`qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript <file.js> <name>`, then `run`).

## Translations

All user-visible text lives in `po/`.

- **Changed or new text** in `main.js` (inside `tr("…")`) or `config.ui`: run `po/update.sh`. It refreshes
  `po/muteonminimize.pot` and merges it into every `po/*.po`; then translate the new or fuzzy entries.
- **A new language**, for example French:
  ```sh
  msginit --no-translator --locale=fr_FR.UTF-8 -i po/muteonminimize.pot -o po/fr.po
  ```
  Translate `po/fr.po`, and add `Name[fr]` and `Description[fr]` to `kwin-script/metadata.json`.
  `make` picks up every `po/*.po` file by itself.
- Check a file: `msgfmt --check --statistics -o /dev/null po/fr.po`.
- For names of System Settings pages, use the official KDE translations, so that users find them.

## Releases

1. Update `version` in `service/Cargo.toml` and `Version` in `kwin-script/metadata.json`, and add the
   changes to `CHANGELOG.md`.
2. After merging: tag `vX.Y.Z` on `main` and push the tag.
3. Build the package from the tag with generic flags, so it runs on any x86-64 CPU
   (a local `makepkg.conf` may contain `-march=native`):
   ```sh
   cd packaging/aur/muteonminimize    # with pkgver set to the new version
   makepkg -f --config <copy of /etc/makepkg.conf with generic CFLAGS and RUSTFLAGS>
   ```
4. `gh release create vX.Y.Z` with the notes and the package plus its `SHA256SUMS`.
5. Update `pkgver` and `.SRCINFO` (`makepkg --printsrcinfo > .SRCINFO`) in `packaging/aur/muteonminimize`.
