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
- [ ] Linux: MPD has no MPRIS of its own; a bridge exposes it on the session D-Bus. Options: `mpd-mpris`
      (packaged, ships a systemd user unit: `systemctl --user enable --now mpd-mpris`), `mpDris2`, or this
      repository's `rmpcd` (MPRIS server, off by default: `config.mpris = true` in `~/.config/rmpcd/init.lua`;
      upstream calls rmpcd early stage). Run exactly one of them. GNOME/KDE map media keys to MPRIS
      themselves; tiling WMs need `playerctl` key bindings, pinned to the MPD player (`playerctl -p ...`) so
      the keys do not drive a browser. Test before documenting; no Linux machine here yet.
- [ ] Windows: no plan unless MPD on Windows is actually used. The realistic path is the `souvlaki` crate
      (Windows SMTC, also MPRIS and macOS MPNowPlayingInfoCenter), which needs a hidden HWND with a message
      pump on its own thread (and a main-thread run loop on macOS). Putting souvlaki in rmpcd could replace
      mpd-now-playable on macOS as well, but would mean owning artwork, seek and metadata bugs the Python tool
      handles today.
- [ ] Whatever is documented, check: works with rormpc closed, survives MPD restart and sleep/wake, clears
      stale metadata when playback stops, uninstall leaves MPD alone.
