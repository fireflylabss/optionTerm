# optionTerm — notas para agentes

## Build
- Dependências de sistema: GTK 4.14+, libadwaita 1.5+, **VTE GTK4** (`vte4` /
  `libvte-2.91-gtk4`), pkg-config, pango, cairo. **Não precisa de Zig.**
- `cargo build --release` ou `./scripts/run.sh`.
- Smoke test: `timeout 5 ./target/release/optionterm` (GApplication é
  single-instance — mate instâncias antigas antes: `pkill optionterm`).
- O warning `AdwTabBox reported min width -6` no startup é bug conhecido e
  inofensivo do libadwaita.
- Testes: `cargo test --release`.

## Empacotamento
- `packaging/deb/build-deb.sh` — usa `dpkg-deb` quando existe, senão monta o
  `.deb` com `ar`+`tar` (permite gerar em Arch/Fedora). Depende de
  `libvte-2.91-gtk4-0` em runtime.
- `packaging/appimage/build-appimage.sh` — linuxdeploy + plugin GTK.
  `NO_STRIP=1` é obrigatório: o `strip` embutido não entende `.relr.dyn`
  moderno. ⚠️ **Nunca construa o AppImage no Arch**: o resultado dá SIGSEGV no
  `ld.so`. O `release.yml` fixa `ubuntu-24.04` (glibc 2.38). O AppImage deve
  puxar `libvte-*.so` via linuxdeploy.
- `NOTICE` lista VTE como dependência LGPL-3.0-or-later (obrigatório); não
  use “powered by VTE” em taglines públicas.

## Release / AUR
- Publicado no AUR como **`optionterm`**.
- Fluxo: bump no `Cargo.toml` → `CHANGELOG.md` → commit → `git tag -a vX.Y.Z` →
  `gh release create` → atualizar `pkgver`+`sha256sums` no `PKGBUILD` →
  `makepkg --printsrcinfo > .SRCINFO` → `makepkg -f` → push no AUR.
- Dependências de runtime incluem `vte4`; makedepends: `cargo`, `pkgconf`
  (sem Zig/git para clonar fontes).

## Arquitetura
- `src/keys.rs` — `keys.toml` (overrides de atalho) + conversão da grafia
  humana (`Ctrl+Shift+T`) para a do GTK (`<Control><Shift>t`).
- `src/default_terminal.rs` — registra como terminal padrão:
  `xdg-terminals.list` + chave do GNOME + `kdeglobals` do KDE.
- `src/config.rs` — config próprio em `~/.option/terminal/config.toml` (TOML).
  Na primeira execução gera defaults (`window.tabs = "left"`).
- `src/terminal.rs` — wrapper fino em torno de `vte4::Terminal` (spawn, cores,
  fonte, busca, hyperlinks). `widget()` devolve o **`Overlay`** raiz.
- `src/app.rs` — shell Adwaita: abas, splits (4 direções, aninháveis), sidebar,
  actions, toasts. Tiling: new_split, goto_split, resize_split,
  toggle_split_zoom, equalize_splits.
- `src/ui.rs` — menus, diálogos e `SearchBar` (VTE `search_set_regex` +
  find next/prev).
- `src/pty.rs` — helpers `/proc` para cwd e “busy” via fd do PTY do VTE.
- `src/session.rs` — `session.toml` (abas, nº de painéis, cwd, títulos). Sem
  dumps de scrollback.

## Detalhes que já morderam
- OSC 7 devolve `file://host/path` — use `pwd_to_path`. A maioria dos shells
  nunca emite OSC 7: `TerminalView::pwd` cai pro `foreground_cwd` via
  `tcgetpgrp` + `/proc/<pgid>/cwd`.
- Touchpad de alta resolução: VTE com
  `enable_fallback_scrolling(false)` + `scroll_unit_is_pixels(true)`.
- `AdwTabBar` fecha aba no clique do meio por conta própria — gesto em
  `PropagationPhase::Capture` + `Claimed`.
- `GtkSearchEntry` engole Escape — trate no `stop-search` e num
  `EventControllerKey` em captura.
- `gtk4::accelerator_parse` exige GTK inicializado.
- Atalhos em `keys.toml`, separado do `config.toml`.
- `AdwTabView::close_page` é assíncrono: `Propagation::Stop` +
  `close_page_finish`.
- O `GFileMonitor` do config dispara nas nossas escritas — guard
  `SELF_WRITE_GRACE`.
- SIGTERM não dispara `close-request`; salve também com
  `unix_signal_add_local`.
- `cargo clippy --fix` desformata: rode `cargo fmt` depois.

## TODO
- [ ] Respeitar `text-scaling-factor` do GNOME Settings (hoje usa DPI/`gtk-font-name`).
- [x] Respeitar `gtk-enable-animations`, `gtk-font-name` no chrome e
      `gtk-decoration-layout` dinâmico.
- [x] `gtk-tabs-location` / `tabs = "bottom"`.
- [x] Salvar a **geometria** dos splits na sessão.

## Release channels
- Ver [VERSIONING.md](VERSIONING.md): changelog usa `x.y.z-stable` (ou
  alpha/beta); `Cargo.toml` / tags git ficam numéricos (`0.2.1`, `v0.2.1`).
- Não marque `stable` no changelog sem estar pronto pra release/AUR.
