//! rormpc: reopen the tab that was active when rormpc last ran (`$XDG_CACHE_HOME/rormpc/last_tab`).

use std::path::PathBuf;

use crate::config::tabs::TabName;

fn path() -> Option<PathBuf> {
    rmpc_shared::paths::cache_dir().map(|dir| dir.join("rormpc").join("last_tab"))
}

/// The saved tab if the config still has it, else the first configured tab.
pub fn initial(names: &[TabName]) -> Option<TabName> {
    let saved = path().and_then(|p| std::fs::read_to_string(p).ok());
    saved
        .and_then(|name| names.iter().find(|t| t.0.as_str() == name.trim()).cloned())
        .or_else(|| names.first().cloned())
}

/// Best effort: a read-only cache dir must not break tab switching.
pub fn save(tab: &TabName) {
    if let Some(p) = path() {
        let _ = p.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(p, tab.0.as_str());
    }
}
