use cosmic::iced::futures::SinkExt;
use cosmic::iced::{Subscription, stream};
use cosmic::{Task, widget};
use gio::glib;
use gio::prelude::*;
use std::any::TypeId;
use std::cell::Cell;
use std::future::pending;
use std::hash::Hash;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::{Mounter, MounterAuth, MounterItem, MounterItems, MounterMessage};
use crate::config::IconSizes;
use crate::err_str;
use crate::tab::{self, ChecksumState, DirSize, ItemMetadata, ItemThumbnail, Location};

const TARGET_URI_ATTRIBUTE: &str = "standard::target-uri";

// Attributes requested when listing a network directory.
const SCAN_ATTRIBUTES: &str = "standard::name,\
standard::display-name,\
standard::type,\
standard::size,\
standard::icon,\
standard::is-hidden,\
time::modified";

fn resolve_uri(uri: &str) -> (String, gio::File) {
    let file = gio::File::for_uri(uri);
    // Resolve the target-uri if it exists
    if let Ok(file_info) = file.query_info(
        TARGET_URI_ATTRIBUTE,
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) && let Some(resolved_uri) = file_info.attribute_as_string(TARGET_URI_ATTRIBUTE)
    {
        let resolved_uri = String::from(resolved_uri);
        let file = gio::File::for_uri(&resolved_uri);
        return (resolved_uri, file);
    }

    (uri.to_string(), file)
}

fn gio_icon_to_path(icon: &gio::Icon, size: u16) -> Option<PathBuf> {
    if let Some(themed_icon) = icon.downcast_ref::<gio::ThemedIcon>() {
        for name in themed_icon.names() {
            let named = widget::icon::from_name(name.as_str()).size(size);
            if let Some(path) = named.path() {
                return Some(path);
            }
        }
    }
    //TODO: handle more gio icon types
    None
}

fn items(monitor: &gio::VolumeMonitor, sizes: IconSizes) -> MounterItems {
    let mut items: MounterItems = (monitor.mounts().into_iter())
        .enumerate()
        // Hide shadowed mounts
        .filter(|(_, mount)| !mount.is_shadowed())
        .map(|(i, mount)| {
            let root = MountExt::root(&mount);
            let is_remote = root
                .query_filesystem_info(
                    gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE,
                    gio::Cancellable::NONE,
                )
                .ok()
                .map(|info| info.boolean(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE))
                .unwrap_or(true); // Default to remote if query fails

            MounterItem::Gvfs(Item {
                uri: mount.root().uri().into(),
                kind: ItemKind::Mount,
                index: i,
                name: mount.name().into(),
                is_mounted: true,
                is_remote,
                icon_opt: gio_icon_to_path(&MountExt::icon(&mount), sizes.grid()),
                icon_symbolic_opt: gio_icon_to_path(&MountExt::symbolic_icon(&mount), 16),
                path_opt: root.path(),
            })
        })
        .collect();
    items.extend(
        (monitor.volumes().into_iter())
            .enumerate()
            // Volumes with mounts are already listed by mount
            .filter(|(_, volume)| volume.get_mount().is_none())
            .map(|(i, volume)| {
                let uri = VolumeExt::activation_root(&volume)
                    .map(|f| f.uri().into())
                    .unwrap_or_default();
                MounterItem::Gvfs(Item {
                    // TODO can we get URI for volumes with no mount?
                    uri,
                    kind: ItemKind::Volume,
                    index: i,
                    name: volume.name().into(),
                    is_mounted: false,
                    is_remote: false,
                    icon_opt: gio_icon_to_path(&VolumeExt::icon(&volume), sizes.grid()),
                    icon_symbolic_opt: gio_icon_to_path(&VolumeExt::symbolic_icon(&volume), 16),
                    path_opt: None,
                })
            }),
    );
    items
}

fn network_scan(uri: &str, sizes: IconSizes) -> Result<Vec<tab::Item>, String> {
    // WMDE: the network:/// root is a device list (Windows-like "Network neighbourhood").
    // Arch's gvfs has no working discovery backends here, so populate it ourselves from
    // mDNS (Avahi) + WS-Discovery instead of the (empty) gio enumeration.
    if uri == "network:///" {
        return Ok(discovered_device_items(sizes));
    }
    let force_dir = uri.starts_with("network:///");
    let (_, file) = resolve_uri(uri);

    // Read .hidden file if present
    let hidden_files: Box<[String]> = if let Some(path) = file.path() {
        let hidden_file_path = path.join(".hidden");
        if hidden_file_path.is_file() {
            tab::parse_hidden_file(&hidden_file_path)
        } else {
            Box::from([])
        }
    } else {
        Box::from([])
    };

    // `filesystem::remote` belongs to the filesystem namespace, which `enumerate_children`
    // never fills in, so it has to be queried once for the directory being listed.
    let remote = file
        .query_filesystem_info(
            gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE,
            gio::Cancellable::NONE,
        )
        .map(|info| info.boolean(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE))
        .unwrap_or_else(|err| {
            log::warn!("failed to get GIO filesystem info for {uri}: {err}");
            // Assume remote, so per-entry work is skipped rather than retried
            true
        });

    let mut items = Vec::new();
    for info_res in file
        .enumerate_children(
            SCAN_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map_err(err_str)?
    {
        let info = info_res.map_err(err_str)?;
        let name = info.name().to_string_lossy().into_owned();
        let display_name = String::from(info.display_name());

        // WMDE: a mountable (e.g. an SMB share listed under a server) exposes its real address
        // in standard::target-uri; file.child(name).uri() yields a mangled "._name" form for it.
        let is_mountable = matches!(info.file_type(), gio::FileType::Mountable);
        let uri = if is_mountable {
            info.attribute_string(gio::FILE_ATTRIBUTE_STANDARD_TARGET_URI)
                .map(|s| s.to_string())
                .unwrap_or_else(|| String::from(file.child(info.name()).uri()))
        } else {
            String::from(file.child(info.name()).uri())
        };

        //TODO: what is the best way to resolve shortcuts?
        let location = Location::Network(uri, display_name.clone(), file.child(&name).path());

        let metadata = if is_mountable {
            // WMDE: treat a share/mountable as an openable directory so double-click navigates;
            // it is mounted on demand when opened (see the DoubleClick handler in tab.rs).
            ItemMetadata::SimpleDir { entries: 0 }
        } else if force_dir {
            ItemMetadata::SimpleDir { entries: 0 }
        } else {
            let mtime = info.attribute_uint64(gio::FILE_ATTRIBUTE_TIME_MODIFIED);
            let is_dir = matches!(info.file_type(), gio::FileType::Directory);
            let size_opt = (!is_dir).then_some(info.size() as u64);
            let mut children_opt = None;

            // Counting children costs a directory listing per entry, which is far too
            // expensive on a remote filesystem
            if is_dir && !remote {
                if let Some(path) = file.child(&name).path() {
                    //TODO: calculate children in the background (and make it cancellable?)
                    match std::fs::read_dir(&path) {
                        Ok(entries) => {
                            children_opt = Some(entries.count());
                        }
                        Err(err) => {
                            log::warn!("failed to read directory {}: {}", path.display(), err);
                            children_opt = Some(0);
                        }
                    }
                } else {
                    children_opt = Some(0);
                }
            }
            ItemMetadata::GvfsPath {
                mtime,
                size_opt,
                children_opt,
                is_dir,
            }
        };

        // WMDE: draw network entries with the WMDE icon theme, matching local browsing - folders
        // use the themed folder icon (folder / folder-remote for a mountable share), files use the
        // mime icon guessed from the name - instead of gio's GIcon (a foreign full-colour icon).
        let (mime, icon_handle_grid, icon_handle_list, icon_handle_list_condensed) =
            if metadata.is_dir() {
                let dir_icon = |size| {
                    widget::icon::from_name(if is_mountable { "folder-remote" } else { "folder" })
                        .prefer_svg(true)
                        .size(size)
                        .handle()
                };
                (
                    "inode/directory".parse().unwrap(),
                    dir_icon(sizes.grid()),
                    dir_icon(sizes.list()),
                    dir_icon(sizes.list_condensed()),
                )
            } else {
                let mime = crate::mime_icon::mime_for_path(&name, None, true);
                (
                    mime.clone(),
                    crate::mime_icon::mime_icon(mime.clone(), sizes.grid()),
                    crate::mime_icon::mime_icon(mime.clone(), sizes.list()),
                    crate::mime_icon::mime_icon(mime, sizes.list_condensed()),
                )
            };

        // Check if item is hidden
        let hidden = name.starts_with('.')
            || info.boolean(gio::FILE_ATTRIBUTE_STANDARD_IS_HIDDEN)
            || hidden_files.contains(&name);

        items.push(tab::Item {
            name,
            is_mount_point: false,
            display_name,
            metadata,
            hidden,
            location_opt: Some(location),
            image_dimensions: None,
            mime,
            icon_handle_grid,
            icon_handle_list,
            icon_handle_list_condensed,
            thumbnail_opt: Some(ItemThumbnail::NotImage),
            button_id: widget::Id::unique(),
            pos_opt: Cell::new(None),
            rect_opt: Cell::new(None),
            selected: false,
            highlighted: false,
            overlaps_drag_rect: false,
            //TODO: scan directory size on gvfs mounts?
            dir_size: DirSize::NotDirectory,
            cut: false,
            tooltip_opt: None,
            checksums: ChecksumState::default(),
        });
    }
    Ok(items)
}

/// WMDE: build network:/// items from LAN device discovery (mDNS + WSD). Computers open into
/// an SMB server-browse; printers/scanners are shown as informational icons.
fn discovered_device_items(sizes: IconSizes) -> Vec<tab::Item> {
    use crate::network_discovery::{DeviceKind, cached_devices, refresh_devices};
    // Kick a background refresh (a no-op if one is already running) and render the last
    // cached result immediately; the view updates in place when the refresh completes.
    refresh_devices();
    cached_devices()
        .into_iter()
        .map(|device| {
            let icon_name = match device.kind {
                DeviceKind::Computer => "network-server",
                DeviceKind::Printer => "printer",
                DeviceKind::Scanner => "scanner",
                DeviceKind::Other => "network-workgroup",
            };
            let icon =
                |size| widget::icon::from_name(icon_name).prefer_svg(true).size(size).handle();
            // Browse the device with the scheme it actually advertises (smb/sftp/ftp/nfs/dav).
            // A device with nothing browsable but a published web UI (a printer's admin page)
            // gets that URL instead - the double-click handler opens http(s) in a browser.
            let location_opt = device
                .open_uri
                .clone()
                .or_else(|| device.web_url.clone())
                .map(|uri| Location::Network(uri, device.name.clone(), None));
            let tooltip_opt = (!device.details.is_empty()).then(|| device.details.clone());
            tab::Item {
                name: device.name.clone(),
                is_mount_point: false,
                display_name: device.name,
                tooltip_opt,
                metadata: ItemMetadata::SimpleDir { entries: 0 },
                hidden: false,
                location_opt,
                image_dimensions: None,
                mime: "inode/directory".parse().unwrap(),
                icon_handle_grid: icon(sizes.grid()),
                icon_handle_list: icon(sizes.list()),
                icon_handle_list_condensed: icon(sizes.list_condensed()),
                thumbnail_opt: Some(ItemThumbnail::NotImage),
                button_id: widget::Id::unique(),
                pos_opt: Cell::new(None),
                rect_opt: Cell::new(None),
                selected: false,
                highlighted: false,
                overlaps_drag_rect: false,
                dir_size: DirSize::NotDirectory,
                cut: false,
                checksums: ChecksumState::default(),
            }
        })
        .collect()
}

fn dir_info(uri: &str) -> Result<(String, String, Option<PathBuf>), glib::Error> {
    let (resolved_uri, file) = resolve_uri(uri);
    let info = file.query_info(
        gio::FILE_ATTRIBUTE_STANDARD_DISPLAY_NAME,
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;

    Ok((resolved_uri, info.display_name().into(), file.path()))
}

/// Whether a volume is one to mount on its own, without the user asking.
///
/// - `should_automount` is udisks' `HintAuto` as gvfs passes it on: true for USB, FireWire,
///   SDIO and optical drives, and for a volume plugged in moments ago or found when the
///   volume monitor started; false for a partition that appeared long after its medium,
///   which means the medium was repartitioned.
/// - `removable` is the drive's own word that the user may take it away. Internal disks and
///   loop devices are never mounted on their own, whatever their hint says.
const fn automount_wanted(
    should_automount: bool,
    can_mount: bool,
    mounted: bool,
    removable: bool,
) -> bool {
    should_automount && can_mount && !mounted && removable
}

/// WMDE: mounts `volume` if [`automount_wanted`] says so, and with `announce` reports the
/// mount so that the desktop layer can show its notification.
///
/// No mount operation is passed on purpose. gvfs then tells udisks not to interact: nothing
/// asks for a password, an encrypted volume stays locked until opened from the file
/// manager, and a session that is not the active one is refused quietly by polkit.
fn automount_volume(
    volume: &gio::Volume,
    announce: bool,
    event_tx: std::sync::Weak<crate::channel::Sender<Event>>,
) {
    let removable = volume.drive().is_some_and(|drive| drive.is_removable());
    if !automount_wanted(
        volume.should_automount(),
        volume.can_mount(),
        volume.get_mount().is_some(),
        removable,
    ) {
        return;
    }

    let name = VolumeExt::name(volume).to_string();
    log::info!("automount {name}");
    let mounted = volume.clone();
    VolumeExt::mount(
        volume,
        gio::MountMountFlags::NONE,
        gio::MountOperation::NONE,
        gio::Cancellable::NONE,
        move |res| match res {
            Ok(()) => {
                let path = mounted
                    .get_mount()
                    .and_then(|mount| MountExt::root(&mount).path());
                log::info!("automount {name}: mounted at {path:?}");
                if announce
                    && let Some(path) = path
                    && let Some(event_tx) = event_tx.upgrade()
                {
                    event_tx.send(Event::Automounted(name, path));
                }
            }
            Err(err) => log::warn!("automount {name}: {err}"),
        },
    );
}

fn mount_op(
    uri: String,
    event_tx: std::sync::Weak<crate::channel::Sender<Event>>,
) -> gio::MountOperation {
    let mount_op = gio::MountOperation::new();
    mount_op.connect_ask_password(
        move |mount_op, message, default_user, default_domain, flags| {
            let auth = MounterAuth {
                message: message.to_string(),
                username_opt: flags
                    .contains(gio::AskPasswordFlags::NEED_USERNAME)
                    .then(|| default_user.to_string()),
                domain_opt: flags
                    .contains(gio::AskPasswordFlags::NEED_DOMAIN)
                    .then(|| default_domain.to_string()),
                password_opt: flags
                    .contains(gio::AskPasswordFlags::NEED_PASSWORD)
                    .then(String::new),
                remember_opt: flags
                    .contains(gio::AskPasswordFlags::SAVING_SUPPORTED)
                    .then_some(false),
                anonymous_opt: flags
                    .contains(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED)
                    .then_some(false),
            };
            let (auth_tx, mut auth_rx) = mpsc::channel(1);
            if let Some(event_tx) = event_tx.upgrade() {
                event_tx.send(Event::NetworkAuth(uri.clone(), auth, auth_tx));
            }
            //TODO: async recv?
            if let Some(auth) = auth_rx.blocking_recv() {
                if auth.anonymous_opt == Some(true) {
                    mount_op.set_anonymous(true);
                } else {
                    mount_op.set_username(auth.username_opt.as_deref());
                    mount_op.set_domain(auth.domain_opt.as_deref());
                    mount_op.set_password(auth.password_opt.as_deref());
                    if auth.remember_opt == Some(true) {
                        mount_op.set_password_save(gio::PasswordSave::Permanently);
                    }
                }
                mount_op.reply(gio::MountOperationResult::Handled);
            } else {
                mount_op.reply(gio::MountOperationResult::Aborted);
            }
        },
    );
    mount_op
}

enum Cmd {
    Items(IconSizes, mpsc::Sender<MounterItems>),
    Rescan,
    Mount(
        MounterItem,
        tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    ),
    NetworkDrive(String, tokio::sync::oneshot::Sender<anyhow::Result<()>>),
    NetworkScan(
        String,
        IconSizes,
        mpsc::Sender<Result<Vec<tab::Item>, String>>,
    ),
    DirInfo(
        String,
        mpsc::Sender<Result<(String, String, Option<PathBuf>), glib::Error>>,
    ),
    Unmount(MounterItem),
    /// WMDE: whether removable volumes are mounted on their own. Only the desktop layer sends
    /// it; see `Mounter::automount`.
    Automount(bool),
}

enum Event {
    Changed,
    /// WMDE: a volume plugged in during the session was mounted on its own: its name and the
    /// directory it is mounted on.
    Automounted(String, PathBuf),
    Items(MounterItems),
    MountResult(MounterItem, Result<bool, String>),
    NetworkAuth(String, MounterAuth, mpsc::Sender<MounterAuth>),
    NetworkResult(String, Result<bool, String>),
}

#[derive(Clone, Debug)]
enum ItemKind {
    Mount,
    Volume,
}

//TODO: better method of matching items
#[derive(Clone, Debug)]
pub struct Item {
    uri: String,
    kind: ItemKind,
    index: usize,
    name: String,
    is_mounted: bool,
    is_remote: bool,
    icon_opt: Option<PathBuf>,
    icon_symbolic_opt: Option<PathBuf>,
    path_opt: Option<PathBuf>,
}

impl Item {
    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub const fn is_mounted(&self) -> bool {
        self.is_mounted
    }

    pub const fn is_remote(&self) -> bool {
        self.is_remote
    }

    pub fn uri(&self) -> String {
        self.uri.clone()
    }

    pub fn icon(&self, symbolic: bool) -> Option<widget::icon::Handle> {
        if symbolic {
            self.icon_symbolic_opt.as_ref()
        } else {
            self.icon_opt.as_ref()
        }
        .map(|icon| widget::icon::from_path(icon.clone()))
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.path_opt.clone()
    }
}

pub struct Gvfs {
    command_tx: mpsc::UnboundedSender<Cmd>,
    event_rx: Arc<crate::channel::Receiver<Event>>,
}

impl Gvfs {
    pub fn new() -> Self {
        //TODO: switch to using gvfs-zbus which will better integrate with async rust
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = crate::channel::channel();
        let event_tx = Arc::new(event_tx);
        std::thread::spawn(move || {
            let main_loop = glib::MainLoop::new(None, false);
            main_loop.context().spawn_local(async move {
                let event_tx = Arc::downgrade(&event_tx);
                let monitor = gio::VolumeMonitor::get();
                // WMDE: `None` until the desktop layer first says whether to automount. The
                // answer that turns it on mounts what was plugged in before it came, without
                // a notification, as Windows gives such a drive its letter at sign-in.
                let automount: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
                {
                    let event_tx = event_tx.clone();
                    monitor.connect_mount_changed(move |_monitor, mount| {
                        log::info!("mount changed {}", MountExt::name(mount));
                        if let Some(event_tx) = event_tx.upgrade() {
                            event_tx.send(Event::Changed);
                        }
                    });
                }
                {
                    let event_tx = event_tx.clone();
                    monitor.connect_mount_added(move |_monitor, mount| {
                        log::info!("mount added {}", MountExt::name(mount));
                        if let Some(event_tx) = event_tx.upgrade() {
                            event_tx.send(Event::Changed);
                        }
                    });
                }
                {
                    let event_tx = event_tx.clone();
                    monitor.connect_mount_removed(move |_monitor, mount| {
                        log::info!("mount removed {}", MountExt::name(mount));
                        if let Some(event_tx) = event_tx.upgrade() {
                            event_tx.send(Event::Changed);
                        }
                    });
                }

                {
                    let event_tx = event_tx.clone();
                    monitor.connect_volume_changed(move |_monitor, volume| {
                        log::info!("volume changed {}", VolumeExt::name(volume));
                        if let Some(event_tx) = event_tx.upgrade() {
                            event_tx.send(Event::Changed);
                        }
                    });
                }
                {
                    let event_tx = event_tx.clone();
                    let automount = automount.clone();
                    monitor.connect_volume_added(move |_monitor, volume| {
                        log::info!("volume added {}", VolumeExt::name(volume));
                        if let Some(event_tx) = event_tx.upgrade() {
                            event_tx.send(Event::Changed);
                        }
                        if automount.get() == Some(true) {
                            automount_volume(volume, true, event_tx.clone());
                        }
                    });
                }
                {
                    let event_tx = event_tx.clone();
                    monitor.connect_volume_removed(move |_monitor, volume| {
                        log::info!("volume removed {}", VolumeExt::name(volume));
                        if let Some(event_tx) = event_tx.upgrade() {
                            event_tx.send(Event::Changed);
                        }
                    });
                }

                while let Some(command) = command_rx.recv().await {
                    match command {
                        Cmd::Items(sizes, items_tx) => {
                            items_tx.send(items(&monitor, sizes)).await.unwrap();
                        }
                        Cmd::Rescan => {
                            let Some(event_tx) = event_tx.upgrade() else {
                                return;
                            };

                            event_tx.send(Event::Items(items(&monitor, IconSizes::default())));
                        }
                        Cmd::Mount(mounter_item, complete_tx) => {
                            let MounterItem::Gvfs(ref item) = mounter_item else {
                                _ = complete_tx.send(Err(anyhow::anyhow!("No mounter item")));
                                continue
                            };
                            let ItemKind::Volume = item.kind else {
                                _ = complete_tx.send(Err(anyhow::anyhow!("No mounter volume")));
                                continue
                            };
                            for (i, volume) in monitor.volumes().into_iter().enumerate() {
                                if i != item.index {
                                    continue;
                                }

                                let name = VolumeExt::name(&volume);
                                if item.name != name {
                                    log::warn!("trying to mount volume {} failed: name is {:?} when {:?} was expected", i, name, item.name);
                                    continue;
                                }

                                log::info!("mount {name}");
                                //TODO: do not use name as a URI for mount_op
                                let mount_op = mount_op(name.to_string(), event_tx.clone());
                                let event_tx = event_tx.clone();
                                let mounter_item = mounter_item.clone();
                                let volume_for_callback = volume.clone();
                                VolumeExt::mount(
                                    &volume,
                                    gio::MountMountFlags::NONE,
                                    Some(&mount_op),
                                    gio::Cancellable::NONE,
                                    move |res| {
                                        log::info!("mount {name}: result {res:?}");
                                        // Update the mounter_item with mount information after successful mount
                                        let mut updated_item = mounter_item.clone();
                                        if res.is_ok()
                                            && let MounterItem::Gvfs(ref mut item) = updated_item
                                                && let Some(mount) = volume_for_callback.get_mount() {
                                                    let root = MountExt::root(&mount);
                                                    item.path_opt = root.path();
                                                    item.is_mounted = true;
                                                    // Query if remote
                                                    item.is_remote = root
                                                        .query_filesystem_info(
                                                            gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE,
                                                            gio::Cancellable::NONE,
                                                        )
                                                        .ok().map(|info| info.boolean(gio::FILE_ATTRIBUTE_FILESYSTEM_REMOTE))
                                                        .unwrap_or(true);
                                                }
                                        let Some(event_tx) = event_tx.upgrade() else {
                                            return;
                                        };
                                        event_tx.send(Event::MountResult(updated_item, match res {
                                            Ok(()) => {
                                                _ = complete_tx.send(Ok(()));
                                                Ok(true)
                                            },
                                            Err(err) => {
                                                _ = complete_tx.send(Err(anyhow::anyhow!("{err:?}")));
                                                match err.kind::<gio::IOErrorEnum>() {
                                                Some(gio::IOErrorEnum::FailedHandled) => Ok(false),
                                                _ => Err(format!("{err}"))
                                            }}
                                        }));
                                    },
                                );
                                break;
                            }
                        }
                        Cmd::NetworkDrive(uri, result_tx) => {
                            let file = gio::File::for_uri(&uri);
                            let mount_op = mount_op(uri.clone(), event_tx.clone());
                            let event_tx = event_tx.clone();
                            file.mount_enclosing_volume(
                                gio::MountMountFlags::NONE,
                                Some(&mount_op),
                                gio::Cancellable::NONE,
                                move |res| {
                                    log::info!("network drive {uri}: result {res:?}");
                                    let Some(event_tx) = event_tx.upgrade() else {
                                        return;
                                    };
                                    event_tx.send(Event::NetworkResult(uri, match res {
                                        Ok(()) => {
                                            _ = result_tx.send(Ok(()));
                                            Ok(true)},
                                        Err(err) => {
                                            _ = result_tx.send(Err(anyhow::anyhow!("{err:?}")));
                                            match err.kind::<gio::IOErrorEnum>() {
                                            Some(gio::IOErrorEnum::FailedHandled) => Ok(false),
                                            _ => Err(format!("{err}"))
                                        }}
                                    }));
                                }
                            );
                        }
                        Cmd::NetworkScan(uri, sizes, items_tx) => {
                            let (resolved_uri, file) = resolve_uri(&uri);

                            let needs_mount = resolved_uri != "network:///" && match file.find_enclosing_mount(gio::Cancellable::NONE) {
                                Ok(_) => false,
                                Err(err) => matches!(err.kind::<gio::IOErrorEnum>(), Some(gio::IOErrorEnum::NotMounted))
                            };

                            if needs_mount {
                                let mount_op = mount_op(resolved_uri.clone(), event_tx.clone());
                                let event_tx = event_tx.clone();
                                file.mount_enclosing_volume(
                                    gio::MountMountFlags::empty(),
                                    Some(&mount_op),
                                    gio::Cancellable::NONE,
                                    move |res| {
                                        log::info!("network scan mounted {resolved_uri}: result {res:?}");
                                        // FIXME sometimes a uri can be mounted and then not recognized as mounted...
                                        // seems to be related to uri with a path
                                        items_tx.blocking_send(network_scan(&uri, sizes)).unwrap();
                                        let Some(event_tx) = event_tx.upgrade() else {
                                            return;
                                        };
                                        event_tx.send(Event::NetworkResult(resolved_uri, match res {
                                            Ok(()) => {
                                                Ok(true)
                                            },
                                            Err(err) => match err.kind::<gio::IOErrorEnum>() {
                                                Some(gio::IOErrorEnum::FailedHandled) => Ok(false),
                                                _ => Err(format!("{err}"))
                                            }
                                        }));
                                    }
                                );
                            } else {
                                items_tx.send(network_scan(&uri, sizes)).await.unwrap();
                            }
                        }
                        Cmd::DirInfo(uri, result_tx) => {
                            result_tx.send(dir_info(&uri)).await.unwrap();
                        }
                        Cmd::Automount(enabled) => {
                            let first = automount.replace(Some(enabled)).is_none();
                            if first && enabled {
                                for volume in monitor.volumes() {
                                    automount_volume(&volume, false, event_tx.clone());
                                }
                            }
                        }
                        Cmd::Unmount(mounter_item) => {
                            let MounterItem::Gvfs(item) = mounter_item else { continue };
                            let ItemKind::Mount = item.kind else { continue };
                            for (i, mount) in monitor.mounts().into_iter().enumerate() {
                                if i != item.index {
                                    continue;
                                }

                                let name = MountExt::name(&mount);
                                if item.name != name {
                                    log::warn!("trying to unmount mount {} failed: name is {:?} when {:?} was expected", i, name, item.name);
                                    continue;
                                }

                                if MountExt::can_eject(&mount) {
                                    log::info!("eject {name}");
                                    MountExt::eject_with_operation(
                                        &mount,
                                        gio::MountUnmountFlags::NONE,
                                        gio::MountOperation::NONE,
                                        gio::Cancellable::NONE,
                                        move |result| {
                                            log::info!("eject {name}: result {result:?}");
                                        },
                                    );
                                } else {
                                    log::info!("unmount {name}");
                                    MountExt::unmount_with_operation(
                                        &mount,
                                        gio::MountUnmountFlags::NONE,
                                        gio::MountOperation::NONE,
                                        gio::Cancellable::NONE,
                                        move |result| {
                                            log::info!("unmount {name}: result {result:?}");
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            });
            main_loop.run();
        });
        Self {
            command_tx,
            event_rx: Arc::new(event_rx),
        }
    }
}

impl Mounter for Gvfs {
    fn items(&self, sizes: IconSizes) -> Option<MounterItems> {
        let (items_tx, mut items_rx) = mpsc::channel(1);
        self.command_tx.send(Cmd::Items(sizes, items_tx)).unwrap();
        items_rx.blocking_recv()
    }

    fn mount(&self, item: MounterItem) -> Task<()> {
        let command_tx = self.command_tx.clone();
        Task::perform(
            async move {
                let (res_tx, res_rx) = tokio::sync::oneshot::channel();

                command_tx.send(Cmd::Mount(item, res_tx)).unwrap();
                res_rx.await
            },
            |x| {
                if let Err(err) = x {
                    log::error!("{err:?}");
                }
            },
        )
    }

    fn network_drive(&self, uri: String) -> Task<bool> {
        let command_tx = self.command_tx.clone();
        Task::perform(
            async move {
                let (res_tx, res_rx) = tokio::sync::oneshot::channel();

                command_tx.send(Cmd::NetworkDrive(uri, res_tx)).unwrap();
                res_rx.await
            },
            |result| match result {
                Ok(Ok(())) => true,
                Ok(Err(err)) => {
                    log::error!("{err:?}");
                    false
                }
                Err(err) => {
                    log::error!("{err:?}");
                    false
                }
            },
        )
    }

    fn network_scan(&self, uri: &str, sizes: IconSizes) -> Option<Result<Vec<tab::Item>, String>> {
        let (items_tx, mut items_rx) = mpsc::channel(1);
        self.command_tx
            .send(Cmd::NetworkScan(uri.to_string(), sizes, items_tx))
            .unwrap();
        items_rx.blocking_recv()
    }

    fn dir_info(&self, uri: &str) -> Option<(String, String, Option<PathBuf>)> {
        let (result_tx, mut result_rx) = mpsc::channel(1);
        self.command_tx
            .send(Cmd::DirInfo(uri.to_string(), result_tx))
            .unwrap();
        result_rx.blocking_recv().and_then(|res| res.ok())
    }

    fn unmount(&self, item: MounterItem) -> Task<()> {
        let command_tx = self.command_tx.clone();
        Task::future(async move {
            command_tx.send(Cmd::Unmount(item)).unwrap();
        })
    }

    fn automount(&self, enabled: bool) {
        if let Err(err) = self.command_tx.send(Cmd::Automount(enabled)) {
            log::warn!("gvfs thread is gone, automount not set: {err}");
        }
    }

    fn subscription(&self) -> Subscription<MounterMessage> {
        let command_tx = self.command_tx.clone();
        let event_rx = self.event_rx.clone();
        struct Wrapper {
            command_tx: mpsc::UnboundedSender<Cmd>,
            event_rx: Arc<crate::channel::Receiver<Event>>,
        }
        impl Hash for Wrapper {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                TypeId::of::<Self>().hash(state);
            }
        }
        Subscription::run_with(
            Wrapper {
                command_tx,
                event_rx,
            },
            |Wrapper {
                 command_tx,
                 event_rx,
             }| {
                let command_tx = command_tx.clone();
                let event_rx = event_rx.clone();
                stream::channel(
                    1,
                    move |mut output: cosmic::iced::futures::channel::mpsc::Sender<
                        MounterMessage,
                    >| async move {
                        command_tx.send(Cmd::Rescan).unwrap();
                        while let Some(event) = event_rx.recv().await {
                            match event {
                                Event::Changed => command_tx.send(Cmd::Rescan).unwrap(),
                                Event::Items(items) => {
                                    output.send(MounterMessage::Items(items)).await.unwrap();
                                }
                                Event::MountResult(item, res) => output
                                    .send(MounterMessage::MountResult(item, res))
                                    .await
                                    .unwrap(),
                                Event::NetworkAuth(uri, auth, auth_tx) => output
                                    .send(MounterMessage::NetworkAuth(uri, auth, auth_tx))
                                    .await
                                    .unwrap(),
                                Event::NetworkResult(uri, res) => output
                                    .send(MounterMessage::NetworkResult(uri, res))
                                    .await
                                    .unwrap(),
                                Event::Automounted(name, path) => output
                                    .send(MounterMessage::Automounted(name, path))
                                    .await
                                    .unwrap(),
                            }
                        }
                        pending().await
                    },
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::automount_wanted;

    /// Only a removable volume that asks for it, can be mounted and is not yet mounted is
    /// mounted on its own; each of the four alone vetoes it.
    #[test]
    fn automount_needs_all_four() {
        for bits in 0u8..16 {
            let [should, can, mounted, removable] = [1, 2, 4, 8].map(|bit| bits & bit != 0);
            assert_eq!(
                automount_wanted(should, can, mounted, removable),
                should && can && !mounted && removable,
                "should={should} can={can} mounted={mounted} removable={removable}"
            );
        }
        assert!(automount_wanted(true, true, false, true));
        assert!(!automount_wanted(true, true, false, false));
        assert!(!automount_wanted(true, true, true, true));
    }
}
