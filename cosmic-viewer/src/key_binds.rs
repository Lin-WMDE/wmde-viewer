// SPDX-License-Identifier: GPL-3.0-only

use crate::message::{ContextMessage, EditMessage, ViewerMessage};
use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use cosmic::iced::keyboard::key::Physical;
use cosmic::shortcuts::{Binding, ModifierName, ShortcutAction};
use cosmic::{
    iced::keyboard::{Key, Modifiers},
    widget::menu::{Action, key_bind::KeyBind},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use viewer_canvas::CanvasMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuAction {
    OpenFile,
    OpenFolder,
    OpenRecent(usize),
    OpenContaining,
    Save,
    SaveAs,
    Settings,
    ImageDetails,
    Quit,
    Undo,
    Redo,
    RevertAll,
    CopyToClipboard,
    RotateLeft,
    RotateRight,
    ZoomIn,
    ZoomOut,
    ActualSize,
    FitToView,
    Fullscreen,
    SetWallpaper,
    MoveToTrash,
    About,
}

impl MenuAction {
    #[must_use]
    pub const fn message(self) -> ViewerMessage {
        match self {
            // Basic App Actions
            Self::OpenFile => ViewerMessage::OpenFileDialog,
            Self::OpenFolder => ViewerMessage::OpenFolderDialog,
            Self::OpenRecent(idx) => ViewerMessage::OpenRecent(idx),
            Self::OpenContaining => ViewerMessage::OpenContaining,
            Self::CopyToClipboard => ViewerMessage::CopyToClipboard,
            Self::Save => ViewerMessage::Save,
            Self::SaveAs => ViewerMessage::SaveAs,
            Self::Quit => ViewerMessage::CloseRequested,
            // Context Actions
            Self::ImageDetails => ViewerMessage::Context(ContextMessage::ImageDetails),
            Self::About => ViewerMessage::Context(ContextMessage::About),
            // Viewport Actions
            Self::ZoomIn => ViewerMessage::Canvas(CanvasMessage::ZoomIn),
            Self::ZoomOut => ViewerMessage::Canvas(CanvasMessage::ZoomOut),
            Self::ActualSize => ViewerMessage::Canvas(CanvasMessage::ActualSize),
            Self::FitToView => ViewerMessage::Canvas(CanvasMessage::FitToView),
            Self::Fullscreen => ViewerMessage::Canvas(CanvasMessage::Fullscreen),
            // Edit Messages
            Self::RotateLeft => ViewerMessage::Edit(EditMessage::RotateLeft),
            Self::RotateRight => ViewerMessage::Edit(EditMessage::RotateRight),
            Self::Undo => ViewerMessage::Edit(EditMessage::Undo),
            Self::Redo => ViewerMessage::Edit(EditMessage::Redo),
            Self::RevertAll => ViewerMessage::Edit(EditMessage::RevertAll),
            Self::Settings => ViewerMessage::Context(ContextMessage::Settings),
            Self::SetWallpaper => ViewerMessage::SetWallpaper,
            Self::MoveToTrash => ViewerMessage::MoveToTrash,
        }
    }
}

impl Action for MenuAction {
    type Message = ViewerMessage;

    fn message(&self) -> ViewerMessage {
        (*self).message()
    }
}

// WMDE: the bindings go through cosmic::shortcuts, shared with the rest of the stack: the
// shipped map below, someone's own `shortcuts_custom` from the viewer's config laid over it,
// and the declaration in res/app-shortcuts/fun.wmde.Viewer.ron that the settings application
// builds the viewer's page from. The test at the bottom keeps the declaration and the code
// saying the same thing.

/// The stored map: what goes into the `shortcuts_custom` config key.
pub type Shortcuts = cosmic::shortcuts::Shortcuts<KeyBindAction>;

/// An action a key combination can be bound to: the menu actions that make sense on a key, and
/// `Disable`, which takes a shipped binding away instead of running something.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum KeyBindAction {
    Disable,
    OpenFile,
    OpenFolder,
    OpenContaining,
    Save,
    SaveAs,
    Quit,
    Undo,
    Redo,
    RevertAll,
    Copy,
    RotateLeft,
    RotateRight,
    ZoomIn,
    ZoomOut,
    ActualSize,
    FitToView,
    Fullscreen,
    ImageDetails,
    Settings,
}

impl ShortcutAction for KeyBindAction {
    fn is_disable(&self) -> bool {
        matches!(self, Self::Disable)
    }
}

impl KeyBindAction {
    /// The menu action a binding runs, which is also what the menu labels its key with; none for
    /// `Disable`.
    #[must_use]
    pub const fn menu_action(self) -> Option<MenuAction> {
        Some(match self {
            Self::Disable => return None,
            Self::OpenFile => MenuAction::OpenFile,
            Self::OpenFolder => MenuAction::OpenFolder,
            Self::OpenContaining => MenuAction::OpenContaining,
            Self::Save => MenuAction::Save,
            Self::SaveAs => MenuAction::SaveAs,
            Self::Quit => MenuAction::Quit,
            Self::Undo => MenuAction::Undo,
            Self::Redo => MenuAction::Redo,
            Self::RevertAll => MenuAction::RevertAll,
            Self::Copy => MenuAction::CopyToClipboard,
            Self::RotateLeft => MenuAction::RotateLeft,
            Self::RotateRight => MenuAction::RotateRight,
            Self::ZoomIn => MenuAction::ZoomIn,
            Self::ZoomOut => MenuAction::ZoomOut,
            Self::ActualSize => MenuAction::ActualSize,
            Self::FitToView => MenuAction::FitToView,
            Self::Fullscreen => MenuAction::Fullscreen,
            Self::ImageDetails => MenuAction::ImageDetails,
            Self::Settings => MenuAction::Settings,
        })
    }
}

/// The bindings the viewer ships. Latin letters are written upper case, as a captured key is
/// spelled, so rebinding one replaces this very entry instead of settling beside it.
#[must_use]
pub fn fallback_shortcuts() -> Shortcuts {
    use KeyBindAction::{
        ActualSize, Copy, FitToView, Fullscreen, ImageDetails, OpenContaining, OpenFile,
        OpenFolder, Quit, Redo, RevertAll, RotateLeft, RotateRight, Save, SaveAs, Settings, Undo,
        ZoomIn, ZoomOut,
    };
    use ModifierName::{Ctrl, Shift};

    [
        (vec![Ctrl], "O", OpenFile),
        (vec![Ctrl, Shift], "O", OpenFolder),
        // Ctrl+C copies the picture, as in wmde-photos.
        (vec![Ctrl, Shift], "C", OpenContaining),
        (vec![Ctrl], "S", Save),
        (vec![Ctrl, Shift], "S", SaveAs),
        (vec![Ctrl], "Q", Quit),
        (vec![Ctrl], "Z", Undo),
        (vec![Ctrl], "Y", Redo),
        (vec![Ctrl, Shift], "Z", RevertAll),
        (vec![Ctrl], "C", Copy),
        (vec![Ctrl, Shift], "R", RotateLeft),
        (vec![Ctrl], "R", RotateRight),
        // Both spellings of the plus key, with Shift and without.
        (vec![Ctrl], "=", ZoomIn),
        (vec![Ctrl, Shift], "=", ZoomIn),
        (vec![Ctrl], "+", ZoomIn),
        (vec![Ctrl, Shift], "+", ZoomIn),
        (vec![Ctrl], "-", ZoomOut),
        (vec![Ctrl], "1", ActualSize),
        (vec![Ctrl], "0", FitToView),
        (vec![Ctrl, Shift], "F", Fullscreen),
        // F11 as in wmde-photos; libcosmic's own F11 is turned off in `on_fullscreen`.
        (vec![], "F11", Fullscreen),
        (vec![Ctrl], "Space", ImageDetails),
        (vec![Ctrl], ",", Settings),
    ]
    .into_iter()
    .map(|(modifiers, key, action)| (Binding::new(modifiers, key), action))
    .collect()
}

/// The shortcuts part of the viewer's config: the same `fun.wmde.Viewer` entry the rest of the
/// settings live in, read on its own so the subscription can follow the one key.
#[derive(Clone, CosmicConfigEntry, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[version = 2]
pub struct ShortcutsEntry {
    pub shortcuts_custom: Shortcuts,
}

/// The viewer's config entry, the one `CosmicViewer::APP_ID` names.
pub const APP_ID: &str = "fun.wmde.Viewer";

/// Someone's own bindings as stored now; none when the key is missing or unreadable.
#[must_use]
pub fn load_custom() -> Shortcuts {
    let config = match cosmic_config::Config::new(APP_ID, ShortcutsEntry::VERSION) {
        Ok(config) => config,
        Err(err) => {
            tracing::warn!("cannot open the shortcuts config: {err}");
            return Shortcuts::new();
        }
    };
    match ShortcutsEntry::get_entry(&config) {
        Ok(entry) => entry.shortcuts_custom,
        Err((errors, entry)) => {
            // A missing key is how a viewer nobody customised starts.
            for error in &errors {
                let missing = matches!(error, cosmic_config::Error::GetKey(_, io)
                    if io.kind() == std::io::ErrorKind::NotFound);
                if error.is_err() && !missing {
                    tracing::warn!("cannot read shortcuts_custom: {error}");
                }
            }
            entry.shortcuts_custom
        }
    }
}

/// The bindings the viewer matches against and the menu labels its items with: the shipped map
/// with `custom` laid over it.
#[must_use]
pub fn key_binds(custom: Shortcuts) -> HashMap<KeyBind, MenuAction> {
    cosmic::shortcuts::ShortcutsConfig::new(fallback_shortcuts(), custom)
        .key_binds(|action| action.menu_action())
}

/// The message a key press runs, if it is bound. The physical key lets a binding written with a
/// latin letter fire under the Ukrainian layout too.
#[must_use]
pub fn shortcut_message<S: std::hash::BuildHasher>(
    key_binds: &HashMap<KeyBind, MenuAction, S>,
    key: &Key,
    modifiers: Modifiers,
    physical_key: &Physical,
) -> Option<ViewerMessage> {
    // A binding written for the very character pressed wins over a latin one that only shares
    // its physical key: someone's own Ctrl+С under the Ukrainian layout over the shipped Ctrl+C.
    key_binds
        .iter()
        .find(|(bind, _)| bind.matches(modifiers, key, None))
        .or_else(|| {
            key_binds
                .iter()
                .find(|(bind, _)| bind.matches(modifiers, key, Some(physical_key)))
        })
        .map(|(_, action)| action.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(
        binds: &HashMap<KeyBind, MenuAction>,
        keys: &[ModifierName],
        key: &str,
    ) -> Option<MenuAction> {
        binds
            .get(&Binding::new(keys.to_vec(), key).to_key_bind().unwrap())
            .copied()
    }

    #[test]
    fn copy_and_open_containing_have_a_key_each() {
        let binds = key_binds(Shortcuts::new());
        assert_eq!(
            bound(&binds, &[ModifierName::Ctrl], "C"),
            Some(MenuAction::CopyToClipboard)
        );
        assert_eq!(
            bound(&binds, &[ModifierName::Ctrl, ModifierName::Shift], "C"),
            Some(MenuAction::OpenContaining)
        );
        assert_eq!(bound(&binds, &[ModifierName::Ctrl], "X"), None);
        assert_eq!(bound(&binds, &[ModifierName::Ctrl], "V"), None);
    }

    #[test]
    fn someone_s_own_binding_replaces_a_shipped_one() {
        let mut custom = Shortcuts::new();
        custom
            .0
            .insert(Binding::new([ModifierName::Ctrl], "R"), KeyBindAction::Copy);
        let binds = key_binds(custom);
        assert_eq!(
            bound(&binds, &[ModifierName::Ctrl], "R"),
            Some(MenuAction::CopyToClipboard)
        );
    }

    #[test]
    fn the_character_pressed_wins_over_the_physical_key() {
        use cosmic::iced::keyboard::key::Code;
        let mut custom = Shortcuts::new();
        custom.0.insert(
            Binding::new([ModifierName::Ctrl], "с"),
            KeyBindAction::ImageDetails,
        );
        let binds = key_binds(custom);
        let message = shortcut_message(
            &binds,
            &Key::Character("с".into()),
            Modifiers::CTRL,
            &Physical::Code(Code::KeyC),
        );
        assert!(matches!(
            message,
            Some(ViewerMessage::Context(ContextMessage::ImageDetails))
        ));
    }

    #[test]
    fn every_default_key_is_spelled_the_way_capture_spells_it() {
        // A default spelled otherwise could not be replaced by a captured binding: the two would
        // sit side by side in the map and both fire.
        for (binding, action) in fallback_shortcuts().iter() {
            let key = cosmic::shortcuts::key_from_string(&binding.key)
                .unwrap_or_else(|| panic!("{action:?}: key {:?} does not resolve", binding.key));
            let spelled = cosmic::shortcuts::key_to_string(&key)
                .unwrap_or_else(|| panic!("{action:?}: key {:?} has no spelling", binding.key));
            assert_eq!(binding.key, spelled, "{action:?}: captured as {spelled:?}");
        }
    }

    #[test]
    fn someone_s_own_binding_disables_and_adds() {
        let mut custom = Shortcuts::new();
        custom.0.insert(
            Binding::new([ModifierName::Ctrl], "Q"),
            KeyBindAction::Disable,
        );
        custom.0.insert(
            Binding::new([ModifierName::Ctrl], "D"),
            KeyBindAction::ImageDetails,
        );
        let binds = key_binds(custom);
        assert_eq!(bound(&binds, &[ModifierName::Ctrl], "Q"), None);
        assert_eq!(
            bound(&binds, &[ModifierName::Ctrl], "D"),
            Some(MenuAction::ImageDetails)
        );
        assert_eq!(
            bound(&binds, &[ModifierName::Ctrl], "Space"),
            Some(MenuAction::ImageDetails)
        );
    }

    #[test]
    fn a_latin_binding_fires_under_the_ukrainian_layout() {
        use cosmic::iced::keyboard::key::Code;
        let binds = key_binds(Shortcuts::new());
        let message = shortcut_message(
            &binds,
            &Key::Character("с".into()),
            Modifiers::CTRL,
            &Physical::Code(Code::KeyC),
        );
        assert!(matches!(message, Some(ViewerMessage::CopyToClipboard)));
    }

    #[test]
    fn the_stored_shape_round_trips() {
        let mut custom = Shortcuts::new();
        custom.0.insert(
            Binding::new([ModifierName::Ctrl, ModifierName::Shift], "T"),
            KeyBindAction::Copy,
        );
        custom.0.insert(
            Binding::new([ModifierName::Ctrl], "Q"),
            KeyBindAction::Disable,
        );
        let text = ron::to_string(&custom).unwrap();
        assert_eq!(
            text,
            r#"{(modifiers:[Ctrl],key:"Q"):Disable,(modifiers:[Ctrl,Shift],key:"T"):Copy}"#
        );
        assert_eq!(ron::from_str::<Shortcuts>(&text).unwrap(), custom);
    }

    #[test]
    fn the_declaration_matches_the_code() {
        #[derive(serde::Deserialize)]
        struct Declaration {
            config: Target,
            groups: Vec<Group>,
        }
        #[derive(serde::Deserialize)]
        struct Target {
            id: String,
            version: u64,
            key: String,
        }
        #[derive(serde::Deserialize)]
        struct Group {
            actions: Vec<Declared>,
        }
        #[derive(serde::Deserialize)]
        struct Declared {
            // Quoted in the file: the settings application reads it as text.
            action: String,
            // Named so RON has somewhere to put it.
            #[allow(dead_code)]
            label: HashMap<String, String>,
            defaults: Vec<Binding>,
        }

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../res/app-shortcuts/fun.wmde.Viewer.ron"
        );
        let text = std::fs::read_to_string(path).expect("the declaration has to ship");
        let declared: Declaration = ron::from_str(&text).expect("the declaration has to parse");

        // Where the settings application writes is where the viewer reads.
        assert_eq!(declared.config.id, APP_ID);
        assert_eq!(declared.config.version, ShortcutsEntry::VERSION);
        assert_eq!(
            declared.config.version,
            viewer_config::ViewerConfig::VERSION
        );
        assert_eq!(declared.config.key, "shortcuts_custom");

        let mut from_declaration = Shortcuts::new();
        for entry in declared.groups.iter().flat_map(|group| &group.actions) {
            let action: KeyBindAction = ron::from_str(&entry.action)
                .unwrap_or_else(|_| panic!("{} is not an action", entry.action));
            for binding in &entry.defaults {
                from_declaration.0.insert(binding.clone(), action);
            }
        }
        assert_eq!(
            from_declaration,
            fallback_shortcuts(),
            "the shortcuts declaration and the code disagree"
        );
    }

    #[test]
    fn every_default_binding_runs_something() {
        for action in fallback_shortcuts().0.values() {
            assert!(
                action.menu_action().is_some(),
                "{action:?} is bound but runs nothing"
            );
        }
    }
}
