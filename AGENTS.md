# optionTerm — notas para agentes

## Build
- `libghostty-vt 0.2.x` exige **Zig 0.15.x** (o Zig 0.16 do sistema não compila).
  Use `./scripts/run.sh` ou: `PATH="/tmp/zig151/zig-0.15.2:$PATH" cargo build --release`
- Smoke test: `timeout 5 ./target/release/option-term` (GApplication é single-instance —
  mate instâncias antigas antes: `pkill option-term`).
- O warning `AdwTabBox reported min width -6` no startup é bug conhecido e inofensivo do libadwaita.
- Testes: `PATH="/tmp/zig151/zig-0.15.2:$PATH" cargo test --release`.

## Empacotamento
- `packaging/deb/build-deb.sh` — usa `dpkg-deb` quando existe, senão monta o `.deb`
  com `ar`+`tar` (permite gerar em Arch/Fedora).
- `packaging/appimage/build-appimage.sh` — linuxdeploy + plugin GTK.
  `NO_STRIP=1` é obrigatório: o `strip` embutido não entende `.relr.dyn` moderno.
  ⚠️ **Nunca construa o AppImage no Arch**: o resultado dá SIGSEGV no `ld.so`.
  O `release.yml` fixa `ubuntu-24.04` (menor distro com GTK 4.14 + libadwaita 1.5);
  isso define o piso de **glibc 2.38**.
- `git` é makedepend real: o `build.rs` do `libghostty-vt-sys` clona o Ghostty.

## Release / AUR
- Publicado no AUR como **`option-term`** (`ssh://aur@aur.archlinux.org/option-term.git`).
- `packaging/aur/` guarda uma cópia do `PKGBUILD`/`.SRCINFO` publicados (fonte da verdade é o repo do AUR).
- O `PKGBUILD` baixa o tarball oficial do **Zig 0.15.2** porque `extra/zig` já é 0.16.
- Fluxo de release: bump no `Cargo.toml` → `CHANGELOG.md` → commit → `git tag -a vX.Y.Z` →
  `gh release create` → atualizar `pkgver`+`sha256sums` no `PKGBUILD` →
  `makepkg --printsrcinfo > .SRCINFO` → `makepkg -f` (valida) → push no AUR.

## Arquitetura
- `src/config.rs` — config próprio em `~/.option/terminal/config.toml` (TOML).
  Na primeira execução é gerado a partir do config do Ghostty do sistema
  (`~/.config/ghostty/config`), com `window.tabs = "left"` (sidebar) por padrão.
- `src/terminal.rs` — DrawingArea + libghostty-vt (render, seleção, cursor, PTY).
- `src/app.rs` — shell Adwaita: abas, splits (4 direções, aninháveis), sidebar, actions, toasts.
  Tiling copiado dos defaults Linux do Ghostty (`Config.zig`): new_split (Ctrl+Shift+O/E),
  goto_split (Ctrl+Alt+setas, Ctrl+Super+[/]), resize_split (Ctrl+Shift+Super+setas, 10px),
  toggle_split_zoom (Ctrl+Shift+Enter), equalize_splits (peso por nº de folhas).
- `src/ui.rs` — menus (main/contexto/tiling), diálogos (palette, preferences, shortcuts, about)
  e a `SearchBar` (busca no scrollback).
- `src/pty.rs` — PTY com leitura limitada por dispatch (anti-flood) e escrita com poll (pastes grandes).
  `Pty::spawn` aceita um `cwd` (usado por splits e restauração de sessão).
- `src/graphics.rs` — Kitty graphics protocol: decoder PNG próprio (o `RustPngDecoder` do
  libghostty é quebrado — só faz `reserve`, nunca `resize`), conversão para BGRA
  premultiplicado e cache de `cairo::ImageSurface` por frame.
  ⚠️ `set_png_decoder` é **thread-local**: instale por thread, nunca com `Once`.
- `src/session.rs` — `~/.option/terminal/session.toml` (abas, nº de painéis, cwd, títulos).
  Só a forma do workspace é salva; nunca o scrollback.

## Detalhes que já morderam
- `cursor.blink` só funciona se for empurrado para o VT via `set_default_cursor_blink`
  (`Config::apply_cursor_to_terminal`); o snapshot de render lê o estado DECSCUSR, não o config.
- OSC 7 devolve `file://host/path` — use `pwd_to_path` antes de usar como diretório.
- O `GFileMonitor` do config dispara também nas **nossas** escritas; há um guard
  (`SELF_WRITE_GRACE`) para não entrar em loop de reload.
- SIGTERM não dispara `close-request`; a sessão é salva também por `unix_signal_add_local`.
- `cargo clippy --fix` converte `if` aninhados em let-chains e **desformata** o código:
  rode `cargo fmt` **depois** do clippy.

## TODO
- [ ] **Renderer GPU** (`GskRenderNode`/`snapshot` no lugar do Cairo). É uma reescrita
      completa do `Session::paint` (glifos, seleção, cursor, imagens kitty) e deve ser
      feita **isolada, na sua própria release**, com comparação de frame antes/depois.
      O Cairo vira gargalo com imagens grandes e scroll rápido.
- [ ] Respeitar `gtk-enable-animations`, `gtk-font-name` no chrome e `text-scaling-factor`.
- [ ] `gtk-decoration-layout` dinâmico (hoje só o padrão do sistema no startup).
- [ ] Mais chaves do Ghostty: `gtk-titlebar`, `gtk-wide-tabs`, `window-decoration`.
- [ ] `gtk-tabs-location = bottom` (hoje mapeado para `top`).
- [ ] Salvar a **geometria** dos splits na sessão (hoje só o nº de painéis; tudo volta
      como splits horizontais).
- [ ] Realçar todos os matches da busca no render, não só o atual.
