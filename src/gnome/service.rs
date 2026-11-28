//! The D-Bus object the extension's settings page talks to. Every call is
//! handed to the main loop, which owns the configuration, and the call waits
//! for its answer (JSON text).

use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use zbus::fdo;

use super::{Event, Request};

/// Long enough for a busy main loop; short enough for a stuck one to be noticed.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Service {
    events: Sender<Event>,
}

impl Service {
    pub fn new(events: Sender<Event>) -> Self {
        Self { events }
    }

    fn ask(&self, request: Request) -> fdo::Result<String> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.events
            .send(Event::Settings(request, reply))
            .map_err(|_| fdo::Error::Failed("SnapGrid is shutting down".into()))?;
        answer
            .recv_timeout(ANSWER_TIMEOUT)
            .map_err(|_| fdo::Error::Failed("SnapGrid did not answer in time".into()))?
            .map_err(fdo::Error::Failed)
    }
}

#[zbus::interface(name = "local.snapgrid.Daemon")]
impl Service {
    /// Preferences, ignored applications and commands, as JSON.
    fn get_settings(&self) -> fdo::Result<String> {
        self.ask(Request::Get)
    }

    /// Sets a preference by its `config.toml` name; the value is JSON.
    fn set_preference(&self, key: String, value: String) -> fdo::Result<String> {
        self.ask(Request::SetPreference(key, value))
    }

    fn set_ignored(&self, app: String, ignored: bool) -> fdo::Result<String> {
        self.ask(Request::SetIgnored(app, ignored))
    }

    /// Replaces the commands; the argument has the shape of a commands file.
    fn set_commands(&self, commands: String) -> fdo::Result<String> {
        self.ask(Request::SetCommands(commands))
    }

    /// `preferences`, `commands` or `all`.
    fn restore_defaults(&self, what: String) -> fdo::Result<String> {
        self.ask(Request::Restore(what))
    }

    fn open_config_file(&self) -> fdo::Result<String> {
        self.ask(Request::OpenConfig)
    }
}
