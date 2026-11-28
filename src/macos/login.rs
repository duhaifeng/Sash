//! "Add SnapGrid to login items": a LaunchAgent that starts this executable at login.

use std::path::PathBuf;

use anyhow::Context;

const LABEL: &str = "local.snapgrid";
/// LaunchAgent label from before the rename.
const PREVIOUS_LABEL: &str = "local.sash";

fn agent_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
    )
}

fn previous_agent_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{PREVIOUS_LABEL}.plist")),
    )
}

pub fn enabled() -> bool {
    agent_path().is_some_and(|path| path.exists())
}

/// Rewrites a login item created under the previous name so it starts this program.
pub fn migrate() {
    let Some(old) = previous_agent_path() else {
        return;
    };
    if !old.exists() {
        return;
    }
    if !enabled() {
        let _ = set_enabled(true);
        return;
    }
    let _ = std::fs::remove_file(old);
}

/// Adds or removes the agent. It takes effect at the next login.
pub fn set_enabled(on: bool) -> anyhow::Result<()> {
    let path = agent_path().context("HOME is not set")?;
    if !on {
        let removed = match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("removing {}", path.display()))
            }
            _ => Ok(()),
        };
        if let Some(old) = previous_agent_path() {
            let _ = std::fs::remove_file(old);
        }
        return removed;
    }
    let exe = std::env::current_exe().context("locating this program")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, plist(&exe.display().to_string()))
        .with_context(|| format!("writing {}", path.display()))?;
    if let Some(old) = previous_agent_path() {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

fn plist(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
"#,
        escape(exe)
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_the_executable_path() {
        assert!(plist("/Apps/R&D <x>/snapgrid").contains("/Apps/R&amp;D &lt;x&gt;/snapgrid"));
    }
}
