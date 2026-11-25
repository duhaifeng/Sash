use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::geometry::{ActivationTuning, Spacing};
use crate::model::CommandSet;

/// User preferences read from `~/.config/sash/config.toml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Snap windows by dragging them to activation areas.
    pub snap_windows: bool,
    /// Dragging a snapped window away restores its size from before snapping.
    pub restore_to_original_size: bool,
    /// Thickness in pixels of activation areas on screen edges.
    pub activation_edge_width: i32,
    /// Scale of activation areas that do not touch a screen edge.
    pub activation_area_scale: f64,
    /// Margin in pixels around snapped windows.
    pub padding: i32,
    /// Do not apply the margin along screen edges.
    pub fit_tightly_to_screen_edges: bool,
    /// Keep the menu bar icon out of the menu bar (macOS). Opening the app again
    /// shows the settings window.
    pub hide_menu_bar_icon: bool,
    pub highlight: Highlight,
    /// WM_CLASS instance or class names of applications Sash leaves alone.
    pub ignored_apps: Vec<String>,
    /// Command definitions replacing the built-in set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands_file: Option<PathBuf>,
    /// Applications ignored from the status menu, one identifier per line in
    /// `ignored-apps.txt` beside the preferences file.
    #[serde(skip)]
    pub user_ignored: Vec<String>,
    /// The preferences file that was read, if any.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Highlight {
    /// Fill colour as `#RRGGBBAA`; needs a compositing manager.
    pub fill: Color,
    pub outline: Color,
    pub outline_width: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            snap_windows: true,
            restore_to_original_size: true,
            activation_edge_width: 6,
            activation_area_scale: 1.0,
            padding: 0,
            fit_tightly_to_screen_edges: false,
            hide_menu_bar_icon: false,
            highlight: Highlight::default(),
            ignored_apps: Vec::new(),
            commands_file: None,
            user_ignored: Vec::new(),
            path: None,
        }
    }
}

impl Default for Highlight {
    fn default() -> Self {
        Self {
            fill: Color {
                r: 0x3d,
                g: 0x8b,
                b: 0xfd,
                a: 0x55,
            },
            outline: Color {
                r: 0x3d,
                g: 0x8b,
                b: 0xfd,
                a: 0xdd,
            },
            outline_width: 3,
        }
    }
}

impl Config {
    pub const EXAMPLE: &'static str = include_str!("../assets/config.example.toml");

    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("sash").join("config.toml"))
    }

    /// Writes the preferences to the file they were read from, or the default
    /// location. An existing file is first kept once as `<name>.bak`, because
    /// comments are not preserved. Nothing changes when the write fails.
    pub fn save(&mut self) -> anyhow::Result<()> {
        let file = self
            .path
            .clone()
            .or_else(Self::default_path)
            .context("no preferences location")?;
        let text = toml::to_string_pretty(self).context("serialising preferences")?;
        write_with_backup(
            &file,
            &format!(
                "# Written by Sash when a setting changes; comments are not kept.\n\
                 # The previous file is saved once as {}.bak.\n{text}",
                file.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            ),
        )?;
        self.path = Some(file);
        Ok(())
    }

    /// Returns every preference to its default. The preferences location,
    /// command file and ignored applications stay as they are.
    pub fn restore_preferences(&mut self) {
        *self = Self {
            ignored_apps: std::mem::take(&mut self.ignored_apps),
            commands_file: self.commands_file.take(),
            user_ignored: std::mem::take(&mut self.user_ignored),
            path: self.path.take(),
            ..Self::default()
        };
    }

    /// Sets one preference by its name in `config.toml` (`padding`, or
    /// `highlight.fill` for a nested one) from a JSON value. The preferences are
    /// checked as a whole and stay unchanged when the value is not valid.
    pub fn set_preference(&mut self, key: &str, value: serde_json::Value) -> anyhow::Result<()> {
        let mut tree = serde_json::to_value(&*self).context("reading preferences")?;
        let slot = match key.split_once('.') {
            Some((section, name)) => tree.get_mut(section).and_then(|node| node.get_mut(name)),
            None => tree.get_mut(key),
        };
        match slot {
            Some(slot) if key != "commands_file" => *slot = value,
            _ => bail!("unknown preference `{key}`"),
        }
        let mut next: Self =
            serde_json::from_value(tree).with_context(|| format!("invalid value for `{key}`"))?;
        next.validate()?;
        next.commands_file = self.commands_file.take();
        next.user_ignored = std::mem::take(&mut self.user_ignored);
        next.path = self.path.take();
        *self = next;
        Ok(())
    }

    /// Writes the commands file, returning whether `commands_file` was newly set
    /// (the preferences then need saving too). Nothing is written while the
    /// commands are the built-in ones and no file is in use.
    pub fn save_commands(&mut self, commands: &CommandSet) -> anyhow::Result<bool> {
        let json = commands.to_json();
        let (file, newly_set) = match &self.commands_file {
            Some(file) => (file.clone(), false),
            None if json == CommandSet::builtin().to_json() => return Ok(false),
            None => {
                let preferences = self
                    .path
                    .clone()
                    .or_else(Self::default_path)
                    .context("no preferences location")?;
                (preferences.with_file_name("commands.json"), true)
            }
        };
        write_with_backup(&file, &json)?;
        self.commands_file = Some(file);
        Ok(newly_set)
    }

    /// Loads `path`, or the default location when it exists; otherwise defaults.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let mut config = Self::load_preferences(path)?;
        let base = config.path.clone().or_else(Self::default_path);
        config.user_ignored = base
            .map(|base| Self::ignored_apps_path(&base))
            .map(|file| read_ignored_apps(&file))
            .unwrap_or_default();
        Ok(config)
    }

    fn load_preferences(path: Option<&Path>) -> anyhow::Result<Self> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => match Self::default_path().filter(|p| p.exists()) {
                Some(p) => p,
                None => return Ok(Self::default()),
            },
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut config: Self =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.validate()?;
        config.path = Some(path.clone());
        if let Some(file) = &config.commands_file
            && file.is_relative()
        {
            config.commands_file = path.parent().map(|dir| dir.join(file));
        }
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !(1..=200).contains(&self.activation_edge_width) {
            bail!("activation_edge_width must be between 1 and 200 pixels");
        }
        if !(0.1..=3.0).contains(&self.activation_area_scale) {
            bail!("activation_area_scale must be between 0.1 and 3.0");
        }
        if !(0..=200).contains(&self.padding) {
            bail!("padding must be between 0 and 200 pixels");
        }
        Ok(())
    }

    pub fn commands(&self) -> anyhow::Result<CommandSet> {
        match &self.commands_file {
            None => Ok(CommandSet::builtin()),
            Some(file) => {
                let json = std::fs::read_to_string(file)
                    .with_context(|| format!("reading {}", file.display()))?;
                CommandSet::parse(&json).with_context(|| format!("parsing {}", file.display()))
            }
        }
    }

    pub fn spacing(&self) -> Spacing {
        Spacing {
            padding: self.padding,
            fit_tightly_to_screen_edges: self.fit_tightly_to_screen_edges,
        }
    }

    pub fn activation(&self) -> ActivationTuning {
        ActivationTuning {
            edge_width: self.activation_edge_width,
            interior_scale: self.activation_area_scale,
        }
    }

    /// Where the menu's ignored applications are kept for a preferences file.
    pub fn ignored_apps_path(preferences: &Path) -> PathBuf {
        preferences.with_file_name("ignored-apps.txt")
    }

    pub fn ignores(&self, instance: &str, class: &str) -> bool {
        self.ignored_by_preferences(instance, class) || self.ignored_by_menu(instance, class)
    }

    /// Ignored through `ignored_apps`, which only the preferences file edits.
    pub fn ignored_by_preferences(&self, instance: &str, class: &str) -> bool {
        matches_any(&self.ignored_apps, instance, class)
    }

    /// Ignored through the status menu.
    pub fn ignored_by_menu(&self, instance: &str, class: &str) -> bool {
        matches_any(&self.user_ignored, instance, class)
    }

    /// Adds or removes `app` from the menu's ignored applications and saves
    /// the list, leaving it unchanged when the file cannot be written.
    pub fn set_ignored_by_menu(&mut self, app: &str, ignored: bool) -> anyhow::Result<()> {
        let mut next: Vec<String> = self
            .user_ignored
            .iter()
            .filter(|entry| !entry.eq_ignore_ascii_case(app))
            .cloned()
            .collect();
        if ignored {
            next.push(app.to_string());
        }
        let preferences = self
            .path
            .clone()
            .or_else(Self::default_path)
            .context("no preferences location")?;
        let file = Self::ignored_apps_path(&preferences);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let mut text = next.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        let temporary = file.with_extension("txt.tmp");
        std::fs::write(&temporary, text)
            .and_then(|()| std::fs::rename(&temporary, &file))
            .with_context(|| format!("writing {}", file.display()))?;
        self.user_ignored = next;
        Ok(())
    }
}

/// Replaces `file` atomically after keeping its previous content once as
/// `<file>.bak`.
pub fn write_with_backup(file: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if file.exists() {
        let mut name = file.as_os_str().to_owned();
        name.push(".bak");
        let backup = PathBuf::from(name);
        if !backup.exists() {
            std::fs::copy(file, &backup)
                .with_context(|| format!("backing up to {}", backup.display()))?;
        }
    }
    let mut temporary = file.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, file))
        .with_context(|| format!("writing {}", file.display()))
}

fn matches_any(apps: &[String], instance: &str, class: &str) -> bool {
    apps.iter()
        .any(|app| app.eq_ignore_ascii_case(instance) || app.eq_ignore_ascii_case(class))
}

fn read_ignored_apps(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// Premultiplied ARGB pixel for 32-bit TrueColor visuals.
    #[allow(dead_code)]
    pub fn argb_premultiplied(self) -> u32 {
        let mul = |c: u8| u32::from(c) * u32::from(self.a) / 255;
        u32::from(self.a) << 24 | mul(self.r) << 16 | mul(self.g) << 8 | mul(self.b)
    }

    #[allow(dead_code)]
    pub fn rgb(self) -> u32 {
        u32::from(self.r) << 16 | u32::from(self.g) << 8 | u32::from(self.b)
    }
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            self.r, self.g, self.b, self.a
        ))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let hex = text.strip_prefix('#').unwrap_or(&text);
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("??"), 16);
        let parsed = match hex.len() {
            6 => (byte(0), byte(2), byte(4), Ok(0xff)),
            8 => (byte(0), byte(2), byte(4), byte(6)),
            _ => {
                return Err(serde::de::Error::custom(format!(
                    "colour `{text}` is not #RRGGBB or #RRGGBBAA"
                )));
            }
        };
        match parsed {
            (Ok(r), Ok(g), Ok(b), Ok(a)) => Ok(Color { r, g, b, a }),
            _ => Err(serde::de::Error::custom(format!(
                "colour `{text}` is not hexadecimal"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sets_one_preference_by_name() {
        let mut config = Config {
            user_ignored: vec!["firefox".into()],
            path: Some("/tmp/sash/config.toml".into()),
            ..Config::default()
        };
        config.set_preference("padding", json!(12)).unwrap();
        config
            .set_preference("highlight.fill", json!("#11223344"))
            .unwrap();
        config.set_preference("snap_windows", json!(false)).unwrap();
        config
            .set_preference("ignored_apps", json!(["Terminal"]))
            .unwrap();
        assert_eq!(config.padding, 12);
        assert_eq!(config.highlight.fill.a, 0x44);
        assert!(!config.snap_windows);
        assert_eq!(config.ignored_apps, ["Terminal"]);
        // What is not a preference of the file survives.
        assert_eq!(config.user_ignored, ["firefox"]);
        assert!(config.path.is_some());
    }

    #[test]
    fn rejects_bad_preferences_and_keeps_the_old_ones() {
        let mut config = Config::default();
        assert!(config.set_preference("padding", json!(500)).is_err());
        assert!(config.set_preference("padding", json!("wide")).is_err());
        assert!(config.set_preference("nonsense", json!(1)).is_err());
        assert!(config.set_preference("highlight.nope", json!(1)).is_err());
        assert!(config.set_preference("commands_file", json!("x")).is_err());
        assert!(
            config
                .set_preference("highlight.fill", json!("red"))
                .is_err()
        );
        assert_eq!(config.padding, 0);
        assert!(config.commands_file.is_none());
    }

    #[test]
    fn example_config_matches_defaults() {
        let example: Config = toml::from_str(Config::EXAMPLE).unwrap();
        example.validate().unwrap();
        let defaults = Config::default();
        assert_eq!(
            example.activation_edge_width,
            defaults.activation_edge_width
        );
        assert_eq!(example.padding, defaults.padding);
        assert_eq!(example.highlight.fill, defaults.highlight.fill);
        assert_eq!(example.highlight.outline, defaults.highlight.outline);
    }

    #[test]
    fn serialised_preferences_load_again() {
        let mut config = Config {
            padding: 12,
            hide_menu_bar_icon: true,
            ignored_apps: vec!["Terminal".into()],
            commands_file: Some(PathBuf::from("/tmp/commands.json")),
            ..Config::default()
        };
        config.highlight.outline_width = 5;
        let text = toml::to_string_pretty(&config).unwrap();
        let again: Config = toml::from_str(&text).unwrap();
        assert_eq!(again.padding, 12);
        assert!(again.hide_menu_bar_icon);
        assert_eq!(again.highlight.outline_width, 5);
        assert_eq!(again.highlight.fill, config.highlight.fill);
        assert_eq!(again.ignored_apps, ["Terminal"]);
        assert_eq!(again.commands_file, config.commands_file);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restoring_preferences_keeps_the_files_they_belong_to() {
        let mut config = Config {
            padding: 12,
            ignored_apps: vec!["Terminal".into()],
            commands_file: Some(PathBuf::from("/tmp/commands.json")),
            path: Some(PathBuf::from("/tmp/config.toml")),
            ..Config::default()
        };
        config.restore_preferences();
        assert_eq!(config.padding, 0);
        assert_eq!(config.ignored_apps, ["Terminal"]);
        assert!(config.commands_file.is_some() && config.path.is_some());
    }

    #[test]
    fn parses_colours_and_rejects_unknown_keys() {
        let config: Config = toml::from_str("[highlight]\nfill = \"#ff000080\"").unwrap();
        assert_eq!(config.highlight.fill.argb_premultiplied(), 0x8080_0000);
        assert!(toml::from_str::<Config>("[highlight]\nfill = \"red\"").is_err());
        assert!(toml::from_str::<Config>("paddding = 4").is_err());
    }
}
