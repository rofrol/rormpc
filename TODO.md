# TODO

## Media keys and Now Playing (outside the TUI)

Hardware media keys (play/pause/next/previous) and the system Now Playing widget work on macOS with
[mpd-now-playable](https://git.00dani.me/00dani/mpd-now-playable), a separate daemon that talks to MPD directly, so
it works while rormpc is closed. Nothing in this repository describes it yet.

Current macOS setup:

    uv tool install mpd-now-playable
    mpd-now-playable install-launchagent            # writes ~/Library/LaunchAgents/me.00dani.mpd-now-playable.plist
                                                    # (RunAtLoad, KeepAlive), bootout + bootstrap + kickstart
    mpd-now-playable uninstall-launchagent          # removes it

The plist runs the uv tool venv's Python (`sys.executable -m mpd_now_playable.cli`), so a reinstall of the tool
or a change of its Python can leave launchd restarting a dead path: rerun `install-launchagent --force` after
`uv tool upgrade` / reinstall.

Plan (2026-10-03, after asking GPT-6.1 Sol and MiMo; both said "document first"):

- [ ] Document it in RORMPC.md ("Media keys" section): what it is, the commands above, how to check it
      (`launchctl print gui/$UID/me.00dani.mpd-now-playable`), the upgrade caveat, and that it is independent
      of rormpc.
- [ ] Do not fold it into `rormpc_install.sh install`: that script installs and rolls back the binary, and a
      binary rollback must not touch the service. If a helper is still wanted, make it a separate opt-in
      subcommand (e.g. `rormpc_install.sh media-keys install|status|uninstall`) that only calls the tool's own
      `install-launchagent` / `uninstall-launchagent`, never writes its own plist, and refuses to run a second
      Now Playing source.
- [ ] Linux: MPD has no MPRIS of its own; a bridge exposes it on the session D-Bus. Options: this
      repository's `rmpcd` (Rust, MPRIS server via zbus, off by default: `config.mpris = true` in
      `~/.config/rmpcd/init.lua`; upstream calls rmpcd early stage), `mpd-mpris` (Go; Arch: `pacman -S
      mpd-mpris`, ships a systemd user unit: `systemctl --user enable --now mpd-mpris`) or `mpDris2`. Run
      exactly one of them. GNOME/KDE map media keys to MPRIS themselves; other WMs need `playerctl` bindings.
  - Omarchy (Arch + Hyprland): `default/hypr/bindings/media.lua` binds XF86AudioPlay/Pause/Next/Prev to
    `omarchy-shell media playPause|next|previous`, a Quickshell service that drives the active MPRIS player.
    Omarchy ships no MPD and no MPD bridge, so the keys reach MPD once one bridge above runs. A playing
    browser or mpv can become the active player and take the keys; for MPD-only keys override the bindings
    with `playerctl -p <name from playerctl -l> play-pause|next|previous`. Try rmpcd first, `mpd-mpris`
    as the fallback.
  - Roguix (Guix, Omarchy look): same Hyprland bindings and `playerctl` are installed; MPD and a bridge are
    not. Guix reportedly packages `mpdris2` (and recently `mpdris2-rs`): check with `guix show mpdris2
    mpdris2-rs` on the pinned channel. Otherwise package rmpcd in roguix-channel. Run it as a Guix Home
    Shepherd service (`home-shepherd-service-type`) and check that it lands on the Hyprland session's D-Bus.
- [ ] Rust instead of the Python tool on macOS (question from 2026-10-03, asked GPT-6.1 Sol and MiMo; both:
      keep mpd-now-playable until a spike passes). The way would be macOS Now Playing support in rmpcd via
      `souvlaki` (MPRIS, macOS MPNowPlayingInfoCenter + MPRemoteCommandCenter, Windows SMTC) or `playwire`
      (newer, same three, claims fixes for souvlaki's playback rate/scrubber and artwork). What is known:
      - The Python tool shows that a bundle-less launchd daemon that plays no audio works: it sets
        `NSApplication` to the accessory activation policy and registers `MPRemoteCommandCenter` handlers
        (`receivers/cocoa/now_playing.py`, via pyobjc), nothing more.
      - Both crates need a running AppKit event loop on the main thread; rmpcd is tokio, so AppKit on the main
        thread, tokio on other threads, channels between them. Days for a prototype, more for parity
        (artwork from MPD `readpicture`/`albumart`, elapsed time and seeking, stale metadata, MPD reconnect).
      - Spike first (about 20 lines): a launchd-started Rust binary without Info.plist that registers
        play/pause and shows a title in Now Playing. If commands do not arrive, stop.
      - Gain: one Rust daemon for macOS and Linux (and Windows SMTC for free later; needs a hidden HWND with a
        message pump), no uv. Cost: owning macOS regressions and drifting rmpcd from upstream (keep it in
        fork-own files, `cfg(target_os = "macos")`).
- [ ] Windows: no plan unless MPD on Windows is actually used; comes almost free with the item above.
- [ ] Whatever is documented, check: works with rormpc closed, survives MPD restart and sleep/wake, clears
      stale metadata when playback stops, uninstall leaves MPD alone.
