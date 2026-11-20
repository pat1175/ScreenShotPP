import { keyEventToAccelerator } from "./accelerator.js";
import { shouldNotify } from "./update.js";

const { invoke } = window.__TAURI__.core;
const dialog = window.__TAURI__.dialog;
const { check } = window.__TAURI__.updater;
const { relaunch } = window.__TAURI__.process;
const { openUrl } = window.__TAURI__.opener;
const { getCurrentWindow } = window.__TAURI__.window;

const RELEASES_URL = "https://github.com/pat1175/ScreenShotPP/releases";
const SKIPPED_KEY = "skippedUpdateVersion";

const shortcutBtn = document.getElementById("shortcut");
const folderEl = document.getElementById("folder");
const formatSel = document.getElementById("format");
const langSel = document.getElementById("ocr-language");
const launchAtLogin = document.getElementById("launch-at-login");
const delayedShortcutBtn = document.getElementById("delayed-shortcut");
const captureDelayEl = document.getElementById("capture-delay");
const cancelShortcutBtn = document.getElementById("cancel-shortcut");
const recordShortcutBtn = document.getElementById("record-shortcut");
const recordDelayEl = document.getElementById("record-delay");
const recordCursorCheckbox = document.getElementById("record-cursor");
const recordFpsSel = document.getElementById("record-fps");

let settings = await invoke("get_settings");
render();
document.getElementById("version").textContent = await invoke("app_version");

function render() {
  shortcutBtn.textContent = settings.capture_shortcut;
  delayedShortcutBtn.textContent = settings.delayed_capture_shortcut;
  captureDelayEl.value = settings.capture_delay_secs;
  cancelShortcutBtn.textContent = settings.cancel_shortcut;
  recordShortcutBtn.textContent = settings.record_shortcut;
  recordDelayEl.value = settings.record_delay_secs ?? 0;
  recordCursorCheckbox.checked = !!settings.record_cursor;
  recordFpsSel.value = String(settings.record_fps);
  folderEl.textContent = settings.default_save_folder || "Desktop";
  formatSel.value = settings.default_format;
  langSel.value = settings.ocr_language;
  launchAtLogin.checked = !!settings.launch_at_login;
}

async function persist() {
  try {
    await invoke("update_settings", { newSettings: settings });
  } catch (error) {
    console.error("update_settings failed:", error);
    window.alert("Could not save settings: " + error);
  }
}

// Enregistreur de touches partagé pour les boutons de raccourci.
let recordingField = null; // "capture_shortcut" | "delayed_capture_shortcut" | "cancel_shortcut" | "record_shortcut"
let recordingBtn = null;

function startRecording(field, btn) {
  recordingField = field;
  recordingBtn = btn;
  btn.textContent = "Press a combination…";
}

shortcutBtn.addEventListener("click", () =>
  startRecording("capture_shortcut", shortcutBtn),
);
delayedShortcutBtn.addEventListener("click", () =>
  startRecording("delayed_capture_shortcut", delayedShortcutBtn),
);
cancelShortcutBtn.addEventListener("click", () =>
  startRecording("cancel_shortcut", cancelShortcutBtn),
);
recordShortcutBtn.addEventListener("click", () =>
  startRecording("record_shortcut", recordShortcutBtn),
);

window.addEventListener("keydown", async (event) => {
  if (!recordingField) return;
  event.preventDefault();
  // Pour le champ d'annulation, Escape est une valeur valide ; ailleurs il annule.
  if (event.key === "Escape" && recordingField !== "cancel_shortcut") {
    recordingField = null;
    recordingBtn = null;
    render();
    return;
  }
  const accelerator =
    event.key === "Escape" ? "Escape" : keyEventToAccelerator(event);
  if (!accelerator) return; // attend une vraie touche (pas que des modificateurs)
  const field = recordingField;
  recordingField = null;
  recordingBtn = null;
  settings = { ...settings, [field]: accelerator };
  render();
  await persist();
});

document.getElementById("choose-folder").addEventListener("click", async () => {
  const dir = await dialog.open({ directory: true });
  if (!dir) return;
  settings = { ...settings, default_save_folder: dir };
  render();
  await persist();
});

formatSel.addEventListener("change", async () => {
  settings = { ...settings, default_format: formatSel.value };
  await persist();
});

langSel.addEventListener("change", async () => {
  settings = { ...settings, ocr_language: langSel.value };
  await persist();
});

launchAtLogin.addEventListener("change", async () => {
  settings = { ...settings, launch_at_login: launchAtLogin.checked };
  await persist();
});

captureDelayEl.addEventListener("change", async () => {
  const secs = Math.min(60, Math.max(1, parseInt(captureDelayEl.value, 10) || 3));
  settings = { ...settings, capture_delay_secs: secs };
  render();
  await persist();
});

recordDelayEl.addEventListener("change", async () => {
  const secs = Math.min(60, Math.max(0, parseInt(recordDelayEl.value, 10) || 0));
  settings = { ...settings, record_delay_secs: secs };
  render();
  await persist();
});

recordCursorCheckbox.addEventListener("change", async () => {
  settings = { ...settings, record_cursor: recordCursorCheckbox.checked };
  await persist();
});

recordFpsSel.addEventListener("change", async () => {
  settings = { ...settings, record_fps: parseInt(recordFpsSel.value, 10) };
  await persist();
});

// Mises à jour de l'application via le plugin updater Tauri.
const updateBtn = document.getElementById("check-updates");
const updateStatus = document.getElementById("update-status");
const banner = document.getElementById("update-banner");
const bannerTitle = document.getElementById("update-banner-title");
const bannerStatus = document.getElementById("update-banner-status");
const changelogLink = document.getElementById("update-changelog");
const installBtn = document.getElementById("update-install");
const laterBtn = document.getElementById("update-later");
const skipBtn = document.getElementById("update-skip");
let currentUpdate = null;

function setUpdateStatus(message) {
  updateStatus.textContent = message;
}

function hideBanner() {
  banner.hidden = true;
  currentUpdate = null;
}

async function presentBanner(update) {
  currentUpdate = update;
  bannerTitle.textContent = `Version ${update.version}`;
  bannerStatus.textContent = "";
  banner.hidden = false;
  try {
    const win = getCurrentWindow();
    await win.show();
    await win.setFocus();
  } catch (error) {
    console.error("show window failed:", error);
  }
}

async function installUpdate(update) {
  let downloaded = 0;
  let total = 0;
  installBtn.disabled = true;
  laterBtn.disabled = true;
  skipBtn.disabled = true;
  try {
    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          total = event.data.contentLength ?? 0;
          bannerStatus.textContent = "Downloading…";
          break;
        case "Progress":
          downloaded += event.data.chunkLength ?? 0;
          bannerStatus.textContent = total
            ? `Downloading… ${Math.round((downloaded / total) * 100)}%`
            : "Downloading…";
          break;
        case "Finished":
          bannerStatus.textContent = "Installing…";
          break;
      }
    });
    bannerStatus.textContent = "Restarting…";
    await relaunch();
  } catch (error) {
    console.error("update failed:", error);
    bannerStatus.textContent = "Update failed: " + error;
    installBtn.disabled = false;
    laterBtn.disabled = false;
    skipBtn.disabled = false;
  }
}

// auto : silencieux si rien/erreur/version ignorée. manuel : feedback + ignore le skip.
async function runUpdateCheck({ auto }) {
  if (!auto) {
    updateBtn.disabled = true;
    setUpdateStatus("Checking…");
  }
  try {
    const update = await check();
    const skipped = localStorage.getItem(SKIPPED_KEY);
    if (shouldNotify(update?.version ?? null, skipped, { auto })) {
      await presentBanner(update);
      if (!auto) setUpdateStatus("");
    } else if (!auto) {
      setUpdateStatus(update ? `Version ${update.version} skipped.` : "You're up to date.");
    }
  } catch (error) {
    console.error("update check failed:", error);
    if (!auto) setUpdateStatus("Update failed: " + error);
  } finally {
    if (!auto) updateBtn.disabled = false;
  }
}

updateBtn.addEventListener("click", () => runUpdateCheck({ auto: false }));
installBtn.addEventListener("click", () => { if (currentUpdate) installUpdate(currentUpdate); });
laterBtn.addEventListener("click", hideBanner);
skipBtn.addEventListener("click", () => {
  if (currentUpdate) localStorage.setItem(SKIPPED_KEY, currentUpdate.version);
  hideBanner();
});
changelogLink.addEventListener("click", (event) => {
  event.preventDefault();
  openUrl(RELEASES_URL).catch((error) => console.error("open changelog failed:", error));
});

// Vérification automatique au démarrage (la fenêtre main se charge même cachée).
runUpdateCheck({ auto: true });
