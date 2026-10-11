// Mute on Minimize: tells the muteonminimized service which processes have
// all their windows minimized, so it can mute their audio.
//
// Windows are grouped by PID: windows of one process (e.g. a browser and its
// web app windows) share their audio, so a process is muted only when every
// one of its windows is minimized and at least one of them belongs to a
// selected application. The service also mutes streams of child processes.

const SERVICE = "io.github.belebob.MuteOnMinimize";
const PATH = "/io/github/belebob/MuteOnMinimize";
const INTERFACE = "io.github.belebob.MuteOnMinimize1";

let applications = new Set();
// Last list sent to the service, as a string; null forces the next send.
let lastSent = null;

function loadConfig() {
    applications = new Set(
        String(readConfig("Applications", ""))
            .split(/[\s,;]+/)
            .map((name) => name.trim().toLowerCase())
            .filter((name) => name.length > 0)
    );
}

function isSelected(window) {
    return [window.resourceClass, window.resourceName, window.desktopFileName]
        .some((name) => name && applications.has(String(name).toLowerCase()));
}

// Windows a user can see and restore; panels, the desktop, notifications,
// popups and the like do not count.
function isAppWindow(window) {
    return window.normalWindow || window.dialog || window.utility;
}

function mutedPids() {
    const byPid = new Map();
    for (const window of workspace.windowList()) {
        if (!isAppWindow(window) || !(window.pid > 1)) {
            continue;
        }
        const entry = byPid.get(window.pid) || { selected: false, allMinimized: true };
        entry.selected = entry.selected || isSelected(window);
        entry.allMinimized = entry.allMinimized && window.minimized;
        byPid.set(window.pid, entry);
    }
    const pids = [];
    for (const [pid, entry] of byPid) {
        if (entry.selected && entry.allMinimized) {
            pids.push(pid);
        }
    }
    return pids.sort((a, b) => a - b);
}

function update() {
    const pids = mutedPids();
    const key = pids.join(",");
    if (key === lastSent) {
        return;
    }
    lastSent = key;
    callDBus(SERVICE, PATH, INTERFACE, "SetMutedPids", pids);
}

function watch(window) {
    window.minimizedChanged.connect(update);
}

loadConfig();
workspace.windowList().forEach(watch);
workspace.windowAdded.connect((window) => {
    watch(window);
    update();
});
workspace.windowRemoved.connect(update);
options.configChanged.connect(() => {
    loadConfig();
    update();
});

// Always send the initial state, even if empty: it clears whatever a previous
// KWin instance left behind.
update();
