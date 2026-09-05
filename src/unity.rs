// SPDX-License-Identifier: GPL-3.0-only
//! Ubuntu/Unity `com.canonical.Unity.LauncherEntry.Update` sender.
//!
//! Reports the summary progress of the pending file operations so the panel can paint it on
//! this application's button. Only `progress` and `progress-visible` are sent: `count`,
//! `count-visible` and `urgent` have no agreed look in this stack.

use std::collections::HashMap;
use std::time::Duration;

use cosmic::iced::futures::StreamExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::sleep;
use zbus::Connection;
use zbus::fdo::DBusProxy;
use zbus::zvariant::Value;

/// The desktop file id, suffix included - that is how libunity builds the uri and how every
/// receiver expects it. Receivers strip the suffix themselves.
// The bar belongs to the window that shows the operation, not to the file manager as a
// whole: the applet pairs this key with the app id of a toplevel, and the operations
// window carries `OPERATIONS_APP_ID`.
const APP_URI: &str = "application://fun.wmde.files.operations.desktop";

/// libunity puts the object at `/com/canonical/unity/launcherentry/<g_str_hash(uri)>`. Every
/// known receiver subscribes with no path filter, so the exact value only matters to somebody
/// reading the bus by hand; the shape is kept recognizable.
const OBJECT_PATH: &str = "/com/canonical/unity/launcherentry/wmde_files";

const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

/// The well-known name a receiver of this protocol takes. Its appearance means a fresh
/// receiver with an empty state, so everything we know has to be announced again.
const UNITY_NAME: &str = "com.canonical.Unity";

/// Delay before reconnecting after the session bus is lost. Same value as the receiver uses.
const RECONNECT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Update {
    /// 0.0 ..= 1.0
    pub progress: f64,
    pub visible: bool,
}

#[derive(Clone, Debug)]
pub struct Sender(UnboundedSender<Update>);

impl Sender {
    pub fn send(&self, update: Update) {
        // A closed channel means the application is going down: `run` only ever leaves its
        // loop when this channel closes, and a bus failure makes it reconnect instead.
        let _ = self.0.send(update);
    }
}

/// Starts the bus task. Must be called after `fork::daemon` and from inside the tokio runtime;
/// `App::init` satisfies both.
pub fn spawn() -> Sender {
    let (tx, rx) = unbounded_channel();
    tokio::spawn(run(rx));
    Sender(tx)
}

/// Keeps one connection alive for the life of the application.
///
/// The last value is remembered across connections on purpose. `App` only publishes what
/// changed, so after a bus failure - or after a receiver restarted with an empty state -
/// nothing at all would be sent until the percentage moved, and a paused operation never moves
/// it.
async fn run(mut rx: UnboundedReceiver<Update>) {
    let mut last: Option<Update> = None;
    loop {
        match serve(&mut rx, &mut last).await {
            Ok(()) => return,
            Err(err) => log::warn!("unity launcher entry sender reconnecting: {err}"),
        }
        sleep(RECONNECT).await;
    }
}

/// Serves the channel over one connection. `Ok` means the application closed the channel and
/// the task is done; `Err` means the connection is unusable and has to be taken again.
async fn serve(rx: &mut UnboundedReceiver<Update>, last: &mut Option<Update>) -> zbus::Result<()> {
    let conn = Connection::session().await?;
    let dbus = DBusProxy::new(&conn).await?;
    // Argument 0 of `NameOwnerChanged` is the name, so the bus filters for us.
    let mut owners = dbus
        .receive_name_owner_changed_with_args(&[(0, UNITY_NAME)])
        .await?;

    // Whatever was already reported is stale for everybody on the new connection.
    if let Some(update) = *last {
        emit(&conn, update).await?;
    }

    loop {
        tokio::select! {
            update = rx.recv() => {
                let Some(update) = update else {
                    // The channel closed together with the application. Nothing to clear:
                    // dropping off the bus is what tells the panel to drop the bar.
                    return Ok(());
                };
                *last = Some(update);
                emit(&conn, update).await?;
            }
            change = owners.next() => {
                let Some(change) = change else {
                    return Err(zbus::Error::InputOutput(std::io::Error::other(
                        "NameOwnerChanged stream ended",
                    ).into()));
                };
                // A receiver builds its state from the signals it sees, so the one that just
                // appeared knows nothing about an operation started before it.
                let took_the_name = change
                    .args()
                    .map(|args| args.new_owner().is_some())
                    .unwrap_or(false);
                if took_the_name && let Some(update) = *last {
                    emit(&conn, update).await?;
                }
            }
        }
    }
}

async fn emit(conn: &Connection, update: Update) -> zbus::Result<()> {
    let mut props: HashMap<&str, Value<'_>> = HashMap::with_capacity(2);
    props.insert("progress", Value::F64(update.progress));
    props.insert("progress-visible", Value::Bool(update.visible));
    // No destination: with one the message becomes unicast and no match rule sees it.
    conn.emit_signal(
        None::<&str>,
        OBJECT_PATH,
        INTERFACE,
        "Update",
        &(APP_URI, props),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::APP_URI;

    #[test]
    fn app_uri_matches_the_operations_window() {
        // The receiver keys the bar by desktop id and pairs it with the app id of a toplevel.
        // A uri that stops matching the window paints the progress on somebody else's button,
        // or on none at all, and nothing else would report it.
        assert_eq!(
            APP_URI,
            format!("application://{}.desktop", crate::app::OPERATIONS_APP_ID)
        );
    }
}
