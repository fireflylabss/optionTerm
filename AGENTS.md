# optionTerm — notas para agentes

## Build
- `libghostty-vt 0.2.x` exige **Zig 0.15.x** (o Zig 0.16 do sistema não compila).
  Use `./scripts/run.sh` ou: `PATH="/tmp/zig151/zig-0.15.2:$PATH" cargo build --release`
- Smoke test: `timeout 5 ./target/release/optionterm` (GApplication é single-instance —
  mate instâncias antigas antes: `pkill optionterm`).
- O warning `AdwTabBox reported min width -6` no startup é bug conhecido e inofensivo do libadwaita.
- Testes: `PATH="/tmp/zig151/zig-0.15.2:$PATH" cargo test --release`.

## Medindo o render
- `OPTION_TERM_PROFILE=1` liga o `src/profile.rs`: resumo por fase de `Session::paint`
  (p50/p99, células e glifos por frame) a cada 120 frames **ou** 1,5 s. Desligado, custa
  um `bool`. O `Drop` não é garantido (a `Session` fica presa em closures do GTK), por isso
  o gatilho por tempo existe.
- `scripts/bench-render.sh [static|scroll|flood] [--ansi]` — usa um `HOME` isolado em
  `target/bench/home` para gerar config/sessão do zero, então a janela sempre abre em
  960x640 e o grid (86x24) é comparável entre runs. `$SHELL` aponta pro
  `scripts/bench-workload.py`, que se dimensiona pelo PTY e não usa RNG nem relógio.
- **Baseline 0.1.5 (86x24, 2064 células, monospace 13):** `paint` p50 ≈ **10,2 ms**,
  dos quais **97% era o Pango** (`layout.set_text` + `show_layout` por célula, ~4,8 µs
  por glifo). Cairo puro (fundo, retângulos, decorações, imagens) é ~3%.
  ⚠️ Conclusão: trocar Cairo→GSK **não resolveria** — `Snapshot::append_layout` também
  recebe um `pango::Layout`. O ganho está em agrupar runs e cachear glifos.
- **Depois do agrupamento em runs** (`BgRuns`/`TextRuns` em `terminal.rs`), mesmo grid:

  | workload | antes | depois | runs/frame |
  |---|---|---|---|
  | `static` | 10,23 ms | **1,14 ms** | 2063 → 24 (uma por linha) |
  | `scroll` | 9,70 ms | **1,05 ms** | 1977 → 23 |
  | `static --ansi` | 9,98 ms | **2,41 ms** | 2063 → 264 |

  `glyphs/frame` não muda em nenhum caso — nada deixou de ser desenhado.
  Cor não custa nada por si: no baseline ANSI dava 9,66 vs 9,92 ms. Ela só importa
  **agora**, porque troca de estilo quebra run (264 runs em vez de 24). É o teto que
  o cache de glifos vai remover.
- No modo `flood` a fonte fd do PTY **starva o frame clock**: ~1 fps, 2 frames em 1,5 s.
  Throughput e latência de render são problemas separados; não meça um com o outro.

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
- Publicado no AUR como **`optionterm`** (`ssh://aur@aur.archlinux.org/optionterm.git`).
  Renomeado de `option-term` na 0.1.7. O AUR **não renomeia pacote**: publica-se um novo e
  pede-se merge do antigo pela interface web (não tem API). O novo PKGBUILD leva
  `conflicts`/`replaces`/`provides = option-term` e instala um symlink `option-term`,
  senão quem já tinha instalado não migra e o binário antigo fica órfão.
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
  E **a maioria dos shells nunca emite OSC 7** (o Ghostty injeta shell integration, nós não):
  por isso `TerminalView::pwd` cai pro `Pty::foreground_cwd`, que lê
  `tcgetpgrp` + `/proc/<pgid>/cwd`. Isso não precisa de cooperação do shell e ainda
  segue `cd` feito dentro de um programa rodando. Teste ponta a ponta: `session.toml`
  guarda o `pwd()` de cada painel.
- **Roda de mouse de alta resolução** (libinput, passos de 1/120 de detent) manda `dy`
  fracionário. Arredondar por evento dá **sempre zero** — foi assim que o scroll ficou
  morto. Use `accumulate_wheel`, que carrega o resto.
- **Modo 1049 ≠ 1047.** `ALT_SCREEN` é 1047, mas vim/less/htop usam **1049**
  (`ALT_SCREEN_SAVE`). Checar só 1047 não detecta tela alternada em programa real nenhum.
  Use `Session::on_alternate_screen`, que cobre 1049/1047/47.
- Roda de mouse tem três destinos, nessa ordem: aplicação (se ligou mouse tracking),
  cursor keys (tela alternada, respeitando DECCKM), viewport (resto). Errar a ordem
  faz o scroll "não funcionar" em contextos específicos.
- **Ligaduras só são seguras** porque fonte de programação mantém o avanço da ligadura
  igual à soma dos glifos que ela substitui. Há teste afirmando isso
  (`ligatures_preserve_the_cell_advance`, pula se a FiraCode não estiver instalada).
  `measure_cell` mede **sempre** com ligaduras desligadas, senão ligar/desligar a config
  reflui a grade inteira.
- O `GFileMonitor` do config dispara também nas **nossas** escritas; há um guard
  (`SELF_WRITE_GRACE`) para não entrar em loop de reload.
- SIGTERM não dispara `close-request`; a sessão é salva também por `unix_signal_add_local`.
- `cargo clippy --fix` converte `if` aninhados em let-chains e **desformata** o código:
  rode `cargo fmt` **depois** do clippy.

## TODO
- [ ] **Pipeline de texto** (mede-se com `scripts/bench-render.sh`, ver "Medindo o render").
      Ordem definida pela medição, do maior retorno pro menor:
      1. ~~Agrupar células contíguas em runs~~ — feito (`BgRuns`/`TextRuns`).
         `is_batchable` decide: só grafema único de largura 1 e sem decoração entra
         numa run; CJK, clusters compostos, sublinhado e tachado seguem no caminho
         per-cell, que ficou intacto. Células vazias viram `gap` e só materializam
         espaços se a run continuar, então tela vazia continua de graça.
         O loop agora faz **duas passadas por linha** (fundos, depois texto): assim
         nenhum glifo é cortado pelo fundo da célula seguinte, o que a ordem
         intercalada antiga permitia.
      2. Cache de glifos por `(grafema, estilo)` guardando o `GlyphString`, desenhado
         com `show_glyph_string`. Tira o shaping do frame — e é a infra que permite
         ligaduras (hoje há um `disable_ligatures()` porque o modelo é per-cell).
      3. Damage tracking: os 20+ `queue_draw()` repintam a superfície inteira; até o
         piscar do cursor custa um frame cheio.
      4. Só então **GSK/GPU** (`GskRenderNode`/`snapshot` no lugar do Cairo), release
         isolada, com comparação de frame antes/depois. Exige um widget próprio
         (`ObjectSubclass` + `WidgetImpl::snapshot`): `DrawingArea` só expõe Cairo, e
         hoje ele está cravado em ~14 assinaturas de `terminal.rs`. Vale pelas imagens
         Kitty (texture nodes) e pelo atlas de glifos do GSK, **não** pelos 97% do Pango.
- [ ] Respeitar `gtk-enable-animations`, `gtk-font-name` no chrome e `text-scaling-factor`.
- [ ] `gtk-decoration-layout` dinâmico (hoje só o padrão do sistema no startup).
- [ ] Mais chaves do Ghostty: `gtk-titlebar`, `gtk-wide-tabs`, `window-decoration`.
- [ ] `gtk-tabs-location = bottom` (hoje mapeado para `top`).
- [ ] Salvar a **geometria** dos splits na sessão (hoje só o nº de painéis; tudo volta
      como splits horizontais).
- [ ] Realçar todos os matches da busca no render, não só o atual.
