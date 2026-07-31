// SPDX-License-Identifier: GPL-3.0-only

//! Key bindings the file manager handles itself.
//!
//! How a binding is written, how the user's map is layered over these defaults and
//! how a combination is displayed live in [`cosmic::shortcuts`], shared with the
//! rest of the stack, so that the settings app can edit this map without knowing
//! anything about files.
//!
//! The defaults below are the full set, the one the application window uses. The
//! desktop and the file chooser dialog carry fewer: [`allowed`] filters the
//! resolved map down to what the mode can actually perform, so a user rebinding an
//! action does not smuggle it into a mode that has no place for it.

use cosmic::shortcuts::ShortcutAction;
use cosmic::widget::menu::key_bind::KeyBind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::app::Action;
use crate::tab;

pub use cosmic::shortcuts::{Binding, ModifierName};

/// The stored map: what goes into the `shortcuts_custom` config key.
pub type Shortcuts = cosmic::shortcuts::Shortcuts<KeyBindAction>;

/// The user's map layered over [`fallback_shortcuts`].
pub type ShortcutsConfig = cosmic::shortcuts::ShortcutsConfig<KeyBindAction>;

/// Builds the runtime configuration from the user's stored map.
#[must_use]
pub fn shortcuts_config(custom: Shortcuts) -> ShortcutsConfig {
    ShortcutsConfig::new(fallback_shortcuts(), custom)
}

/// An action a key combination can be bound to.
///
/// Kept apart from [`Action`] so that the storable set is explicit and can carry
/// `Disable`, which takes a shipped binding away instead of running anything.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum KeyBindAction {
    Disable,
    AddToSidebar,
    Copy,
    CopyPath,
    Cut,
    Delete,
    EditLocation,
    Gallery,
    HistoryNext,
    HistoryPrevious,
    ItemDown,
    ItemLeft,
    ItemPageDown,
    ItemPageUp,
    ItemRight,
    ItemUp,
    LocationUp,
    NewFolder,
    Open,
    OpenInNewTab,
    OpenInNewWindow,
    Paste,
    PermanentlyDelete,
    Preview,
    Reload,
    Rename,
    SearchActivate,
    SelectAll,
    SelectFirst,
    SelectLast,
    Settings,
    TabClose,
    TabNew,
    TabNext,
    TabPrev,
    TabViewGrid,
    TabViewList,
    ToggleShowHidden,
    WindowClose,
    WindowNew,
    ZoomDefault,
    ZoomIn,
    ZoomOut,
}

impl ShortcutAction for KeyBindAction {
    fn is_disable(&self) -> bool {
        matches!(self, Self::Disable)
    }
}

impl KeyBindAction {
    fn to_action(self) -> Option<Action> {
        Some(match self {
            Self::Disable => return None,
            Self::AddToSidebar => Action::AddToSidebar,
            Self::Copy => Action::Copy,
            Self::CopyPath => Action::CopyPath,
            Self::Cut => Action::Cut,
            Self::Delete => Action::Delete,
            Self::EditLocation => Action::EditLocation,
            Self::Gallery => Action::Gallery,
            Self::HistoryNext => Action::HistoryNext,
            Self::HistoryPrevious => Action::HistoryPrevious,
            Self::ItemDown => Action::ItemDown,
            Self::ItemLeft => Action::ItemLeft,
            Self::ItemPageDown => Action::ItemPageDown,
            Self::ItemPageUp => Action::ItemPageUp,
            Self::ItemRight => Action::ItemRight,
            Self::ItemUp => Action::ItemUp,
            Self::LocationUp => Action::LocationUp,
            Self::NewFolder => Action::NewFolder,
            Self::Open => Action::Open,
            Self::OpenInNewTab => Action::OpenInNewTab,
            Self::OpenInNewWindow => Action::OpenInNewWindow,
            Self::Paste => Action::Paste,
            Self::PermanentlyDelete => Action::PermanentlyDelete,
            Self::Preview => Action::Preview,
            Self::Reload => Action::Reload,
            Self::Rename => Action::Rename,
            Self::SearchActivate => Action::SearchActivate,
            Self::SelectAll => Action::SelectAll,
            Self::SelectFirst => Action::SelectFirst,
            Self::SelectLast => Action::SelectLast,
            Self::Settings => Action::Settings,
            Self::TabClose => Action::TabClose,
            Self::TabNew => Action::TabNew,
            Self::TabNext => Action::TabNext,
            Self::TabPrev => Action::TabPrev,
            Self::TabViewGrid => Action::TabViewGrid,
            Self::TabViewList => Action::TabViewList,
            Self::ToggleShowHidden => Action::ToggleShowHidden,
            Self::WindowClose => Action::WindowClose,
            Self::WindowNew => Action::WindowNew,
            Self::ZoomDefault => Action::ZoomDefault,
            Self::ZoomIn => Action::ZoomIn,
            Self::ZoomOut => Action::ZoomOut,
        })
    }
}

/// Resolves both maps into the bindings the given mode matches key presses against.
#[must_use]
pub fn key_binds(mode: &tab::Mode, config: &ShortcutsConfig) -> HashMap<KeyBind, Action> {
    let mut binds = config.key_binds(|action| action.to_action());
    binds.retain(|_, action| allowed(*action, mode));
    binds
}

/// Whether a mode can perform an action at all.
///
/// The desktop has no tabs and no settings window; the file chooser dialog neither
/// copies files nor renames them. A binding for something the mode cannot do is
/// dropped rather than left to fire into nothing.
fn allowed(action: Action, mode: &tab::Mode) -> bool {
    match action {
        Action::AddToSidebar
        | Action::OpenInNewTab
        | Action::Settings
        | Action::TabClose
        | Action::TabNew
        | Action::TabNext
        | Action::TabPrev
        | Action::WindowClose
        | Action::WindowNew => matches!(mode, tab::Mode::App),

        Action::Copy
        | Action::CopyPath
        | Action::Cut
        | Action::Delete
        | Action::OpenInNewWindow
        | Action::Paste
        | Action::PermanentlyDelete
        | Action::Rename => matches!(mode, tab::Mode::App | tab::Mode::Desktop),

        Action::EditLocation
        | Action::HistoryNext
        | Action::HistoryPrevious
        | Action::LocationUp
        | Action::SearchActivate => matches!(mode, tab::Mode::App | tab::Mode::Dialog(_)),

        _ => true,
    }
}

/// The bindings the application ships.
///
/// Single latin letters are written upper case, which is how a captured key is
/// spelled: a user rebinding one has to land on this very entry and replace it,
/// not settle beside it.
#[must_use]
pub fn fallback_shortcuts() -> Shortcuts {
    let mut shortcuts = Shortcuts::new();

    macro_rules! bind {
        ([$($modifier:ident),* $(,)?], $key:expr, $action:ident) => {{
            shortcuts.0.insert(
                Binding::new([$(ModifierName::$modifier),*], $key),
                KeyBindAction::$action,
            );
        }};
    }

    // Common keys
    bind!([], "ArrowDown", ItemDown);
    bind!([], "ArrowLeft", ItemLeft);
    bind!([], "ArrowRight", ItemRight);
    bind!([], "ArrowUp", ItemUp);
    bind!([], "F5", Reload);
    bind!([], "Home", SelectFirst);
    bind!([], "End", SelectLast);
    bind!([], "PageDown", ItemPageDown);
    bind!([], "PageUp", ItemPageUp);
    bind!([Shift], "ArrowDown", ItemDown);
    bind!([Shift], "ArrowLeft", ItemLeft);
    bind!([Shift], "ArrowRight", ItemRight);
    bind!([Shift], "ArrowUp", ItemUp);
    bind!([Shift], "Home", SelectFirst);
    bind!([Shift], "End", SelectLast);
    bind!([Shift], "PageDown", ItemPageDown);
    bind!([Shift], "PageUp", ItemPageUp);
    bind!([Ctrl, Shift], "N", NewFolder);
    bind!([], "Enter", Open);
    bind!([Ctrl], "Space", Preview);
    bind!([], "Space", Gallery);

    bind!([Ctrl], "H", ToggleShowHidden);
    bind!([Ctrl], "A", SelectAll);
    bind!([Ctrl], "=", ZoomIn);
    bind!([Ctrl], "+", ZoomIn);
    bind!([Ctrl], "0", ZoomDefault);
    bind!([Ctrl], "-", ZoomOut);
    // Switch view
    bind!([Ctrl], "1", TabViewList);
    bind!([Ctrl], "2", TabViewGrid);

    // App-only keys
    bind!([Ctrl], "D", AddToSidebar);
    bind!([Ctrl], "Enter", OpenInNewTab);
    bind!([Ctrl], ",", Settings);
    bind!([Ctrl], "W", TabClose);
    bind!([Ctrl], "T", TabNew);
    bind!([Ctrl], "Tab", TabNext);
    bind!([Ctrl, Shift], "Tab", TabPrev);
    bind!([Ctrl], "Q", WindowClose);
    bind!([Ctrl], "N", WindowNew);

    // App and desktop only keys
    bind!([Ctrl], "C", Copy);
    bind!([Ctrl, Shift], "C", CopyPath);
    bind!([Ctrl], "X", Cut);
    bind!([], "Delete", Delete);
    bind!([Shift], "Delete", PermanentlyDelete);
    bind!([Shift], "Enter", OpenInNewWindow);
    bind!([Ctrl], "V", Paste);
    bind!([], "F2", Rename);

    // App and dialog only keys
    bind!([Ctrl], "L", EditLocation);
    bind!([Alt], "ArrowRight", HistoryNext);
    bind!([Alt], "ArrowLeft", HistoryPrevious);
    bind!([], "Backspace", HistoryPrevious);
    bind!([Alt], "ArrowUp", LocationUp);
    bind!([Ctrl], "F", SearchActivate);

    shortcuts
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::core::keyboard::key::Named;
    use cosmic::iced::keyboard::Key;

    fn binds(mode: &tab::Mode) -> HashMap<KeyBind, Action> {
        key_binds(mode, &shortcuts_config(Shortcuts::new()))
    }

    #[test]
    fn every_default_binding_resolves() {
        for (binding, action) in fallback_shortcuts().iter() {
            assert!(
                binding.to_key_bind().is_some(),
                "default binding {binding:?} for {action:?} does not resolve to a key"
            );
        }
    }

    /// The global actions the application declares, in the shape `xdgen` writes
    /// them: it drops the blank lines between groups, which the desktop entry
    /// specification allows but which a reader could still trip over.
    #[cfg(feature = "desktop")]
    #[test]
    fn the_declared_global_actions_are_readable() {
        use cosmic::desktop::fde::{DesktopEntry, get_languages_from_env};

        let generated = std::path::Path::new("target/xdgen/fun.wmde.files.desktop");
        let Ok(text) = std::fs::read_to_string(generated) else {
            panic!("build.rs has to have written {}", generated.display());
        };

        let locales = get_languages_from_env();
        let entry = DesktopEntry::from_str(generated, &text, Some(&locales))
            .expect("the generated desktop entry has to parse");

        // The trailing separator the specification asks for comes back as an empty
        // name, so a reader has to drop it rather than look it up.
        let raw: Vec<_> = entry.actions().into_iter().flatten().collect();
        assert_eq!(raw, ["new-window", "trash", "network", ""]);

        let actions: Vec<_> = raw.into_iter().filter(|name| !name.is_empty()).collect();
        for action in actions {
            assert!(
                entry.action_entry(action, "Exec").is_some(),
                "action {action} has no command to run"
            );
            assert!(
                entry
                    .action_entry_localized(action, "Name", &locales)
                    .is_some(),
                "action {action} has no name to show"
            );
        }

        assert_eq!(
            entry.action_entry("trash", "Exec"),
            Some("wmde-files --trash")
        );
    }

    /// The declaration the package ships, which the settings app builds its page
    /// from. It repeats the default bindings above, so it can drift; this is what
    /// makes the drift loud.
    #[test]
    fn the_declaration_matches_the_code() {
        #[derive(serde::Deserialize)]
        struct Declaration {
            groups: Vec<Group>,
        }
        #[derive(serde::Deserialize)]
        struct Group {
            actions: Vec<Declared>,
        }
        #[derive(serde::Deserialize)]
        struct Declared {
            /// Quoted in the file: the settings app reads it as text, since it has
            /// no enum to put it in.
            action: String,
            // Named so that RON has somewhere to put it: an ignored map with string
            // keys is not something it can skip past.
            #[allow(dead_code)]
            label: HashMap<String, String>,
            defaults: Vec<Binding>,
        }

        let path = "res/app-shortcuts/fun.wmde.files.ron";
        let text = std::fs::read_to_string(path).expect("the declaration has to ship");
        let declared: Declaration = ron::from_str(&text).expect("the declaration has to parse");

        let mut from_declaration = Shortcuts::new();
        let mut declared_actions = Vec::new();
        for group in &declared.groups {
            for entry in &group.actions {
                let action: KeyBindAction = ron::from_str(&entry.action)
                    .unwrap_or_else(|_| panic!("{} is not an action", entry.action));
                declared_actions.push(action);
                for binding in &entry.defaults {
                    from_declaration.0.insert(binding.clone(), action);
                }
            }
        }

        assert_eq!(
            from_declaration,
            fallback_shortcuts(),
            "{path} no longer lists the same default bindings as fallback_shortcuts()"
        );

        // Every action the file manager can bind is offered, so that none of them is
        // quietly uneditable.
        for (_, action) in fallback_shortcuts().iter() {
            assert!(
                declared_actions.contains(action),
                "{path} is missing {action:?}"
            );
        }
    }

    #[test]
    fn every_default_key_is_spelled_the_way_capture_spells_it() {
        // A captured key comes back upper case for single latin letters. A default
        // spelled any other way could not be replaced by a user binding: the two
        // would sit side by side in the map and both fire.
        for (binding, action) in fallback_shortcuts().iter() {
            let key = cosmic::shortcuts::key_from_string(&binding.key)
                .unwrap_or_else(|| panic!("{action:?}: key {:?} does not resolve", binding.key));
            let spelled = cosmic::shortcuts::key_to_string(&key)
                .unwrap_or_else(|| panic!("{action:?}: key {:?} has no spelling", binding.key));
            assert_eq!(
                binding.key, spelled,
                "{action:?}: a captured key would be stored as {spelled:?}"
            );
        }
    }

    #[test]
    fn a_mode_only_gets_what_it_can_do() {
        let app = binds(&tab::Mode::App);
        let desktop = binds(&tab::Mode::Desktop);

        assert!(app.values().any(|a| matches!(a, Action::TabNew)));
        assert!(
            !desktop.values().any(|a| matches!(a, Action::TabNew)),
            "the desktop has no tabs"
        );
        assert!(
            desktop.values().any(|a| matches!(a, Action::Copy)),
            "the desktop still copies files"
        );
        assert!(
            desktop.values().any(|a| matches!(a, Action::ItemDown)),
            "arrow keys work everywhere"
        );
    }

    #[test]
    fn a_rebound_action_stays_out_of_a_mode_that_cannot_do_it() {
        let custom = Shortcuts::from_iter([(
            Binding::new([ModifierName::Alt], "T"),
            KeyBindAction::TabNew,
        )]);
        let config = shortcuts_config(custom);

        assert!(
            key_binds(&tab::Mode::App, &config)
                .values()
                .any(|a| matches!(a, Action::TabNew))
        );
        assert!(
            !key_binds(&tab::Mode::Desktop, &config)
                .values()
                .any(|a| matches!(a, Action::TabNew))
        );
    }

    #[test]
    fn a_custom_binding_replaces_the_default_it_lands_on() {
        // Ctrl+T ships as TabNew. Capturing it for something else has to take the
        // old action away, which only happens if both spell the key the same.
        let captured = cosmic::shortcuts::binding_from_key(
            cosmic::iced::keyboard::Modifiers::CTRL,
            &Key::Character("t".into()),
        )
        .expect("Ctrl+T has to capture");
        assert_eq!(captured, Binding::new([ModifierName::Ctrl], "T"));

        let config = shortcuts_config(Shortcuts::from_iter([(captured, KeyBindAction::TabClose)]));
        let binds = key_binds(&tab::Mode::App, &config);

        let ctrl_t = Binding::new([ModifierName::Ctrl], "T")
            .to_key_bind()
            .expect("Ctrl+T has to resolve");
        assert_eq!(binds.get(&ctrl_t), Some(&Action::TabClose));
        assert!(
            !binds.values().any(|a| matches!(a, Action::TabNew)),
            "the default lost its only binding"
        );
    }

    #[test]
    fn disable_takes_a_default_away() {
        let config = shortcuts_config(Shortcuts::from_iter([(
            Binding::new([], "F5"),
            KeyBindAction::Disable,
        )]));

        assert!(
            !key_binds(&tab::Mode::App, &config)
                .values()
                .any(|a| matches!(a, Action::Reload))
        );
        // Named keys are unaffected by the letter-case rule.
        assert_eq!(
            cosmic::shortcuts::binding_from_key(
                cosmic::iced::keyboard::Modifiers::empty(),
                &Key::Named(Named::F5)
            ),
            Some(Binding::new([], "F5"))
        );
    }
}
