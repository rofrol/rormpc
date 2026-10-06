use std::{process::Command, sync::LazyLock};

use rmpc_mpd::address::MpdAddress;

pub static FFMPEG: LazyLock<Dep> = LazyLock::new(|| Dep::new("ffmpeg", "ffmpeg", &["-version"]));
pub static FFPROBE: LazyLock<Dep> = LazyLock::new(|| Dep::new("ffprobe", "ffprobe", &["-version"]));
pub static YTDLP: LazyLock<Dep> = LazyLock::new(|| Dep::new("yt-dlp", "yt-dlp", &["--version"]));
pub static UEBERZUGPP: LazyLock<Dep> =
    LazyLock::new(|| Dep::new("ueberzugpp", "ueberzugpp", &["--version"]));
pub static PYTHON3: LazyLock<Dep> =
    LazyLock::new(|| Dep::new("python3", "python3", &["--version"]));
pub static PYTHON3MUTAGEN: LazyLock<Dep> = LazyLock::new(|| {
    Dep::new("python-mutagen", "python3", &[
        "-c",
        "try:\n\timport mutagen\n\tprint(\"PRESENT\")\nexcept ImportError:\n\tprint(\"NOT PRESENT\")",
    ])
});
pub static CAVA: LazyLock<Dep> = LazyLock::new(|| Dep::new("cava", "cava", &["-v"]));
/// rormpc: the delete menu, Deleted and Versions panes, lyrics and tags run it (rormpc-tools)
pub static MUSICDB: LazyLock<Dep> = LazyLock::new(|| Dep::new("musicdb", "musicdb", &["--version"]));
/// rormpc: the Hits pane runs it (rormpc-tools)
pub static HITS: LazyLock<Dep> = LazyLock::new(|| Dep::new("hits", "hits", &["--version"]));

pub static DEPENDENCIES: [&std::sync::LazyLock<Dep>; 9] =
    [&FFMPEG, &FFPROBE, &YTDLP, &UEBERZUGPP, &PYTHON3, &PYTHON3MUTAGEN, &CAVA, &MUSICDB, &HITS];

/// rormpc: the rormpc-tools release this rormpc expects, read from `scripts/rormpc_install.sh` (its
/// `RORMPC_TOOLS_TAG`), so a release bumps it in one place.
pub static RORMPC_TOOLS_TAG: LazyLock<&'static str> = LazyLock::new(|| {
    include_str!("../../../scripts/rormpc_install.sh")
        .lines()
        .find_map(|l| l.strip_prefix("RORMPC_TOOLS_TAG="))
        .unwrap_or("main")
});

/// rormpc: why a rormpc-tools command (`musicdb`, `hits`) could not run. A missing command gets the install
/// command, since rormpc users don't have it unless they installed rormpc-tools.
pub fn cannot_run(tool: &str, err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        format!(
            "{tool} not found: it comes with rormpc-tools, install it with `uv tool install \
             'git+https://github.com/rofrol/rormpc-tools@{}'` (or scripts/rormpc_install.sh companions)",
            *RORMPC_TOOLS_TAG
        )
    } else {
        format!("cannot run {tool}: {err}")
    }
}

pub fn is_youtube_supported(mpd_address: &MpdAddress) -> Result<(), Vec<String>> {
    let mut unsupported = Vec::new();
    if !YTDLP.installed {
        unsupported.push("yt-dlp".to_string());
    }
    if !FFMPEG.installed {
        unsupported.push("ffmpeg".to_string());
    }
    if !FFPROBE.installed {
        unsupported.push("ffprobe".to_string());
    }
    if !PYTHON3.installed {
        unsupported.push("python3".to_string());
    }
    if !PYTHON3MUTAGEN.installed {
        unsupported.push("python-mutagen".to_string());
    }
    if matches!(mpd_address, MpdAddress::IpAndPort(_)) {
        unsupported.push("socket connection to MPD".to_string());
    }

    if unsupported.is_empty() { Ok(()) } else { Err(unsupported) }
}

pub struct Dep {
    pub name: &'static str,
    pub installed: bool,
    pub version: String,
}

impl Dep {
    fn new(name: &'static str, bin: &'static str, version_args: &'static [&str]) -> Self {
        let mut installed = which::which(bin).is_ok();
        let version = if installed {
            Command::new(bin)
                .args(version_args)
                .output()
                .ok()
                .map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                })
                .unwrap_or_default()
        } else {
            "Version not available".to_string()
        };
        if version == "NOT PRESENT" {
            installed = false;
        }

        Self { name, installed, version }
    }

    pub fn display(&self) -> String {
        format!(
            "{:<20} {:<15} {:<20}",
            self.name,
            if self.installed { "installed" } else { "not installed" },
            self.version
        )
    }

    pub fn log(&self) {
        log::info!(name = self.name, installed = self.installed, version = self.version.as_str(); "Dependency check");
    }
}

#[cfg(test)]
mod tests {
    use super::{RORMPC_TOOLS_TAG, cannot_run};

    #[test]
    fn rormpc_tools_tag_comes_from_the_install_script() {
        let tag = *RORMPC_TOOLS_TAG;
        assert!(tag.starts_with('v') && tag[1..].split('.').count() == 3, "{tag}");
    }

    #[test]
    fn a_missing_tool_gets_the_install_command() {
        let missing = cannot_run("musicdb", &std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(missing.contains(&format!("uv tool install 'git+https://github.com/rofrol/rormpc-tools@{}'", *RORMPC_TOOLS_TAG)), "{missing}");
        let denied = cannot_run("hits", &std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(denied.starts_with("cannot run hits:"), "{denied}");
    }
}
