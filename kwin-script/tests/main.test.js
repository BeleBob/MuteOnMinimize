// Runs main.js against a fake KWin and checks what it sends to the service.
// Usage: node kwin-script/tests/main.test.js  (or: make check-script)
"use strict";
const vm = require("vm"), fs = require("fs");
function signal() { const h = []; return { connect: (f) => h.push(f), emit: (...a) => h.forEach((f) => f(...a)) }; }
const HDMI = { name: "HDMI-A-1" }, DP = { name: "DP-1" };
let order = 0;
function win(pid, cls, output, extra = {}) {
  return Object.assign({ pid, resourceClass: cls, resourceName: cls, desktopFileName: "", minimized: false,
    normalWindow: true, dialog: false, utility: false, output, desktops: [], onAllDesktops: false, activities: [],
    stackingOrder: order++, minimizedChanged: signal(), outputChanged: signal(), desktopsChanged: signal(), activitiesChanged: signal() }, extra);
}
const game = win(100, "mgsvtpp.exe", HDMI), fox = win(200, "firefox", HDMI), tg = win(300, "telegram", DP);
game.stackingOrder = 10;                      // game on top of stacking
const windows = [fox, tg, game];
const calls = []; const saved = []; const osd = []; let shortcut = null, menu = null;
let config = { Applications: "mgsvtpp.exe", MuteInBackground: "true" };
const desk1 = { name: "1" }, desk2 = { name: "2" };
const ctx = {
  console,
  workspace: { windowList: () => windows, windowAdded: signal(), windowRemoved: signal(), windowActivated: signal(),
    currentDesktopChanged: signal(), currentActivityChanged: signal(), currentActivity: "a1",
    currentDesktopForScreen: () => ctx.workspace.currentDesktop, currentDesktop: desk1, activeWindow: game },
  options: { configChanged: signal() },
  readConfig: (k, d) => (k in config ? config[k] : d),
  callDBus: (svc, path, iface, method, ...args) => {
    if (method === "SetMutedPids") calls.push(args[0]);
    else if (method === "SetApplications") saved.push(args[0]);
    else if (method === "showText") osd.push(args[1]);
    else if (method === "Translations") args[0](JSON.stringify({ "Mute when minimized": "Глушить при сворачивании", "%1: muted when minimized": "%1: глушится при сворачивании" }));
  },
  registerShortcut: (name, text, keys, cb) => { shortcut = { name, text, keys, cb }; },
  registerUserActionsMenu: (cb) => { menu = cb; },
};
vm.runInNewContext(fs.readFileSync(require("path").join(__dirname, "../contents/code/main.js"), "utf8"), ctx);
const W = ctx.workspace;
let fails = 0;
const check = (name, got, want) => { const ok = JSON.stringify(got) === JSON.stringify(want); if (!ok) fails++; console.log((ok ? "ok  " : "FAIL") + " " + name + ": " + JSON.stringify(got)); };
const activate = (w) => { W.activeWindow = w; W.windowActivated.emit(w); };

check("initial: game in front on HDMI", calls.at(-1), []);
activate(fox); check("Alt+Tab to Firefox on same screen -> game muted", calls.at(-1), [100]);
activate(tg);  check("then Telegram on second monitor -> still muted (Firefox in front on HDMI)", calls.at(-1), [100]);
activate(game); check("back to game", calls.at(-1), []);
activate(tg);  check("game active, user goes to second monitor -> sound stays", calls.at(-1), []);
game.minimized = true; game.minimizedChanged.emit(); check("game minimized", calls.at(-1), [100]);
game.minimized = false; game.minimizedChanged.emit(); check("restored (still front on HDMI)", calls.at(-1), []);
W.currentDesktop = desk2; game.desktops = [desk1]; fox.desktops = [desk1]; W.currentDesktopChanged.emit(); check("switched to another desktop", calls.at(-1), [100]);
W.currentDesktop = desk1; W.currentDesktopChanged.emit(); check("back to its desktop", calls.at(-1), []);
config.MuteInBackground = "false"; ctx.options.configChanged.emit(); activate(fox); check("background mode off: Alt+Tab keeps sound", calls.at(-1), []);
config.MuteInBackground = "true"; ctx.options.configChanged.emit(); check("background mode back on", calls.at(-1), [100]);
check("shortcut registered after translations", [shortcut.name, shortcut.keys], ["MuteOnMinimizeToggle", "Meta+Ctrl+M"]);
const item = menu(fox); check("menu item for Firefox", [item.text, item.checked, item.checkable], ["Глушить при сворачивании", false, true]);
item.triggered(); check("menu: Firefox selected, list saved", saved.at(-1), "mgsvtpp.exe, firefox"); check("osd", osd.at(-1), "firefox: глушится при сворачивании");
check("menu now checked", menu(fox).checked, true);
activate(game); check("Firefox (selected) now behind game -> muted", calls.at(-1), [200]);
shortcut.cb(); check("shortcut on active game deselects it", saved.at(-1), "firefox");
check("menu not offered for panels", menu(win(1, "plasmashell", HDMI, { normalWindow: false })), null);
W.windowRemoved.emit(fox); check("Firefox closed (still listed during the signal)", calls.at(-1), []);
windows.splice(windows.indexOf(fox), 1);
// Minimize-only semantics from the first version.
config = { Applications: "firefox, chromium", MuteInBackground: "false" }; ctx.options.configChanged.emit();
const f1 = win(400, "firefox", HDMI), f2 = win(400, "firefox", HDMI), cr = win(500, "chromium", HDMI), pwa = win(500, "chrome-app-xyz", HDMI);
for (const w of [f1, f2, cr, pwa]) { windows.push(w); W.windowAdded.emit(w); }
f1.minimized = true; f1.minimizedChanged.emit(); check("one of two Firefox windows minimized", calls.at(-1), []);
f2.minimized = true; f2.minimizedChanged.emit(); check("both minimized", calls.at(-1), [400]);
cr.minimized = true; cr.minimizedChanged.emit(); check("Chromium minimized but its PWA is open", calls.at(-1), [400]);
pwa.minimized = true; pwa.minimizedChanged.emit(); check("PWA minimized too", calls.at(-1), [400, 500]);
process.exitCode = fails ? 1 : 0;
