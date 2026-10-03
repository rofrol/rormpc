//! rormpc: reopen the tab that was active when rormpc last ran.

use std::path::PathBuf;

use crate::config::tabs::TabName;

/// `$XDG_STATE_HOME/rormpc/last_tab` (default ~/.local/state): runtime state, not config (hand-edited, in git)
/// and not cache (may be wiped).
fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| rmpc_shared::paths::home_dir().map(|h| h.join(".local").join("state")))?;
    let path = state.join("rormpc").join("last_tab");
    // one-time move from where the first version kept it
    if let Some(old) = rmpc_shared::paths::cache_dir().map(|d| d.join("rormpc").join("last_tab")) {
        if old.exists() && !path.exists() {
            let _ = path.parent().map(std::fs::create_dir_all);
            let _ = std::fs::rename(&old, &path);
        }
    }
    Some(path)
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
