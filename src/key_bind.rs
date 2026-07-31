use cosmic::widget::menu::key_bind::KeyBind;
use std::collections::HashMap;

use crate::app::Action;
use crate::shortcuts::ShortcutsConfig;
use crate::tab;

pub fn key_binds(mode: &tab::Mode, shortcuts: &ShortcutsConfig) -> HashMap<KeyBind, Action> {
    crate::shortcuts::key_binds(mode, shortcuts)
}
