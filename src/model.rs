use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::geometry::GridRect;
use crate::shortcut::Shortcut;

/// Command list selected by monitor orientation: landscape monitors use a
/// 24×12 grid and portrait monitors a 12×24 grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    pub const fn grid(self) -> (u32, u32) {
        match self {
            Axis::Horizontal => (24, 12),
            Axis::Vertical => (12, 24),
        }
    }

    pub fn for_size(width: i32, height: i32) -> Axis {
        if width >= height {
            Axis::Horizontal
        } else {
            Axis::Vertical
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Special {
    NextDisplay,
    PreviousDisplay,
    Center,
    Restore,
    StayOnTop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Area(GridRect),
    Special(Special),
}

#[derive(Clone, Debug)]
pub struct Command {
    /// Identifies the command while the set is edited; not stored in files.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub id: u64,
    pub name: String,
    pub axis: Axis,
    pub action: Action,
    pub activation: Vec<GridRect>,
    pub shortcut: Option<Shortcut>,
    /// Menus draw a divider after this command, as Magnet's command lists do.
    pub separator_after: bool,
    /// Listed in the status menu.
    pub in_menu: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommandSet {
    horizontal: Vec<RawCommand>,
    vertical: Vec<RawCommand>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<[[u32; 2]; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    special: Option<Special>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    activation: Vec<[[u32; 2]; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shortcut: Option<String>,
    #[serde(default = "enabled_by_default", skip_serializing)]
    enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    separator_after: bool,
    /// Whether the status menu lists the command.
    #[serde(default = "enabled_by_default", skip_serializing_if = "is_true")]
    menu_bar: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn is_true(value: &bool) -> bool {
    *value
}

fn enabled_by_default() -> bool {
    true
}

impl RawCommand {
    fn parse(self, axis: Axis, id: u64) -> anyhow::Result<Command> {
        let action = match (self.target, self.special) {
            (Some(target), None) => {
                let target = GridRect::from(target);
                if !target.fits(axis) {
                    bail!("target {target:?} lies outside the {axis:?} grid");
                }
                Action::Area(target)
            }
            (None, Some(special)) => Action::Special(special),
            _ => bail!("exactly one of `target` or `special` is required"),
        };
        let activation = self
            .activation
            .into_iter()
            .map(GridRect::from)
            .map(|segment| {
                if segment.fits(axis) {
                    Ok(segment)
                } else {
                    bail!("activation {segment:?} lies outside the {axis:?} grid")
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        if !activation.is_empty() && matches!(action, Action::Special(_)) {
            bail!("special commands have no target to snap to by dragging");
        }
        let shortcut = self.shortcut.as_deref().map(str::parse).transpose()?;
        Ok(Command {
            id,
            name: self.name,
            axis,
            action,
            activation,
            shortcut,
            separator_after: self.separator_after,
            in_menu: self.menu_bar,
        })
    }
}

#[derive(Clone)]
pub struct CommandSet {
    commands: Vec<Command>,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    next_id: u64,
}

impl Command {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn to_raw(&self) -> RawCommand {
        let (target, special) = match self.action {
            Action::Area(grid) => (Some(grid.into()), None),
            Action::Special(special) => (None, Some(special)),
        };
        RawCommand {
            name: self.name.clone(),
            target,
            special,
            activation: self.activation.iter().map(|&g| g.into()).collect(),
            shortcut: self.shortcut.as_ref().map(ToString::to_string),
            enabled: true,
            separator_after: self.separator_after,
            menu_bar: self.in_menu,
        }
    }
}

impl CommandSet {
    pub const BUILTIN_JSON: &'static str = include_str!("../assets/commands.json");

    pub fn builtin() -> Self {
        Self::parse(Self::BUILTIN_JSON).expect("built-in commands are valid")
    }

    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let raw: RawCommandSet = serde_json::from_str(json)?;
        let mut commands = Vec::new();
        let mut next_id = 0;
        for (axis, list) in [
            (Axis::Horizontal, raw.horizontal),
            (Axis::Vertical, raw.vertical),
        ] {
            for command in list.into_iter().filter(|c| c.enabled) {
                let name = command.name.clone();
                let command = command
                    .parse(axis, next_id)
                    .with_context(|| format!("{axis:?} command `{name}`"))?;
                if let Some(shortcut) = &command.shortcut
                    && let Some(other) = commands.iter().find(|c: &&Command| {
                        c.axis == axis && c.shortcut.as_ref() == Some(shortcut)
                    })
                {
                    bail!(
                        "{axis:?} commands `{}` and `{name}` share shortcut {shortcut}",
                        other.name
                    );
                }
                commands.push(command);
                next_id += 1;
            }
        }
        Ok(Self { commands, next_id })
    }

    pub fn for_axis(&self, axis: Axis) -> impl Iterator<Item = &Command> {
        self.commands.iter().filter(move |c| c.axis == axis)
    }

    /// Distinct shortcuts across both axes, in declaration order.
    pub fn shortcuts(&self) -> Vec<&Shortcut> {
        let mut out: Vec<&Shortcut> = Vec::new();
        for shortcut in self.commands.iter().filter_map(|c| c.shortcut.as_ref()) {
            if !out.contains(&shortcut) {
                out.push(shortcut);
            }
        }
        out
    }

    /// The command bound to `shortcut` for a monitor with this axis. Special
    /// commands declared only for the other axis still apply, since they do
    /// not depend on the grid.
    pub fn by_shortcut(&self, axis: Axis, shortcut: &Shortcut) -> Option<&Command> {
        self.lookup(axis, |c| c.shortcut.as_ref() == Some(shortcut))
    }

    /// Case-insensitive lookup that also accepts `left-third` for `Left Third`.
    pub fn by_name(&self, axis: Axis, name: &str) -> Option<&Command> {
        let wanted = normalise(name);
        self.lookup(axis, |c| normalise(&c.name) == wanted)
    }

    fn lookup(&self, axis: Axis, matches: impl Fn(&Command) -> bool) -> Option<&Command> {
        self.for_axis(axis).find(|c| matches(c)).or_else(|| {
            self.commands
                .iter()
                .find(|c| c.axis != axis && matches!(c.action, Action::Special(_)) && matches(c))
        })
    }
}

/// The editing API the macOS settings window uses.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl CommandSet {
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Positions of `axis` commands in `commands()`, in list order.
    pub fn indices(&self, axis: Axis) -> Vec<usize> {
        (0..self.commands.len())
            .filter(|&i| self.commands[i].axis == axis)
            .collect()
    }

    /// Commands that ship with the program keep their names and cannot be removed.
    pub fn is_builtin(&self, index: usize) -> bool {
        static NAMES: std::sync::OnceLock<Vec<(Axis, String)>> = std::sync::OnceLock::new();
        let names = NAMES.get_or_init(|| {
            CommandSet::builtin()
                .commands
                .iter()
                .map(|c| (c.axis, c.name.clone()))
                .collect()
        });
        let command = &self.commands[index];
        names
            .iter()
            .any(|(axis, name)| *axis == command.axis && *name == command.name)
    }

    /// JSON in the shape `commands_file` expects, one command per line.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        for (n, (key, axis)) in [
            ("horizontal", Axis::Horizontal),
            ("vertical", Axis::Vertical),
        ]
        .into_iter()
        .enumerate()
        {
            out.push_str(&format!("  \"{key}\": [\n"));
            let lines: Vec<String> = self
                .for_axis(axis)
                .map(|c| {
                    let json = serde_json::to_string(&c.to_raw()).expect("plain data serialises");
                    format!("    {json}")
                })
                .collect();
            out.push_str(&lines.join(",\n"));
            if !lines.is_empty() {
                out.push('\n');
            }
            out.push_str(if n == 0 { "  ],\n" } else { "  ]\n" });
        }
        out.push_str("}\n");
        out
    }

    /// The commands for the Shell extension's settings page: both orientations,
    /// every field spelled out, and whether the command ships with the program.
    pub fn editor_json(&self) -> serde_json::Value {
        let list = |axis: Axis| -> Vec<serde_json::Value> {
            self.commands
                .iter()
                .enumerate()
                .filter(|(_, command)| command.axis == axis)
                .map(|(index, command)| {
                    let raw = command.to_raw();
                    serde_json::json!({
                        "name": raw.name,
                        "target": raw.target,
                        "special": raw.special,
                        "activation": raw.activation,
                        "shortcut": raw.shortcut,
                        "separator_after": raw.separator_after,
                        "menu_bar": raw.menu_bar,
                        "builtin": self.is_builtin(index),
                    })
                })
                .collect()
        };
        serde_json::json!({
            "horizontal": list(Axis::Horizontal),
            "vertical": list(Axis::Vertical),
        })
    }

    /// Checks a set that replaces the current one: names must be distinct and
    /// the built-in commands must all still be there.
    pub fn check_replacement(&self) -> anyhow::Result<()> {
        for (index, command) in self.commands.iter().enumerate() {
            if normalise(&command.name).is_empty() {
                bail!("a command needs a name with letters or digits");
            }
            if self.name_taken(command.axis, &command.name, Some(index)) {
                bail!(
                    "another {:?} command already uses the name `{}`",
                    command.axis,
                    command.name
                );
            }
        }
        for builtin in &Self::builtin().commands {
            if !self
                .commands
                .iter()
                .any(|c| c.axis == builtin.axis && c.name == builtin.name)
            {
                bail!("the built-in command `{}` can't be removed", builtin.name);
            }
        }
        Ok(())
    }

    /// Appends a command with a centred half-screen target and returns its index.
    pub fn add_command(&mut self, axis: Axis) -> usize {
        let (cols, rows) = axis.grid();
        let mut name = String::from("Command");
        let mut count = 1;
        while self.name_taken(axis, &name, None) {
            count += 1;
            name = format!("Command {count}");
        }
        let command = Command {
            id: self.next_id,
            name,
            axis,
            action: Action::Area(GridRect {
                x: cols / 4,
                y: rows / 4,
                w: cols / 2,
                h: rows / 2,
            }),
            activation: Vec::new(),
            shortcut: None,
            separator_after: false,
            in_menu: true,
        };
        self.next_id += 1;
        let at = self.commands.iter().rposition(|c| c.axis == axis).map_or(
            if axis == Axis::Horizontal {
                0
            } else {
                self.commands.len()
            },
            |i| i + 1,
        );
        self.commands.insert(at, command);
        at
    }

    /// Puts a divider after the last command of `axis`; false when it has one already.
    pub fn add_divider(&mut self, axis: Axis) -> bool {
        match self.commands.iter_mut().rfind(|c| c.axis == axis) {
            Some(last) if !last.separator_after => {
                last.separator_after = true;
                true
            }
            _ => false,
        }
    }

    pub fn set_separator_after(&mut self, index: usize, divider: bool) {
        self.commands[index].separator_after = divider;
    }

    pub fn set_in_menu(&mut self, index: usize, visible: bool) {
        self.commands[index].in_menu = visible;
    }

    /// Removes a command added by the user. A divider that followed it moves up.
    pub fn remove(&mut self, index: usize) -> anyhow::Result<()> {
        if self.is_builtin(index) {
            bail!("built-in commands can't be removed");
        }
        let removed = self.commands.remove(index);
        if removed.separator_after
            && let Some(previous) = self.commands[..index]
                .iter_mut()
                .rfind(|c| c.axis == removed.axis)
        {
            previous.separator_after = true;
        }
        Ok(())
    }

    pub fn set_name(&mut self, index: usize, name: &str) -> anyhow::Result<()> {
        let name = name.trim();
        if self.is_builtin(index) {
            bail!("built-in commands can't be renamed");
        }
        if normalise(name).is_empty() {
            bail!("a command needs a name with letters or digits");
        }
        let axis = self.commands[index].axis;
        if self.name_taken(axis, name, Some(index)) {
            bail!("another {axis:?} command already uses this name");
        }
        self.commands[index].name = name.to_string();
        Ok(())
    }

    /// Binds or clears the shortcut; fails when another command of the same
    /// axis owns it.
    pub fn set_shortcut(&mut self, index: usize, shortcut: Option<Shortcut>) -> anyhow::Result<()> {
        if let Some(shortcut) = &shortcut {
            let axis = self.commands[index].axis;
            if let Some(other) = self.commands.iter().enumerate().find(|(i, c)| {
                *i != index && c.axis == axis && c.shortcut.as_ref() == Some(shortcut)
            }) {
                bail!("{shortcut} is taken by `{}`", other.1.name);
            }
        }
        self.commands[index].shortcut = shortcut;
        Ok(())
    }

    pub fn set_target(&mut self, index: usize, target: GridRect) -> anyhow::Result<()> {
        let command = &mut self.commands[index];
        if !matches!(command.action, Action::Area(_)) || !target.fits(command.axis) {
            bail!("{target:?} is not an area of the {:?} grid", command.axis);
        }
        command.action = Action::Area(target);
        Ok(())
    }

    pub fn set_activation(&mut self, index: usize, areas: Vec<GridRect>) -> anyhow::Result<()> {
        let command = &mut self.commands[index];
        if !matches!(command.action, Action::Area(_)) || !areas.iter().all(|a| a.fits(command.axis))
        {
            bail!("activation areas do not fit the {:?} grid", command.axis);
        }
        command.activation = areas;
        Ok(())
    }

    fn name_taken(&self, axis: Axis, name: &str, except: Option<usize>) -> bool {
        let wanted = normalise(name);
        self.commands
            .iter()
            .enumerate()
            .any(|(i, c)| Some(i) != except && c.axis == axis && normalise(&c.name) == wanted)
    }
}

fn normalise(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_commands_cover_both_axes() {
        let set = CommandSet::builtin();
        assert_eq!(set.for_axis(Axis::Horizontal).count(), 20);
        assert_eq!(set.for_axis(Axis::Vertical).count(), 19);

        let left = set.by_name(Axis::Vertical, "left").unwrap();
        assert_eq!(
            left.action,
            Action::Area(GridRect {
                x: 0,
                y: 0,
                w: 6,
                h: 24
            })
        );

        let top_third = set.by_name(Axis::Vertical, "top-third").unwrap();
        assert_eq!(
            top_third.shortcut.as_ref().unwrap().to_string(),
            "Ctrl+Alt+D"
        );
    }

    #[test]
    fn menu_dividers_follow_magnets_groups() {
        let set = CommandSet::builtin();
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let before_divider: Vec<&str> = set
                .for_axis(axis)
                .filter(|c| c.separator_after)
                .map(|c| c.name.as_str())
                .collect();
            assert_eq!(before_divider.len(), 5);
            assert_eq!(before_divider[0], "Bottom");
            assert_eq!(before_divider[4], "Previous Display");
        }
    }

    #[test]
    fn specials_fall_back_to_the_other_axis() {
        let set = CommandSet::builtin();
        let shortcut: Shortcut = "Ctrl+Alt+Super+Up".parse().unwrap();
        let command = set.by_shortcut(Axis::Vertical, &shortcut).unwrap();
        assert_eq!(command.action, Action::Special(Special::StayOnTop));
        assert!(set.by_name(Axis::Vertical, "left third").is_none());
    }

    #[test]
    fn rejects_invalid_definitions() {
        let outside = r#"{"horizontal":[{"name":"x","target":[[20,0],[8,12]]}],"vertical":[]}"#;
        assert!(CommandSet::parse(outside).is_err());
        let both = r#"{"horizontal":[{"name":"x","target":[[0,0],[1,1]],"special":"center"}],"vertical":[]}"#;
        assert!(CommandSet::parse(both).is_err());
        let duplicate = r#"{"horizontal":[
            {"name":"a","special":"center","shortcut":"Ctrl+C"},
            {"name":"b","special":"restore","shortcut":"ctrl+c"}],"vertical":[]}"#;
        assert!(CommandSet::parse(duplicate).is_err());
        let disabled = r#"{"horizontal":[{"name":"x","target":[[20,0],[8,12]],"enabled":false}],"vertical":[]}"#;
        assert_eq!(
            CommandSet::parse(disabled)
                .unwrap()
                .for_axis(Axis::Horizontal)
                .count(),
            0
        );
    }

    #[test]
    fn serialises_back_to_the_same_commands() {
        let set = CommandSet::builtin();
        let again = CommandSet::parse(&set.to_json()).unwrap();
        assert_eq!(again.to_json(), set.to_json());
        assert_eq!(again.commands().len(), 39);
    }

    #[test]
    fn edits_commands_like_the_settings_window() {
        let mut set = CommandSet::builtin();
        let index = set.add_command(Axis::Vertical);
        assert_eq!(set.commands()[index].name, "Command");
        assert!(!set.is_builtin(index));
        assert_eq!(set.add_command(Axis::Vertical), index + 1);
        assert_eq!(set.commands()[index + 1].name, "Command 2");

        set.set_name(index, "Reading Pane").unwrap();
        assert!(set.set_name(index + 1, "reading-pane").is_err());
        let grid = GridRect {
            x: 0,
            y: 0,
            w: 4,
            h: 24,
        };
        set.set_target(index, grid).unwrap();
        assert!(
            set.set_target(
                index,
                GridRect {
                    x: 10,
                    y: 0,
                    w: 4,
                    h: 24
                }
            )
            .is_err()
        );
        set.set_activation(
            index,
            vec![GridRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            }],
        )
        .unwrap();

        let taken: Shortcut = "Ctrl+Alt+Left".parse().unwrap();
        assert!(set.set_shortcut(index, Some(taken)).is_err());
        let free: Shortcut = "Ctrl+Alt+Shift+R".parse().unwrap();
        set.set_shortcut(index, Some(free.clone())).unwrap();

        let reloaded = CommandSet::parse(&set.to_json()).unwrap();
        let saved = reloaded.by_name(Axis::Vertical, "reading pane").unwrap();
        assert_eq!(saved.action, Action::Area(grid));
        assert_eq!(saved.shortcut.as_ref(), Some(&free));

        assert!(set.remove(0).is_err());
        set.remove(index).unwrap();
        assert!(set.by_name(Axis::Vertical, "reading pane").is_none());
    }

    #[test]
    fn dividers_move_up_when_their_command_is_removed() {
        let mut set = CommandSet::builtin();
        let index = set.add_command(Axis::Horizontal);
        assert!(set.add_divider(Axis::Horizontal));
        assert!(!set.add_divider(Axis::Horizontal));
        set.remove(index).unwrap();
        let last = set.for_axis(Axis::Horizontal).last().unwrap();
        assert_eq!(last.name, "Stay On Top");
        assert!(last.separator_after);
    }

    #[test]
    fn editor_json_lists_every_field() {
        let json = CommandSet::builtin().editor_json();
        let left = &json["horizontal"][0];
        assert_eq!(left["name"], "Left");
        assert_eq!(left["target"], serde_json::json!([[0, 0], [12, 12]]));
        assert_eq!(left["special"], serde_json::Value::Null);
        assert_eq!(left["menu_bar"], true);
        assert_eq!(left["builtin"], true);
        assert_eq!(json["vertical"].as_array().unwrap().len(), 19);
        let center = json["horizontal"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Center")
            .unwrap();
        assert_eq!(center["special"], "center");
        assert_eq!(center["target"], serde_json::Value::Null);
    }

    #[test]
    fn replacements_keep_builtins_and_distinct_names() {
        let mut set = CommandSet::builtin();
        assert!(set.check_replacement().is_ok());
        let index = set.add_command(Axis::Horizontal);
        assert!(set.check_replacement().is_ok());
        set.commands[index].name = "left".into();
        assert!(set.check_replacement().is_err());
        set.commands[index].name = "!!".into();
        assert!(set.check_replacement().is_err());

        let mut without = CommandSet::builtin();
        without.commands.remove(0);
        assert!(without.check_replacement().is_err());
    }

    #[test]
    fn menu_visibility_round_trips() {
        let mut set = CommandSet::builtin();
        set.set_in_menu(0, false);
        let again = CommandSet::parse(&set.to_json()).unwrap();
        assert!(!again.commands()[0].in_menu);
        assert!(again.commands()[1].in_menu);
    }
}
