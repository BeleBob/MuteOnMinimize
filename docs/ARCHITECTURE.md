# How MuteOnMinimize works

Two parts work together:

```
 KWin script (JavaScript)                 muteonminimized (Rust, systemd --user)
 ─────────────────────────                ──────────────────────────────────────
 watches windows, decides         D-Bus   watches PipeWire output streams,
 which processes to mute   ─────────────▶ mutes those of the given processes
                                 SetMutedPids(av)
```

Only KWin knows reliably whether windows are minimized or in front on Wayland, and a KWin script
cannot run programs or talk to PipeWire. So the script decides and the service acts.

## Repository layout

| Path | Contents |
|---|---|
| `kwin-script/` | The KWin script package: `metadata.json`, `contents/code/main.js`, settings page (`contents/ui/config.ui`, `contents/config/main.xml`) |
| `kwin-script/tests/` | Tests that run `main.js` against a fake KWin under node |
| `service/` | The Rust service; `data/` holds the systemd user unit and the D-Bus activation file |
| `po/` | Translations (gettext) and `update.sh` |
| `packaging/aur/` | AUR packages for releases and for `main` |
| `Makefile`, `install.sh` | Build and install; per-user install for development |

## KWin script

`kwin-script/contents/code/main.js`.

**Which processes are muted.** Windows are grouped by PID, because windows of one process share its
audio (a browser and its web app windows, for example). A PID is muted when at least one of its windows
belongs to a selected application and none of its windows is *in front*:

- a minimized window is never in front;
- with `MuteInBackground` (on by default), the window in front of a screen is the most recently
  activated shown app window on that screen, on the current virtual desktop and activity.
  Until windows get activated, the stacking order decides. So Alt+Tab to another window on the
  same screen mutes, while working on a second monitor does not;
- without `MuteInBackground`, only "all windows minimized" counts.

Normal windows, dialogs and utility windows count; panels, the desktop and popups do not.

**What is sent.** On every relevant change (minimize, activation, window added or removed, desktop,
activity or screen change, settings) the script computes the full list of PIDs and sends it with
`SetMutedPids`, but only if it differs from the last one. Sending the full state makes restarts of KWin
or the service harmless: the next call puts everything right. The initial state is always sent.

**Selecting applications.**
- The settings page edits the `Applications` key (comma-separated window classes or desktop file names,
  case-insensitive) in `kwinrc`, group `[Script-muteonminimize]`.
- `registerUserActionsMenu` adds a checkable *Mute when minimized* entry to the window menu.
- `registerShortcut` adds *Meta+Ctrl+M* for the active window.
- KWin scripts cannot write their own settings, so the menu and the shortcut call `SetApplications`
  and the service writes `kwinrc` with `kwriteconfig6`. A Plasma OSD (`org.kde.osdService.showText`)
  confirms the change.

**Translations.** KWin's JavaScript has no i18n functions. The script asks the service for its strings
(`Translations`) and registers the shortcut once they arrive, because the shortcut's name is shown in
System Settings.

## D-Bus interface

Bus name `io.github.belebob.MuteOnMinimize`, object `/io/github/belebob/MuteOnMinimize`,
interface `io.github.belebob.MuteOnMinimize1`, on the session bus.

| Method | Description |
|---|---|
| `SetMutedPids(av)` | Replaces the set of processes to mute; their descendants are muted too. `av` because KWin's `callDBus` sends a JS array as int32 variants. PIDs 0 and 1 are rejected. |
| `SetApplications(s)` | Stores the comma-separated application list in the script's settings. |
| `Translations() → s` | The script's strings for the session language, as a JSON object from English text to translated text. |

The service remembers the unique name of the last `SetMutedPids` caller. When that name leaves the bus
(KWin exits or crashes), everything is unmuted.

The service is started by D-Bus activation (`SystemdService=muteonminimized.service`) on the first call.
It claims the bus name before connecting to PipeWire, so the script gets an answer right away at login;
calls made meanwhile are queued.

## Service

`service/src/`: `main.rs` (startup), `dbus.rs` (interface), `audio.rs` (PipeWire), `proctree.rs`
(process tree), `sandbox.rs` (Flatpak), `i18n.rs` (translations), `kwinconfig.rs` (settings).

### Matching streams to processes

The service binds every `Stream/Output/Audio` node. A stream belongs to a muted process when its PID
equals one of the PIDs from the script or descends from one (`/proc/<pid>/stat`). Child processes matter:
Electron and Chromium play audio from a separate audio service process.

The stream's PID is found like this:

1. `application.process.id` on the node. PulseAudio clients (through pipewire-pulse) set it; their
   PipeWire client is pipewire-pulse itself, so the client's PID is useless for them.
2. For Flatpak apps that PID is inside the sandbox's PID namespace (often 2). The node carries
   `pipewire.access.portal.instance_id`; `$XDG_RUNTIME_DIR/.flatpak/<id>/bwrapinfo.json` gives the
   instance's `pid-namespace`, and the process in that namespace whose innermost `NSpid` matches is the
   host process.
3. Otherwise `pipewire.sec.pid` of the stream's client, which is right for native PipeWire clients.

### Muting without side effects

Streams are muted through `softMute` in the node's `Props`, not through `mute`:

- WirePlumber saves `mute` (and volumes) per application in
  `~/.local/state/wireplumber/stream-properties`. An app closed while muted with `mute` would start
  muted next time. `softMute` and `softVolumes` are not saved.
- The user's own `mute` stays untouched, and when we unmute, a stream the user muted stays silent.

The catch: any volume or mute change by the user, and WirePlumber restoring a new stream, clears
`softMute` and `softVolumes`, while the node keeps *reporting* the old values. So reported values cannot
be trusted. Instead, every `Props` change that we did not cause re-applies ours. Each `set_param` of ours
produces exactly one params change on the node; the service counts these echoes and skips them, so
re-applying does not loop. If a node keeps changing without a visible reason, it gives up after a few
attempts.

The node's `info` event signals params changes; `Props` are then enumerated again (`subscribe_params` does
not deliver these changes for stream nodes).

### Fading out

Muting fades out over a second. A 20 ms timer on the PipeWire loop steps `softVolumes` from the user's
volume down to zero along a cubic curve, then sets `softMute`. `softVolumes` replaces the user's volume
rather than scaling it, so each step uses the user's volumes times the fade gain. PipeWire's own
`volumeRamp*` properties have no effect on stream nodes. Unmuting, including going back mid-fade,
restores the user's `softMute` and volumes at once.

New streams of an app that is already muted are muted at once, without a fade, so they never blip.

### Shutdown and errors

On SIGTERM or SIGINT the service gives back every stream it muted and waits for PipeWire to process it
before exiting. The signals are blocked in `main()` before any thread starts: PipeWire receives them
through a signalfd, which only works if no thread accepts them directly.

Only an error on the PipeWire core with `-EPIPE` (connection lost) is fatal. Other core errors are races,
such as asking for the params of a stream the server has just removed, and are only logged.

## Translations

`po/*.po` hold the translations of the script's strings (`tr("…")` in `main.js`) and of the settings page.
`make` compiles them into the script package as `contents/locale/<lang>/LC_MESSAGES/muteonminimize.mo`:

- the settings page is translated by KWin's generic config module through
  `X-KWin-Config-TranslationDomain` in `metadata.json`;
- the service reads the same catalogs (a small `.mo` reader, no dependency) for the language from
  `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` or `LANG`, and hands them to the script.

## Known limitations

- The mixer shows a muted stream as playing, because `softMute` is separate from the user's mute.
- A volume change *during* a fade-out lets one 20–40 ms piece through before the next step.
- *Peek at Desktop* (Meta+D by default) only hides windows; KWin does not let scripts read that state,
  so it does not mute. Minimizing all windows does.
