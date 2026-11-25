//! "Add Sash to login items": a LaunchAgent that starts this executable at login.

use std::path::PathBuf;

use anyhow::Context;

const LABEL: &str = "local.sash";

fn agent_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
    )
}

pub fn enabled() -> bool {
    agent_path().is_some_and(|path| path.exists())
}

/// Adds or removes the agent. It takes effect at the next login.
pub fn set_enabled(on: bool) -> anyhow::Result<()> {
    let path = agent_path().context("HOME is not set")?;
    if !on {
        return match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("removing {}", path.display()))
            }
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().context("locating this program")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, plist(&exe.display().to_string()))
        .with_context(|| format!("writing {}", path.display()))
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
        assert!(plist("/Apps/R&D <x>/sash").contains("/Apps/R&amp;D &lt;x&gt;/sash"));
    }
}
