// Mute on Minimize: tells the muteonminimized service which processes have
// all their windows minimized, so it can mute their audio.
//
// Windows are grouped by PID: windows of one process (e.g. a browser and its
// web app windows) share their audio. A process is muted when at least one
// of its windows belongs to a selected application and none of its windows
// is in front: either all are minimized or, with MuteInBackground, another
// window was activated after them on every screen they are on. So switching
// away with Alt+Tab mutes a game, while working on a second monitor does not.
// The service also mutes streams of child processes.
//
// Applications are selected on the settings page, from a window's menu, or
// with a shortcut on the active window.

const SERVICE = "io.github.belebob.MuteOnMinimize";
const PATH = "/io/github/belebob/MuteOnMinimize";
const INTERFACE = "io.github.belebob.MuteOnMinimize1";

let applications = new Set();
let muteInBackground = true;
// Windows by recency of activation, most recent first.
let recent = [];
// Last list sent to the service, as a string; null forces the next send.
let lastSent = null;
// English text -> text in the session language. KWin's JavaScript has no
// i18n functions, so the service reads the script's gettext catalog for us.
let translations = {};

function tr(text) {
    return translations[text] || text;
}

function loadConfig() {
    applications = new Set(
        String(readConfig("Applications", ""))
            .split(/[\s,;]+/)
            .map((name) => name.trim().toLowerCase())
            .filter((name) => name.length > 0)
    );
    muteInBackground = String(readConfig("MuteInBackground", true)) !== "false";
}

// Every name a window can be selected by.
function namesOf(window) {
    return [window.resourceClass, window.resourceName, window.desktopFileName]
        .map((name) => String(name || "").toLowerCase())
        .filter((name) => name.length > 0);
}

// The name stored when a window is selected: its class, as Window Rules
// shows it.
function primaryName(window) {
    return namesOf(window)[0] || "";
}

function isSelected(window) {
    return namesOf(window).some((name) => applications.has(name));
}

// Windows a user can see and restore; panels, the desktop, notifications,
// popups and the like do not count.
function isAppWindow(window) {
    return window.normalWindow || window.dialog || window.utility;
}

function isOnCurrentDesktop(window) {
    const onDesktop = window.onAllDesktops || window.desktops.length === 0 ||
        window.desktops.includes(workspace.currentDesktopForScreen(window.output));
    const inActivity = window.activities.length === 0 ||
        window.activities.includes(workspace.currentActivity);
    return onDesktop && inActivity;
}

// PIDs owning the front window of some screen: the most recently activated
// app window there that is shown.
function frontPids() {
    const screens = new Set();
    const pids = new Set();
    for (const window of recent) {
        if (!isAppWindow(window) || window.minimized || !window.output || !isOnCurrentDesktop(window)) {
            continue;
        }
        if (!screens.has(window.output.name)) {
            screens.add(window.output.name);
            pids.add(window.pid);
        }
    }
    return pids;
}

// `closing` is a window being removed, which may still be listed.
function mutedPids(closing) {
    const front = muteInBackground ? frontPids() : null;
    const byPid = new Map();
    for (const window of workspace.windowList()) {
        if (window === closing || !isAppWindow(window) || !(window.pid > 1)) {
            continue;
        }
        const entry = byPid.get(window.pid) || { selected: false, allMinimized: true };
        entry.selected = entry.selected || isSelected(window);
        entry.allMinimized = entry.allMinimized && window.minimized;
        byPid.set(window.pid, entry);
    }
    const pids = [];
    for (const [pid, entry] of byPid) {
        if (entry.selected && (entry.allMinimized || (front && !front.has(pid)))) {
            pids.push(pid);
        }
    }
    return pids.sort((a, b) => a - b);
}

function update(closing) {
    const pids = mutedPids(closing);
    const key = pids.join(",");
    if (key === lastSent) {
        return;
    }
    lastSent = key;
    callDBus(SERVICE, PATH, INTERFACE, "SetMutedPids", pids);
}

// Selects or deselects a window's application and saves the list.
function toggle(window) {
    if (!window || !isAppWindow(window) || !primaryName(window)) {
        return;
    }
    const wasSelected = isSelected(window);
    if (wasSelected) {
        namesOf(window).forEach((name) => applications.delete(name));
    } else {
        applications.add(primaryName(window));
    }
    // Scripts cannot write their config; the service stores it in kwinrc.
    callDBus(SERVICE, PATH, INTERFACE, "SetApplications", Array.from(applications).join(", "));

    const message = wasSelected ? tr("%1: not muted when minimized") : tr("%1: muted when minimized");
    callDBus("org.kde.plasmashell", "/org/kde/osdService", "org.kde.osdService", "showText",
        wasSelected ? "audio-volume-high" : "audio-volume-muted",
        message.replace("%1", primaryName(window)));
    update();
}

function watch(window) {
    for (const changed of [window.minimizedChanged, window.outputChanged, window.desktopsChanged, window.activitiesChanged]) {
        changed.connect(() => update());
    }
}

loadConfig();
// Until windows get activated, the stacking order is the best guess.
recent = workspace.windowList().slice().sort((a, b) => b.stackingOrder - a.stackingOrder);
workspace.windowList().forEach(watch);
workspace.windowAdded.connect((window) => {
    watch(window);
    update();
});
workspace.windowRemoved.connect((window) => {
    recent = recent.filter((other) => other !== window);
    update(window);
});
workspace.windowActivated.connect((window) => {
    if (window) {
        recent = [window].concat(recent.filter((other) => other !== window));
    }
    update();
});
workspace.currentDesktopChanged.connect(() => update());
workspace.currentActivityChanged.connect(() => update());
options.configChanged.connect(() => {
    loadConfig();
    update();
});

// A checkable entry in the window menu (Alt+F3, or right click on the title
// bar). Its text is built when the menu opens, after translations arrived.
registerUserActionsMenu((window) => {
    if (!isAppWindow(window) || !primaryName(window)) {
        return null;
    }
    return {
        text: tr("Mute when minimized"),
        checkable: true,
        checked: isSelected(window),
        triggered: () => toggle(window),
    };
});

// The shortcut's name is shown in System Settings, so register it once the
// translations are known. Without the service nothing works anyway.
callDBus(SERVICE, PATH, INTERFACE, "Translations", (json) => {
    try {
        translations = JSON.parse(json) || {};
    } catch (error) {
        console.log("Mute on Minimize: bad translations: " + error);
    }
    registerShortcut("MuteOnMinimizeToggle", tr("Mute on Minimize: toggle for the active window"),
        "Meta+Ctrl+M", () => toggle(workspace.activeWindow));
});

// Always send the initial state, even if empty: it clears whatever a previous
// KWin instance left behind.
update();
