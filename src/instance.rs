//! One running Sash per user. Every Linux backend (X11, GNOME Shell) shares it, so
//! a second start, from the application menu for instance, cannot leave two
//! copies grabbing shortcuts, and quitting Sash really stops everything the user
//! was seeing.

use std::io::ErrorKind;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{SocketAddr, UnixListener};

use anyhow::Context;

/// Held for as long as this process runs. The name lives in the abstract socket
/// namespace, so the kernel frees it when the process exits or crashes, and no
/// stale file can lock the next start out.
pub struct Instance(#[allow(dead_code)] UnixListener);

/// Takes the per-user instance name. `None` means another Sash already holds it.
pub fn acquire() -> anyhow::Result<Option<Instance>> {
    let uid = std::fs::metadata("/proc/self")
        .context("reading /proc/self")?
        .uid();
    acquire_named(&format!("sash.instance.{uid}"))
}

fn acquire_named(name: &str) -> anyhow::Result<Option<Instance>> {
    let address =
        SocketAddr::from_abstract_name(name).context("building the instance address")?;
    match UnixListener::bind_addr(&address) {
        Ok(listener) => Ok(Some(Instance(listener))),
        Err(error) if error.kind() == ErrorKind::AddrInUse => Ok(None),
        Err(error) => Err(error).context("taking the single-instance lock"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        // Not the real name: a Sash that is running must not fail the test.
        let name = format!("sash.instance.test.{}", std::process::id());
        let first = acquire_named(&name).unwrap();
        assert!(first.is_some());
        assert!(acquire_named(&name).unwrap().is_none());
        drop(first);
        assert!(acquire_named(&name).unwrap().is_some());
    }
}
