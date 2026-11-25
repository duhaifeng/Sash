mod config;
mod daemon;
mod geometry;
#[cfg(not(target_os = "macos"))]
mod gnome;
#[cfg(not(target_os = "macos"))]
mod instance;
#[cfg(target_os = "macos")]
mod macos;
mod memory;
mod model;
mod session;
mod shortcut;
#[cfg(not(target_os = "macos"))]
mod x11;

use std::path::PathBuf;
#[cfg(not(target_os = "macos"))]
use std::time::Duration;

/// How long a background Sash waits for the GNOME Shell extension to appear.
#[cfg(not(target_os = "macos"))]
const GNOME_EXTENSION_WAIT: Duration = Duration::from_secs(30);

use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::model::{Action, Axis};

/// Snap windows by shortcut or by dragging. Linux uses X11, or a GNOME Shell
/// extension on Wayland; macOS uses Accessibility.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Preferences file (default: ~/.config/sash/config.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run in the background: global shortcuts and drag-to-snap (default).
    Run,
    /// Apply one command to the active window, e.g. `sash apply left-third`.
    Apply { name: String },
    /// List commands with their shortcuts.
    List,
    /// Print the built-in command definitions as JSON for `commands_file`.
    PrintCommands,
    /// Print an example preferences file.
    PrintConfig,
    /// Install and enable the GNOME Shell extension Sash needs on Wayland.
    #[cfg(not(target_os = "macos"))]
    InstallExtension,
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Cmd::Run) {
        Cmd::PrintCommands => print!("{}", model::CommandSet::BUILTIN_JSON),
        Cmd::PrintConfig => print!("{}", Config::EXAMPLE),
        Cmd::List => {
            let commands = Config::load(cli.config.as_deref())?.commands()?;
            for axis in [Axis::Horizontal, Axis::Vertical] {
                println!("{axis:?} monitors:");
                for command in commands.for_axis(axis) {
                    let shortcut = command
                        .shortcut
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let drag = match command.action {
                        Action::Area(_) if !command.activation.is_empty() => "drag",
                        _ => "",
                    };
                    println!("  {:<20} {:<22} {drag}", command.name, shortcut);
                }
            }
        }
        Cmd::Apply { name } => {
            let config = Config::load(cli.config.as_deref())?;
            #[cfg(target_os = "macos")]
            {
                let session = macos::MacSession::connect(&config)?;
                daemon::apply_command(session, config, &name)?;
            }
            #[cfg(not(target_os = "macos"))]
            match gnome::session(&config, Duration::ZERO)? {
                Some(session) => daemon::apply_command(session, config, &name)?,
                None => {
                    let session = x11::X11Session::connect(&config)?;
                    daemon::apply_command(session, config, &name)?;
                }
            }
        }
        Cmd::Run => {
            let config = Config::load(cli.config.as_deref())?;
            #[cfg(target_os = "macos")]
            macos::launch(config)?;
            #[cfg(not(target_os = "macos"))]
            {
                let Some(_instance) = instance::acquire()? else {
                    log::info!("Sash is already running; leaving it as it is");
                    return Ok(());
                };
                // At login Sash can start before the Shell has loaded its extension.
                match gnome::session(&config, GNOME_EXTENSION_WAIT)? {
                    Some(session) => gnome::launch(session, config)?,
                    None => x11::launch(config)?,
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        Cmd::InstallExtension => gnome::install_extension()?,
    }
    Ok(())
}
