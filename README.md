# treetop

An htop-style system monitor for KDE Plasma, written in Rust with
[CXX-Qt](https://github.com/KDAB/cxx-qt) and Qt Quick. It is part htop
(a process tree you can expand, sort, filter and signal), part
[kstats](https://github.com/oowl/kstats) (live graphs in the panel), and part
amdgpu_top (detailed GPU activity), plus container awareness.

## What it shows

- **Process tree** with htop's tree lines, expand/collapse, sort by any column,
  filter, tagging, and htop's keys (F9 kill menu, F7/F8 nice, F5 tree, `/`
  filter…). The order stays stable: near-idle processes don't reshuffle, and
  it holds still entirely while the pointer is over the list.
- **Signals and renice**, falling back to `pkexec` (a polkit prompt) for
  processes you don't own, such as root's.
- **Containers**: Docker/Podman/containerd processes get a coloured stripe and
  a badge naming their container, and a "Containers" view groups them, with
  names from the engines' API sockets.
- **CPU** per core (htop meters, or one combined meter), **memory**/swap,
  **disks** (I/O and filesystem usage) and **network**.
- **GPU** (AMD via sysfs, NVIDIA via nvidia-smi): busy %, memory controller,
  VRAM/GTT, shader/memory clocks, edge/junction/memory temperatures, fan, power
  vs. cap, voltage and PCIe link; per-process GPU %, VRAM and per-engine use
  (GFX/compute/DMA/decode/encode) from DRM fdinfo, like amdgpu_top.
- A **tray icon** (closing the window hides it there) and a **Plasma panel
  widget** with mini graphs; clicking the widget toggles the window.

Every column header has a tooltip explaining it; right-click it to choose
columns. `/nix/store` hashes are shortened to `…` (toggle in that menu).

## Design notes

All sampling runs on a worker thread in Rust. The GUI thread only receives the
rows that changed since the last update, so scrolling stays smooth. Text uses
Qt's native renderer and KDE's fixed-width font.

## Build and run

```sh
nix develop -c cargo run --release
```

or `nix build` / `nix run`. The package also installs the panel widget
(`share/plasma/plasmoids/com.benwbooth.treetop`); from a checkout, install it
with `kpackagetool6 -t Plasma/Applet -i plasmoid`.

## Keys

`F1`/`h` help · `↑↓ PgUp PgDn` move · `Space` tag · `+ − ← →` expand/collapse ·
`*` expand all · `Enter` details · `F9`/`k` signals · `F7`/`F8` nice ·
`F5`/`t` tree/list · `c` containers · `g` GPU view · `1` per-core/combined CPU ·
`r` collapse resources · `K` kernel threads · `P M T` sort · `/` filter ·
`Esc`/`q` hide to tray · `Ctrl+Q` quit · `Ctrl+=`/`Ctrl+-` font size

## Credits

Inspired by [htop](https://htop.dev), [kstats](https://github.com/oowl/kstats)
(MIT) and [amdgpu_top](https://github.com/Umio-Yasuno/amdgpu_top).
