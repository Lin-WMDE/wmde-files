// SPDX-License-Identifier: GPL-3.0-only

//! WMDE: the notification of a volume mounted on its own, as Windows announces a drive
//! plugged in.
//!
//! It goes straight to `org.freedesktop.Notifications` because it has to hear back: the
//! card offers Open, and the daemon answers a click with `ActivationToken` and then
//! `ActionInvoked`. The token is handed to the file manager it starts, so that the window
//! is allowed to take the focus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;

use cosmic::iced::futures::StreamExt;
use zbus::zvariant::Value;

use crate::fl;
use crate::spawn_detached::spawn_detached;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const ICON: &str = "drive-removable-media";
const DESKTOP_ENTRY: &str = "fun.wmde.files";

/// Shows that `name` is mounted at `path` and opens it in the file manager when the user
/// asks to. Lasts until the notification is closed.
pub async fn show(name: String, path: PathBuf) {
    if let Err(err) = notify(&name, &path).await {
        log::warn!(
            "cannot announce {name} mounted at {}: {err}",
            path.display()
        );
    }
}

async fn notify(name: &str, path: &Path) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(&connection, NAME, PATH, NAME).await?;
    // Subscribed before the card exists, so that no answer to it can be missed. One stream
    // for all three signals keeps them in the order the daemon sent them: the token comes
    // before the action it belongs to.
    let mut signals = proxy.receive_all_signals().await?;

    let open = fl!("open");
    // `default` is a click on the card, `open` the button under its text.
    let actions = ["default", open.as_str(), "open", open.as_str()];
    let hints = HashMap::from([("desktop-entry", Value::from(DESKTOP_ENTRY))]);
    let id: u32 = proxy
        .call(
            "Notify",
            &(
                fl!("wmde-files"),
                0u32,
                ICON,
                name,
                fl!("automount-connected"),
                &actions[..],
                hints,
                -1i32,
            ),
        )
        .await?;

    let mut token = None;
    while let Some(message) = signals.next().await {
        let header = message.header();
        let Some(member) = header.member() else {
            continue;
        };
        match member.as_str() {
            "ActivationToken" => {
                let (of, value): (u32, String) = message.body().deserialize()?;
                if of == id {
                    token = Some(value);
                }
            }
            "ActionInvoked" => {
                let (of, _action): (u32, String) = message.body().deserialize()?;
                if of == id {
                    open_in_files(path, token.take());
                }
            }
            "NotificationClosed" => {
                let (of, _reason): (u32, u32) = message.body().deserialize()?;
                if of == id {
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn open_in_files(path: &Path, token: Option<String>) {
    let mut command = process::Command::new("wmde-files");
    command.arg(path);
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
    if let Err(err) = spawn_detached(&mut command) {
        log::warn!("cannot open {} in wmde-files: {err}", path.display());
    }
}
