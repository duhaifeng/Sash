# Sash

**English** · [中文](README.zh-CN.md)

Sash places the focused window into halves, thirds, quarters, and other regions of the screen. Use a shortcut, or drag the window to an edge or corner and release it when the preview appears.

macOS uses the Accessibility API. Linux uses X11 and an EWMH window manager, or on GNOME Wayland a small Shell extension. The layout and default shortcuts follow [Magnet](https://magnet.crowdcafe.com) 3.0.7. Sash is a separate project and is not affiliated with CrowdCafé.

<p>
  <img src="assets/sash.svg" width="96" alt="Sash icon: four windows on a blue tile">
</p>

## Features

- Halves, quarters, thirds, and two-thirds, plus maximize, center, and restore.
- Drag-to-snap with a preview. Dragging a snapped window away can restore its previous size.
- Move the window to the next or previous display, and toggle stay-on-top.
- Separate layouts for landscape and portrait displays. The same shortcut does the matching region on each.
- A menu-bar app on macOS, with a settings window. On Linux, the same behavior is configured in a TOML file.
- Per-app ignore lists.

## Requirements

- Rust 1.85 or newer (the crate uses the 2024 edition).
- macOS 11 or newer, or a Linux session on X11 (GNOME/Mutter, KDE/KWin, Xfwm, Openbox, and other EWMH window managers).
- On Wayland, GNOME Shell 50 with the Sash extension (see [GNOME on Wayland](#gnome-on-wayland)). Wayland does not let one client move another application's windows, so other compositors are limited to XWayland windows.

## Install

Build from source:

```sh
cargo build --release
./target/release/sash
```

Or install the binary onto `PATH`:

```sh
cargo install --path .
sash
```

### macOS application

`./package-mac.sh` produces `dist/Sash.app` and `dist/sash_<version>_macos_<arch>.zip`. The first launch needs Accessibility permission for the binary you actually run (`target/debug/sash`, `target/release/sash`, or `Sash.app`). A new build can be a different binary to macOS, so the permission may have to be granted again.

If Magnet.app is running it owns the same shortcuts. Quit it before expecting Sash to receive them.

### Debian package

`./package-deb.sh` builds `dist/sash_<version>_amd64.deb`. On Linux x86_64 it compiles locally. On any other machine it uses Docker. The package installs `/usr/bin/sash`, a scalable icon, an XDG autostart entry, and the GNOME Shell extension for all users (`/usr/share/gnome-shell/extensions/sash@sash.local`). On install and upgrade its `postinst` enables the extension for the users already on the machine and removes any older copy in their home directory. On GNOME Wayland, **log out and back in once** after installing; nothing else needs to be set up.

## Usage

With no subcommand, Sash keeps running: shortcuts and, unless disabled, drag-to-snap. Only one copy runs per user on Linux: starting it again, for instance from the application menu, does nothing, and quitting it stops everything.

```sh
sash                                  # run in the background
sash list                             # commands and shortcuts
sash apply left-third                 # run one command on the focused window
sash print-commands > commands.json   # built-in command definitions
sash print-config > config.toml       # commented example preferences
sash --config /path/to/config.toml    # use this file instead of the default
```

Command names are matched without case, spaces, or hyphens, so `left-third` and `Left Third` are the same command. `RUST_LOG=sash=debug` raises the log level. The default level is `info`.

On macOS the process is a menu-bar extra and has no Dock icon. The menu lists the commands for the display that contains the focused window, then **Ignore** for the current app, **Settings…**, **About Sash**, and **Quit Sash**. If Accessibility permission is missing, the menu starts with **Not Authorized to Control Your Windows** and **Authorize…**.

## Shortcuts

Shortcut strings are written as `Ctrl+Alt+…`. On macOS, Alt is Option and Super is Command. On Linux, Super is the Super key. These defaults match Magnet: its Ctrl+Option is Ctrl+Alt here, and its Command is Super.

The table is the landscape layout. On a portrait display the same keys apply the portrait regions (for example Ctrl+Alt+D is Top Third instead of Left Third).

| Command | Shortcut |
| --- | --- |
| Left / Right / Top / Bottom half | Ctrl+Alt+Left / Right / Up / Down |
| Top Left / Top Right / Bottom Left / Bottom Right | Ctrl+Alt+U / I / J / K |
| Left / Center / Right third | Ctrl+Alt+D / F / G |
| Left / Center / Right two thirds | Ctrl+Alt+E / R / T |
| Maximize / Center / Restore | Ctrl+Alt+Return / C / Backspace |
| Next / Previous display | Ctrl+Alt+Super+Right / Left |
| Stay on top | Ctrl+Alt+Super+Up |

If the desktop already owns a shortcut, the log names it and Sash skips that binding. On Ubuntu, Ctrl+Alt+T is often the terminal, which collides with Right Two Thirds. Rebind it in the commands file, or on macOS in **Settings… → Commands**.

## Configuration

Preferences are read from `$XDG_CONFIG_HOME/sash/config.toml`, or `~/.config/sash/config.toml` when `XDG_CONFIG_HOME` is unset. If that file does not exist, Sash uses its built-in defaults. Create a starting file with:

```sh
mkdir -p ~/.config/sash
sash print-config > ~/.config/sash/config.toml
```

| Key | Default | Meaning |
| --- | --- | --- |
| `snap_windows` | `true` | Snap while dragging. |
| `restore_to_original_size` | `true` | Dragging a snapped window away restores the size from before the snap. |
| `activation_edge_width` | `6` | Thickness, in pixels, of the strips along screen edges. |
| `activation_area_scale` | `1.0` | Scale of activation areas that do not touch a screen edge, such as Center Two Thirds. |
| `padding` | `0` | Margin, in pixels, around a snapped window. |
| `fit_tightly_to_screen_edges` | `false` | Leave the margin off where the window meets the screen edge. |
| `hide_menu_bar_icon` | `false` | Hide the status icon. On macOS, launching Sash again opens Settings; on GNOME, set it back to `false` in the file and restart Sash. |
| `ignored_apps` | `[]` | Applications to leave alone. Linux: `WM_CLASS` instance or class (`xprop WM_CLASS`). macOS: application name or bundle id. |
| `commands_file` | unset | JSON file that replaces the built-in commands. |
| `highlight.fill` | `#3d8bfd55` | Drag-preview fill, `#RRGGBBAA`. On Linux the fill needs a compositor; otherwise only the outline is drawn. |
| `highlight.outline` | `#3d8bfddd` | Drag-preview outline. |
| `highlight.outline_width` | `3` | Outline width in pixels. |

The annotated example is [`assets/config.example.toml`](assets/config.example.toml).

### Commands file

`sash print-commands` writes the built-in definitions. Point `commands_file` at that JSON to edit them. Paths in the preferences file are relative to the preferences file's directory.

Landscape commands use a 24×12 grid. Portrait commands use 12×24. A region is `[[x, y], [width, height]]` in grid cells. Each entry is either a `target` region or a `special` command: `next_display`, `previous_display`, `center`, `restore`, or `stay_on_top`. `activation` lists the drag areas that select the command. Optional fields:

- `menu_bar` (default `true`): show the command in the macOS status menu.
- `separator_after`: draw a menu divider after the command.

See [`assets/commands.json`](assets/commands.json).

### Ignored apps from the menu

**Ignore** on macOS appends the application's bundle id to `ignored-apps.txt` next to the preferences file, one identifier per line. That list is separate from `ignored_apps` in the TOML file.

## macOS settings window

**Settings…** opens the window. Launching Sash again also opens it, which is how you get the window back after hiding the menu-bar icon.

The sidebar has two icon-only items, Preferences and Commands. The window has no title text and no zoom button. Preferences is 600 pt wide, Commands is 876 pt wide, and the height is 830 pt or the visible screen height when the screen is shorter.

**Preferences** is grouped into General, Mouse, Activation Areas, Target Areas, Spacing, and Defaults. Each row is a label and a switch, stepper, or color well: launch at login, hide the menu-bar icon, drag-to-snap, restore size on drag-away, activation scale (percent) and edge width, preview fill, padding, fit to screen edges, and Restore to defaults (preferences only, commands only, or both). Launch at login writes `~/Library/LaunchAgents/local.sash.plist` and takes effect at the next login.

**Commands** lists Horizontal and Vertical. Each row is a thumbnail of the target region (special commands use an icon) and the name. A divider is a short bar. The group header can collapse the group, and **+** adds a command or a divider. The selected command is edited on the right: the target grid (fraction labels such as ⁵⁄₈ sit above and beside it), the shortcut (a switch and a recorder; a conflict within the same orientation is reported and not saved; Delete clears; Esc cancels), drag activation areas (a switch and a grid; drag to add a rectangle, click an existing one to remove it), and whether the command appears in the status menu. The selected row has two buttons, menu visibility and delete. Built-in commands cannot be deleted. The header is the command name; custom commands can be renamed there.

Edits apply immediately. The window rewrites `config.toml` and `commands.json`, creating `commands_file` on the first command edit. The first rewrite of each file is copied to `config.toml.bak` or `commands.json.bak`. **Comments in those files are not kept.** `ignored-apps.txt` is left as it is.

There is no settings window on Linux. Edit the files directly and restart Sash.

### GNOME on Wayland

Wayland gives an ordinary program no way to move other applications' windows or to receive global keys, so on GNOME Sash uses a Shell extension. The extension does the window operations, grabs the shortcuts and watches window drags; `sash` holds all the snapping logic and talks to it over D-Bus (`local.sash.Shell`).

With the Debian package the extension is already installed and enabled, and no command is needed. For a binary you built yourself, run once:

```sh
sash install-extension   # copies the extension to ~/.local/share/gnome-shell/extensions and enables it
```

`sash` does the same by itself when it starts and finds the extension not loaded, and shows a notification. It also notifies when the Shell still runs an older extension than the binary expects.

GNOME Shell on Wayland cannot be restarted, so **log out and back in** once for it to load the extension (and again after updating it). Then run `sash` as usual. Sash picks the GNOME backend by itself on a GNOME Wayland session. At login it waits up to 30 seconds for the Shell to load the extension. If the extension is still not running it warns and falls back to X11, which only reaches XWayland windows. `SASH_BACKEND=gnome` or `SASH_BACKEND=x11` forces one. The extension is written for GNOME Shell 50; other versions need a matching entry in its `metadata.json` and have not been tried.

Things to know:

- GNOME's own Ctrl+Alt+Left, Right, Up and Down switch workspaces. GNOME wins, so the log reports those four as refused and the half-screen shortcuts do nothing. Free them, for example:

  ```sh
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-left  "['<Super>Page_Up', '<Super>KP_Prior', '<Super><Alt>Left']"
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-right "['<Super>Page_Down', '<Super>KP_Next', '<Super><Alt>Right']"
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-up    "[]"
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-down  "[]"
  ```

  or rebind those commands in `commands_file`. Shortcuts only fire in normal mode, not in the overview or on the lock screen.
- Ubuntu ships the Tiling Assistant extension, which can also tile a window dragged to a screen edge. If you use drag-to-snap, disable it (`gnome-extensions disable tiling-assistant@ubuntu.com`) and turn off `org.gnome.mutter edge-tiling`, or both may act on the same drag.
- Windows that cannot be resized (many dialogs) are only moved.
- Drag-to-snap follows the window moves GNOME itself runs (a title-bar drag, or Super+drag).

- The top bar shows a Sash icon while `sash` runs, and hides it when `sash` quits. Its menu lists the commands for the display the window is on, then **Ignore** for that application, switches for **Snap Windows by Dragging** and **Restore Original Size When Dragging Away**, **Settings…**, **Reload Settings** and **Quit Sash**. A failed reload keeps the old settings and shows a notification.

- **Settings…** opens a settings window (also `gnome-extensions prefs sash@sash.local`). *General* has the preferences, the snap preview colours, the ignored applications, **Open config.toml** and **Restore Preferences to Defaults**. *Landscape* and *Portrait* list the commands of each orientation: expand one to change its shortcut (press **Set…**, then the keys), whether it is in the menu and followed by a divider, its target area and its activation areas (drag over the grid), or add and delete your own commands. Changes apply at once and are saved to `config.toml` and `commands.json` (the previous files are kept once as `.bak`), so comments in `config.toml` are not kept. Sash has to be running for the window to work. If GNOME refuses a shortcut, the window says so and the command needs another one.

### Linux autostart

The Debian package installs an autostart entry. For a binary you built yourself:

```ini
# ~/.config/autostart/sash.desktop
[Desktop Entry]
Type=Application
Name=Sash
Exec=/path/to/sash
```

## Development

```sh
cargo test
./restart.sh              # debug build, then run; stops a previous sash from this repo
./restart.sh --release
./restart.sh -- list      # arguments after -- are passed to sash
```

## Relation to Magnet

The behavior is modelled on Magnet 3.0.7 for macOS. What follows is from static inspection of `/Applications/Magnet.app` (REA `inspect-native-dispatch-metadata`, `decode-interface-builder`, `inspect-signature`, plus the app's resources and Mach-O strings) and, for the settings window, from screenshots and the accessibility tree of a running Magnet. Magnet was not decompiled. Sash does not share Magnet's code.

Observed directly:

- Commands live in `HorizontalCommands.json` and `VerticalCommands.json`. Landscape uses a 24×12 grid and portrait a 12×24 grid. Each command has a `targetArea`, an `activationArea`, and a Carbon shortcut. [`assets/commands.json`](assets/commands.json) uses those coordinates and the default shortcuts.
- Special commands: next and previous display, center, restore, stay on top.
- Preference fields include `snapWindows`, `restoreToOriginalSize`, `activationAreaEdgeWidth`, `activationAreaScale`, `highlightActivationAreas`, activation and target appearance, `paddingAroundWindows`, `fitTightlyToScreenEdges`, the green-button menu, an application blacklist, and iCloud sync.
- `MouseConductor` states include `idle`, `down`, `drag`, and `skip`, and it stores `originalWindowFrame`. `WindowConductor` stores `originalFrames` and `expandedFrames`.
- `MainMenu.nib` is the application menu. The status menu is built in code (`StatusMenu`, `MenuConductor`, `BlacklistConductor`, `onMenuOpen`). Every command JSON entry has `visibleInMenuBar`, and unnamed separators sit after Down, Bottom Right, Right Third (Bottom Third in portrait), Right Two Thirds (Bottom Two Thirds in portrait), and Previous Display. `separator_after` matches those positions. Localized strings include “Ignore …”, “Not Authorized to Control Your Windows”, “Authorize…”, “Quit Magnet”, “Settings…”, and “About Magnet”.

Inferred, and not verified by decompiling Magnet:

- An edge activation area is only live within `activation_edge_width` pixels of the screen edge. An interior area is scaled by `activation_area_scale`.
- One `padding` separates adjacent snapped windows. The side against the screen edge is a full `padding` unless `fit_tightly_to_screen_edges` is set.
- Numeric defaults (6 px edge, 0 padding, preview colors) were chosen for Sash. Magnet's defaults were not recovered.
- Menu order, when each item appears, and the exact Ignore behavior were inferred from the strings and the command tables, not from watching Magnet's menu. Switching command tables with the display orientation, storing the ignore list in its own text file, and opening a preferences file from Settings were Sash's own choices. The settings window's page names, copy, and class names (`SettingsWindowController`, `SettingsSidebarViewController`, `SettingsContentViewController`) come from the static inspection. Its layout comes from screenshots and the accessibility tree of Magnet's settings window (a 70 pt sidebar, 275 pt list, 530 pt detail, 52 pt header, 30 pt rows, 486 pt cards, and a 24×12 grid of 13 pt cells with 3 pt gaps) and from icon names in the asset catalog (`inspect-asset-catalog`: Commands, Preferences, add, chevron, remove, menu-bar, and others). Dark colors were sampled from screenshots. Light colors were not observed and were chosen for Sash. Some control behavior was observed; the rest is inferred from the appearance.

Not implemented: a Stay On Top checkmark in the menu, the Help link, the green-button menu and iCloud sync in Settings (Linux has no equivalent control), highlighting activation areas, a target-area style menu and outline (Sash has a single fill color), Magnet's preset cells for choosing an activation area (Sash drags a free rectangle), a confirmation when a region conflicts with another command, and the third button on a selected command row (green-button menu visibility).
