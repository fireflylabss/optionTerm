# optionTerm — notas para agentes

## Build
- `libghostty-vt 0.2.x` exige **Zig 0.15.x** (o Zig 0.16 do sistema não compila).
  Use `./scripts/run.sh` ou: `PATH="/tmp/zig151/zig-0.15.2:$PATH" cargo build --release`
- Smoke test: `timeout 5 ./target/release/option-term` (GApplication é single-instance —
  mate instâncias antigas antes: `pkill option-term`).
- O warning `AdwTabBox reported min width -6` no startup é bug conhecido e inofensivo do libadwaita.
- Testes: `PATH="/tmp/zig151/zig-0.15.2:$PATH" cargo test --release`.

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
- `src/ui.rs` — menus (main/contexto/tiling) e diálogos (palette, preferences, shortcuts, about).
- `src/pty.rs` — PTY com leitura limitada por dispatch (anti-flood) e escrita com poll (pastes grandes).

## TODO
- [ ] **Obedecer muito bem a configuração do sistema (GTK/Adwaita), estilo Ghostty**:
  - [ ] Reagir em runtime a mudanças de tema claro/escuro do sistema (`AdwStyleManager::dark`)
        e a mudanças no `gtk-decoration-layout` (posição/estilo dos botões de janela).
  - [ ] Respeitar `gtk-enable-animations`, fontes do sistema (`gtk-font-name`) no chrome,
        e escala de texto (`text-scaling-factor`).
  - [ ] Suportar mais chaves do Ghostty: `gtk-titlebar`, `gtk-wide-tabs`,
        `window-decoration`, `background-opacity` (`window-theme` já é suportado).
  - [ ] Recarregar o config automaticamente quando o arquivo mudar (GFileMonitor),
        como o Ghostty faz.
- [ ] `gtk-tabs-location = bottom` (hoje mapeado para `top`).
