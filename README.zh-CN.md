# Sash

[English](README.md) · **中文**

Sash 把当前窗口放到半屏、三分之一、四分之一和其他屏幕分区。按快捷键即可，也可以把窗口拖到屏幕边缘或角落，出现预览后松开。

macOS 通过辅助功能接口管理窗口，Linux 通过 X11 和遵循 EWMH 的窗口管理器管理窗口；在 GNOME Wayland 下则通过一个 Shell 扩展。分区和默认快捷键参照 [Magnet](https://magnet.crowdcafe.com) 3.0.7。Sash 是独立项目，与 CrowdCafé 无关。

<p>
  <img src="assets/sash.svg" width="96" alt="Sash 图标：蓝底上的四个窗口">
</p>

## 功能

- 半屏、四分之一、三分之一、三分之二，以及最大化、居中和还原。
- 拖拽吸附，并显示预览。把已吸附的窗口拖走时，可以恢复吸附前的大小。
- 移到下一个或上一个显示器，以及切换置顶。
- 横屏和竖屏使用不同的分区。同一个快捷键会落到该方向上对应的区域。
- macOS 上是菜单栏程序，带设置窗口。Linux 上用 TOML 文件配置同样的行为。
- 可以按应用忽略。

## 环境要求

- Rust 1.85 或更新版本（crate 使用 2024 edition）。
- macOS 11 或更新版本，或者运行在 X11 上的 Linux 会话（GNOME/Mutter、KDE/KWin、Xfwm、Openbox，以及其他遵循 EWMH 的窗口管理器）。
- 在 Wayland 下需要 GNOME Shell 50 和 Sash 扩展（见 [GNOME Wayland](#gnome-wayland)）。Wayland 没有让一个程序移动其他应用窗口的通用接口，所以其他合成器只能移动 XWayland 窗口。

## 安装

从源码构建：

```sh
cargo build --release
./target/release/sash
```

或者把二进制装进 `PATH`：

```sh
cargo install --path .
sash
```

### macOS 应用

`./package-mac.sh` 会生成 `dist/Sash.app` 和 `dist/sash_<版本>_macos_<架构>.zip`。第一次运行需要给实际执行的那个二进制授予辅助功能权限（`target/debug/sash`、`target/release/sash` 或 `Sash.app`）。重新编译后，系统可能把它当成另一个程序，需要再授权一次。

如果 Magnet.app 正在运行，它会占用同一套快捷键。要让 Sash 收到按键，先退出 Magnet.app。

### Debian 包

`./package-deb.sh` 会生成 `dist/sash_<版本>_amd64.deb`。在 Linux x86_64 上本地编译；在其他系统上通过 Docker 编译。包内包含 `/usr/bin/sash`、可缩放图标、一个 XDG 开机自启项，以及系统级的 GNOME Shell 扩展（`/usr/share/gnome-shell/extensions/sash@sash.local`）。安装或升级时，`postinst` 会为本机已有用户启用该扩展，并清掉旧的用户级副本。GNOME Wayland 用户装完后只需**注销再登录一次**，不用再手动设置。

## 使用

不带子命令时，Sash 会一直运行：响应快捷键，并在未关闭时响应拖拽吸附。Linux 上每个用户只会运行一个：再次启动（例如从应用菜单）什么都不会发生，退出它就会停掉全部功能。

```sh
sash                                  # 在后台运行
sash list                             # 查看命令和快捷键
sash apply left-third                 # 对当前窗口执行一次命令
sash print-commands > commands.json   # 导出内置命令
sash print-config > config.toml       # 导出带注释的示例配置
sash --config /path/to/config.toml    # 使用指定配置文件
```

命令名不区分大小写，空格和连字符也会被忽略，因此 `left-third` 和 `Left Third` 是同一条命令。`RUST_LOG=sash=debug` 可以提高日志级别，默认是 `info`。

在 macOS 上，程序只出现在菜单栏，没有程序坞图标。菜单先列出当前窗口所在显示器适用的命令，然后是当前应用的 **Ignore**、**Settings…**、**About Sash** 和 **Quit Sash**。没有辅助功能权限时，菜单顶部会多出 **Not Authorized to Control Your Windows** 和 **Authorize…**。

## 快捷键

快捷键写成 `Ctrl+Alt+…`。在 macOS 上，Alt 是 Option，Super 是 Command。在 Linux 上，Super 是 Super 键。这套默认值沿用 Magnet：它的 Ctrl+Option 在这里是 Ctrl+Alt，它的 Command 在这里是 Super。

下表是横屏分区。竖屏使用同一批按键，但落到竖屏分区上（例如 Ctrl+Alt+D 是顶部三分之一，而不是左侧三分之一）。

| 命令 | 快捷键 |
| --- | --- |
| 左 / 右 / 上 / 下半屏 | Ctrl+Alt+Left / Right / Up / Down |
| 左上 / 右上 / 左下 / 右下 | Ctrl+Alt+U / I / J / K |
| 左 / 中 / 右三分之一 | Ctrl+Alt+D / F / G |
| 左 / 中 / 右三分之二 | Ctrl+Alt+E / R / T |
| 最大化 / 居中 / 还原 | Ctrl+Alt+Return / C / Backspace |
| 下一个 / 上一个显示器 | Ctrl+Alt+Super+Right / Left |
| 置顶 | Ctrl+Alt+Super+Up |

如果桌面环境已经占用了某个快捷键，日志会写出这个按键，Sash 会跳过它。Ubuntu 上的 Ctrl+Alt+T 通常打开终端，会和右侧三分之二冲突。可以在命令文件里改掉，macOS 上也可以在 **Settings… → Commands** 里改。

## 配置

配置文件是 `$XDG_CONFIG_HOME/sash/config.toml`；没有设置 `XDG_CONFIG_HOME` 时是 `~/.config/sash/config.toml`。文件不存在时使用内置默认值。可以这样生成一份起点：

```sh
mkdir -p ~/.config/sash
sash print-config > ~/.config/sash/config.toml
```

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `snap_windows` | `true` | 拖拽时吸附。 |
| `restore_to_original_size` | `true` | 把已吸附的窗口拖走时，恢复吸附前的大小。 |
| `activation_edge_width` | `6` | 屏幕边缘激活条的厚度，单位是像素。 |
| `activation_area_scale` | `1.0` | 不贴着屏幕边缘的激活区的缩放，例如 Center Two Thirds。 |
| `padding` | `0` | 吸附后窗口周围的边距，单位是像素。 |
| `fit_tightly_to_screen_edges` | `false` | 窗口贴着屏幕边缘的那一侧不留边距。 |
| `hide_menu_bar_icon` | `false` | 隐藏状态栏图标。macOS 上再次启动 Sash 会打开设置窗口；GNOME 上需要在文件里改回 `false` 再重启 Sash。 |
| `ignored_apps` | `[]` | 不处理的应用。Linux 填 `WM_CLASS` 的 instance 或 class（`xprop WM_CLASS`）。macOS 填应用名或 bundle id。 |
| `commands_file` | 未设置 | 用来替换内置命令的 JSON 文件。 |
| `highlight.fill` | `#3d8bfd55` | 拖拽预览的填充色，`#RRGGBBAA`。Linux 上填充需要合成器，否则只画边框。 |
| `highlight.outline` | `#3d8bfddd` | 拖拽预览的边框颜色。 |
| `highlight.outline_width` | `3` | 边框宽度，单位是像素。 |

带注释的示例在 [`assets/config.example.toml`](assets/config.example.toml)。

### 命令文件

`sash print-commands` 会写出内置命令。把 `commands_file` 指到这份 JSON 后再修改。配置文件里的相对路径相对于配置文件所在的目录。

横屏命令使用 24×12 的网格，竖屏命令使用 12×24 的网格。一个区域写成 `[[x, y], [宽度, 高度]]`，单位是格子。每一项要么有 `target` 区域，要么是 `special` 命令：`next_display`、`previous_display`、`center`、`restore`、`stay_on_top`。`activation` 是拖拽时选中这条命令的区域。可选字段：

- `menu_bar`（默认 `true`）：是否出现在 macOS 状态菜单里。
- `separator_after`：在这条命令后面画一条菜单分隔线。

见 [`assets/commands.json`](assets/commands.json)。

### 从菜单忽略的应用

macOS 菜单里的 **Ignore** 会把该应用的 bundle id 追加到配置文件旁边的 `ignored-apps.txt`，每行一个标识。这份名单和 TOML 里的 `ignored_apps` 是分开的。

## macOS 设置窗口

**Settings…** 会打开设置窗口。再次启动 Sash 也会打开它；隐藏菜单栏图标之后，用这个办法把窗口找回来。

侧栏只有两个图标：Preferences 和 Commands。窗口没有标题文字，也没有缩放按钮。Preferences 宽 600 pt，Commands 宽 876 pt，高度是 830 pt；屏幕更矮时，高度取屏幕的可用高度。

**Preferences** 分成 General、Mouse、Activation Areas、Target Areas、Spacing、Defaults。每一行左边是名称，右边是开关、步进器或颜色框：登录时启动、隐藏菜单栏图标、拖拽吸附、拖离时恢复大小、激活区缩放（百分比）和边缘宽度、预览填充色、窗口边距、贴紧屏幕边缘，以及 Restore to defaults（只恢复偏好、只恢复命令，或全部恢复）。登录时启动会写入 `~/Library/LaunchAgents/local.sash.plist`，下次登录生效。

**Commands** 按 Horizontal 和 Vertical 分组。每一行是目标区域的缩略图（特殊命令用图标）和名称。分隔线显示成一条短横条。分组标题可以折叠该组，**+** 用来添加命令或分隔线。选中一条命令后，右侧是它的编辑区：目标网格（上方和侧边标着占屏幕的比例，例如 ⁵⁄₈）、快捷键（开关和录制框；同一方向里发生冲突会提示并且不保存；Delete 清除，Esc 取消）、拖拽激活区域（开关和网格；拖动添加矩形，点击已有区域删除），以及是否显示在状态菜单里。选中的行上有两个按钮：菜单可见性和删除。内置命令不能删除。顶栏就是命令名，自建命令可以在那里改名。

修改会立即生效。设置窗口会改写 `config.toml` 和 `commands.json`；第一次修改命令时，如果还没有 `commands_file`，会创建它。每个文件第一次被改写之前，会复制为 `config.toml.bak` 或 `commands.json.bak`。**文件里的注释不会保留。** `ignored-apps.txt` 保持原样。

Linux 没有设置窗口。直接编辑这些文件，然后重新启动 Sash。

### GNOME Wayland

Wayland 不允许普通程序移动其他应用的窗口，也收不到全局按键，所以在 GNOME 上 Sash 借助一个 Shell 扩展：扩展负责移动窗口、抢快捷键、监听窗口拖动；吸附逻辑全在 `sash` 里，两者通过 D-Bus（`local.sash.Shell`）通信。

通过 Debian 包安装时，扩展已经装好并启用，不需要任何额外命令。自己编译的二进制可以运行一次：

```sh
sash install-extension   # 复制到 ~/.local/share/gnome-shell/extensions 并启用
```

`sash` 启动时发现扩展没有加载，也会自己做这一步，并弹出通知。若 Shell 里运行的扩展比 `sash` 要求的旧，它同样会通知。

Wayland 下 GNOME Shell 不能重启，所以第一次安装后要**注销再登录**，扩展才会加载（更新扩展后同样需要）。之后照常运行 `sash`。在 GNOME Wayland 会话里，Sash 会自动选用 GNOME 后端；登录时它最多等 30 秒，让 Shell 先把扩展加载起来；扩展仍然没有运行时，它会给出警告并退回 X11，此时只能管理 XWayland 窗口。可以用 `SASH_BACKEND=gnome` 或 `SASH_BACKEND=x11` 强制指定。扩展按 GNOME Shell 50 编写；其他版本需要在它的 `metadata.json` 里加上对应版本，且没有试过。

需要注意：

- GNOME 自己的 Ctrl+Alt+Left、Right、Up、Down 用来切换工作区，冲突时 GNOME 优先，所以日志会报这四个被拒绝，半屏快捷键不会生效。可以先释放它们，例如：

  ```sh
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-left  "['<Super>Page_Up', '<Super>KP_Prior', '<Super><Alt>Left']"
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-right "['<Super>Page_Down', '<Super>KP_Next', '<Super><Alt>Right']"
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-up    "[]"
  gsettings set org.gnome.desktop.wm.keybindings switch-to-workspace-down  "[]"
  ```

  或者在 `commands_file` 里改这几条命令的快捷键。快捷键只在正常模式下响应，概览和锁屏界面里不会触发。
- Ubuntu 自带 Tiling Assistant 扩展，它也可能对拖到屏幕边缘的窗口做平铺。要用拖拽吸附的话，请停用它（`gnome-extensions disable tiling-assistant@ubuntu.com`），并关闭 `org.gnome.mutter edge-tiling`，否则两者可能对同一次拖动都生效。
- 不能调整大小的窗口（很多对话框）只会被移动。
- 拖拽吸附跟随 GNOME 自己执行的窗口移动（拖标题栏，或 Super+拖动）。

- `sash` 运行时顶栏会显示 Sash 图标，退出后图标消失。菜单先列出当前窗口所在显示器适用的命令，然后是针对该应用的 **Ignore**、**Snap Windows by Dragging** 和 **Restore Original Size When Dragging Away** 两个开关、**Settings…**、**Reload Settings** 和 **Quit Sash**。重新加载失败时沿用旧设置，并弹出通知。

- **Settings…** 会打开设置窗口（也可以运行 `gnome-extensions prefs sash@sash.local`）。*General* 页是偏好、拖拽预览颜色、忽略的应用、**Open config.toml** 和 **Restore Preferences to Defaults**。*Landscape* 和 *Portrait* 页分别列出横屏和竖屏的命令：展开一条可以改快捷键（点 **Set…** 再按键）、是否出现在菜单里、后面是否有分隔线、目标区域和激活区域（在网格上拖选），也可以添加或删除自己的命令。改动立即生效，并写入 `config.toml` 和 `commands.json`（原文件各保留一份 `.bak`），所以 `config.toml` 里的注释不会保留。设置窗口需要 Sash 正在运行。GNOME 拒绝某个快捷键时，窗口会提示，这条命令需要换一个快捷键。

### Linux 开机自启

Debian 包会安装自启项。自己编译的二进制可以写成：

```ini
# ~/.config/autostart/sash.desktop
[Desktop Entry]
Type=Application
Name=Sash
Exec=/path/to/sash
```

## 开发

```sh
cargo test
./restart.sh              # 调试构建并运行；会先停掉本仓库里已有的 sash
./restart.sh --release
./restart.sh -- list      # -- 后面的参数会传给 sash
```

## 与 Magnet 的关系

行为参照 macOS 上的 Magnet 3.0.7。下面的结论来自对 `/Applications/Magnet.app` 的静态检查（REA 的 `inspect-native-dispatch-metadata`、`decode-interface-builder`、`inspect-signature`，以及应用资源和 Mach-O 字符串），设置窗口还参考了正在运行的 Magnet 的截图和辅助功能树。没有反编译 Magnet。Sash 不包含 Magnet 的代码。

直接观察到的：

- 命令定义在 `HorizontalCommands.json` 和 `VerticalCommands.json` 里。横屏是 24×12 网格，竖屏是 12×24 网格。每条命令有 `targetArea`、`activationArea` 和 Carbon 快捷键。[`assets/commands.json`](assets/commands.json) 沿用了这些坐标和默认快捷键。
- 特殊命令：下一个和上一个显示器、居中、还原、置顶。
- 偏好项包括 `snapWindows`、`restoreToOriginalSize`、`activationAreaEdgeWidth`、`activationAreaScale`、`highlightActivationAreas`、激活区和目标区的外观、`paddingAroundWindows`、`fitTightlyToScreenEdges`、绿色按钮菜单、应用黑名单、iCloud 同步。
- `MouseConductor` 的状态有 `idle`、`down`、`drag`、`skip`，并保存 `originalWindowFrame`。`WindowConductor` 保存 `originalFrames` 和 `expandedFrames`。
- `MainMenu.nib` 是应用主菜单。状态栏菜单由代码生成（`StatusMenu`、`MenuConductor`、`BlacklistConductor`、`onMenuOpen`）。命令 JSON 里每条都有 `visibleInMenuBar`，没有名称的分隔条目位于 Down、Bottom Right、Right Third（竖屏为 Bottom Third）、Right Two Thirds（竖屏为 Bottom Two Thirds）和 Previous Display 之后。`separator_after` 与这些位置对应。本地化字符串里有 “Ignore …”、“Not Authorized to Control Your Windows”、“Authorize…”、“Quit Magnet”、“Settings…”、“About Magnet”。

以下部分是推断的，没有通过反编译 Magnet 验证：

- 边缘激活区只在距离屏幕边缘 `activation_edge_width` 像素以内生效。内部激活区按 `activation_area_scale` 缩放。
- 相邻的吸附窗口之间留一个 `padding`。靠屏幕边缘的一侧也是完整的 `padding`，除非设置了 `fit_tightly_to_screen_edges`。
- 各项默认值（边缘 6 px、边距 0、预览颜色）是 Sash 自己定的。没有还原出 Magnet 的默认值。
- 菜单项的顺序、显示条件，以及 Ignore 的具体行为，是根据字符串和命令表推断的，没有观察 Magnet 实际弹出的菜单。命令表随显示器方向切换、忽略名单单独放在文本文件里、Settings 打开配置文件，这些是 Sash 自己的做法。设置窗口的页面名称、文案和类名（`SettingsWindowController`、`SettingsSidebarViewController`、`SettingsContentViewController`）来自静态检查。版式来自 Magnet 设置窗口的截图和辅助功能树（70 pt 侧栏、275 pt 列表、530 pt 详情、52 pt 顶栏、30 pt 行高、486 pt 宽的卡片，24×12 网格每格 13 pt、间距 3 pt），以及资源目录里的图标名（`inspect-asset-catalog`：Commands、Preferences、add、chevron、remove、menu-bar 等）。深色取自截图的像素。浅色没有观察过，是 Sash 自己选的。一部分控件行为观察过，其余是按外观推断的。

没有实现的部分：菜单里 Stay On Top 的勾选状态、Help 链接、设置窗口里的绿色按钮菜单和 iCloud 同步（Linux 没有对应控件）、激活区高亮、目标区外观菜单和轮廓（Sash 只有一个填充色）、Magnet 用预设格子选择激活区的方式（Sash 是自由拖出矩形）、区域与其他命令冲突时的确认，以及选中行上的第三个按钮（绿色按钮菜单是否可见）。
