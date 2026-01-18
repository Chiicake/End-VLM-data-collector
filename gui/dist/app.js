const tauriInvoke = window.__TAURI__?.tauri?.invoke;

const statusPill = document.getElementById("status-pill");
const statusMeta = document.getElementById("status-meta");
const logOutput = document.getElementById("log-output");

const startSessionBtn = document.getElementById("start-session");
const joinSessionBtn = document.getElementById("join-session");
const startPackageBtn = document.getElementById("start-package");
const joinPackageBtn = document.getElementById("join-package");

let sessionId = null;
let packageId = null;

function log(line) {
  const entry = document.createElement("div");
  entry.textContent = line;
  logOutput.appendChild(entry);
  logOutput.scrollTop = logOutput.scrollHeight;
}

function setStatus(label, meta) {
  statusPill.textContent = label;
  statusMeta.textContent = meta;
}

function buildOptions(sessionName) {
  return {
    schema_version: 1,
    capture: {
      api: "WindowsGraphicsCapture",
      fps: 5,
      record_resolution: [1280, 720],
      resize_mode: "letterbox",
      color_format: "BGRA8",
      include_cursor_in_video: false,
      target: {
        method: "gui",
        window_title: null,
        process_name: null,
      },
    },
    input: {
      keyboard: "RawInput",
      mouse: "RawInput",
      mouse_mode: "relative_plus_pointer_mixed",
      dpi_awareness: "PerMonitorV2",
      foreground_only: true,
    },
    timing: {
      clock: "QPC",
      step_ms: 200,
      fps: 5,
    },
    auto_events: {
      enabled: false,
      roi_config: "rois_config_1280x720.json",
      stability_frames: 3,
    }
  };
}

function buildMeta(sessionName) {
  return {
    session_id: sessionName,
    game: "",
    os: "unknown",
    cpu: "unknown",
    gpu: "unknown",
    qpc_frequency_hz: 0,
    build: {
      collector_version: "0.1.0",
      git_commit: "unknown",
    },
    notes: "",
  };
}

function parseHwnd(value) {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith("0x")) {
    return parseInt(trimmed, 16);
  }
  return parseInt(trimmed, 10);
}

async function invokeCommand(command, payload) {
  if (!tauriInvoke) {
    throw new Error("Tauri runtime not detected.");
  }
  return tauriInvoke(command, payload);
}

async function startSession() {
  const datasetRoot = document.getElementById("dataset-root").value.trim();
  const sessionName = document.getElementById("session-name").value.trim();
  const ffmpegPath = document.getElementById("ffmpeg-path").value.trim();
  const hwndValue = parseHwnd(document.getElementById("target-hwnd").value);
  const cursorDebug = document.getElementById("cursor-debug").checked;

  if (!sessionName || !datasetRoot || !ffmpegPath || !Number.isFinite(hwndValue)) {
    log("Missing required session fields.");
    return;
  }

  setStatus("Starting", "Launching session...");
  const config = {
    dataset_root: datasetRoot,
    session_name: sessionName,
    ffmpeg_path: ffmpegPath,
    target_hwnd: hwndValue,
    options: buildOptions(sessionName),
    meta: buildMeta(sessionName),
    cursor_debug: cursorDebug,
  };

  try {
    sessionId = await invokeCommand("start_session", { config });
    log(`Session started: id=${sessionId}`);
    setStatus("Recording", "Session running");
  } catch (err) {
    log(`Start failed: ${err}`);
    setStatus("Idle", "Error starting session");
  }
}

async function pollSession() {
  if (sessionId == null) return;
  const updates = await invokeCommand("poll_session", { id: sessionId });
  updates.forEach((entry) => {
    if (entry.type === "frame") {
      statusMeta.textContent = `Step ${entry.step_index} | fg=${entry.is_foreground}`;
    } else if (entry.type === "finished") {
      log(`Session finished: ${entry.output_dir}`);
      setStatus("Idle", "Ready");
    } else if (entry.type === "error") {
      log(`Session error: ${entry.message}`);
      setStatus("Idle", "Error");
    } else if (entry.type === "started") {
      log(`Session started: ${entry.session_name}`);
    }
  });
}

async function joinSession() {
  if (sessionId == null) return;
  try {
    const outputDir = await invokeCommand("join_session", { id: sessionId });
    log(`Session join completed: ${outputDir}`);
    setStatus("Idle", "Ready");
  } catch (err) {
    log(`Join failed: ${err}`);
  }
}

async function startPackage() {
  const datasetRoot = document.getElementById("dataset-root").value.trim();
  const outputZip = document.getElementById("package-output").value.trim();
  const rawSessions = document.getElementById("package-sessions").value.trim();
  const deleteAfter = document.getElementById("package-delete").checked;
  const sessionNames = rawSessions
    ? rawSessions.split(",").map((name) => name.trim()).filter(Boolean)
    : [];

  if (!datasetRoot || !outputZip) {
    log("Missing packaging fields.");
    return;
  }

  const request = {
    dataset_root: datasetRoot,
    session_names: sessionNames,
    output_zip: outputZip,
    delete_after: deleteAfter,
  };

  try {
    packageId = await invokeCommand("start_package", { request });
    log(`Packaging started: id=${packageId}`);
    setStatus("Packaging", "Compressing sessions...");
  } catch (err) {
    log(`Package failed: ${err}`);
    setStatus("Idle", "Error");
  }
}

async function pollPackage() {
  if (packageId == null) return;
  const updates = await invokeCommand("poll_package", { id: packageId });
  updates.forEach((entry) => {
    if (entry.type === "file") {
      log(`Packed ${entry.index}/${entry.total_files}: ${entry.path}`);
    } else if (entry.type === "finished") {
      log(`Packaging complete: ${entry.output_zip}`);
      setStatus("Idle", "Ready");
    } else if (entry.type === "error") {
      log(`Packaging error: ${entry.message}`);
      setStatus("Idle", "Error");
    }
  });
}

async function joinPackage() {
  if (packageId == null) return;
  try {
    const outputZip = await invokeCommand("join_package", { id: packageId });
    log(`Packaging join completed: ${outputZip}`);
    setStatus("Idle", "Ready");
  } catch (err) {
    log(`Join failed: ${err}`);
  }
}

startSessionBtn.addEventListener("click", startSession);
joinSessionBtn.addEventListener("click", joinSession);
startPackageBtn.addEventListener("click", startPackage);
joinPackageBtn.addEventListener("click", joinPackage);

setInterval(() => {
  pollSession();
  pollPackage();
}, 500);

setStatus("Idle", "Ready");
if (!tauriInvoke) {
  log("Tauri runtime not detected. Open inside the Tauri app.");
}
