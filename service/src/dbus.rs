//! D-Bus interface used by the KWin script.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;

use pipewire::channel::Sender;
use zbus::message::Header;
use zbus::names::OwnedUniqueName;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{blocking, fdo, interface};

use crate::audio::Msg;

pub const BUS_NAME: &str = "io.github.belebob.MuteOnMinimize";
pub const OBJECT_PATH: &str = "/io/github/belebob/MuteOnMinimize";

struct Service {
    tx: Sender<Msg>,
    /// Unique name of the last caller, so its streams can be released when
    /// it disconnects (e.g. KWin exits or crashes).
    client: Arc<Mutex<Option<OwnedUniqueName>>>,
}

#[interface(name = "io.github.belebob.MuteOnMinimize1")]
impl Service {
    /// Replaces the set of processes whose audio is muted, together with
    /// their descendants.
    ///
    /// Takes `av` because KWin's `callDBus` marshals a JS array of numbers
    /// as an array of int32 variants.
    fn set_muted_pids(
        &self,
        #[zbus(header)] header: Header<'_>,
        pids: Vec<OwnedValue>,
    ) -> fdo::Result<()> {
        let pids = pids
            .iter()
            .map(|v| pid_from_value(v))
            .collect::<Option<HashSet<u32>>>()
            .ok_or_else(|| fdo::Error::InvalidArgs("expected an array of PIDs".into()))?;
        if let Some(sender) = header.sender() {
            *self.client.lock().unwrap() = Some(sender.to_owned().into());
        }
        self.tx
            .send(Msg::SetMutedPids(pids))
            .map_err(|_| fdo::Error::Failed("audio loop is not running".into()))
    }
}

/// Accepts any integer type, since callers other than KWin may send `au`-like
/// values in a different width. PIDs 0 and 1 are rejected: every process
/// descends from init, so they would mute everything.
fn pid_from_value(value: &Value<'_>) -> Option<u32> {
    let pid = match value {
        Value::I32(n) => u32::try_from(*n).ok()?,
        Value::U32(n) => *n,
        Value::I64(n) => u32::try_from(*n).ok()?,
        Value::U64(n) => u32::try_from(*n).ok()?,
        Value::Value(inner) => return pid_from_value(inner),
        _ => return None,
    };
    (pid > 1).then_some(pid)
}

/// Claims the bus name and serves the interface. The returned connection
/// must be kept alive.
pub fn serve(tx: Sender<Msg>) -> zbus::Result<blocking::Connection> {
    let client = Arc::new(Mutex::new(None::<OwnedUniqueName>));
    let connection = blocking::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            OBJECT_PATH,
            Service {
                tx: tx.clone(),
                client: client.clone(),
            },
        )?
        .build()?;

    let owner_changes = blocking::fdo::DBusProxy::new(&connection)?.receive_name_owner_changed()?;
    thread::spawn(move || {
        for signal in owner_changes {
            let Ok(args) = signal.args() else {
                continue;
            };
            if args.new_owner().is_some() {
                continue;
            }
            let mut client = client.lock().unwrap();
            if client
                .as_deref()
                .is_some_and(|c| c.as_str() == args.name().as_str())
            {
                *client = None;
                eprintln!("Caller left the bus, unmuting everything");
                let _ = tx.send(Msg::SetMutedPids(HashSet::new()));
            }
        }
    });

    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_values() {
        assert_eq!(pid_from_value(&Value::I32(4242)), Some(4242));
        assert_eq!(pid_from_value(&Value::U64(7)), Some(7));
        assert_eq!(
            pid_from_value(&Value::Value(Box::new(Value::I32(9)))),
            Some(9)
        );
        assert_eq!(pid_from_value(&Value::I32(1)), None);
        assert_eq!(pid_from_value(&Value::I32(-5)), None);
        assert_eq!(pid_from_value(&Value::from("12")), None);
    }
}
