const tauri = window.__TAURI__;
const isTauri = Boolean(tauri?.core?.invoke);

const state = {
  files: [],
  selectedIndex: 0,
  mode: "add",
  running: false,
  lastOutputPath: null,
};

const fileList = document.querySelector("#file-list");
const queueSubtitle = document.querySelector("#queue-subtitle");
const previewSubtitle = document.querySelector("#preview-subtitle");
const statFormat = document.querySelector("#stat-format");
const statSize = document.querySelector("#stat-size");
const statMode = document.querySelector("#stat-mode");
const modeControl = document.querySelector("#mode-control");
const suffixField = document.querySelector("#suffix-field");
const inputFormat = document.querySelector("#input-format");
const outputFormat = document.querySelector("#output-format");
const csvPanel = document.querySelector("#csv-panel");
const csvLonColumn = document.querySelector("#csv-lon-column");
const csvLatColumn = document.querySelector("#csv-lat-column");
const csvNoHeader = document.querySelector("#csv-no-header");
const addFileButton = document.querySelector("#add-file");
const clearFilesButton = document.querySelector("#clear-files");
const dropZone = document.querySelector("#drop-zone");
const fileInput = document.querySelector("#file-input");
const startButton = document.querySelector("#start-convert");
const progressWrap = document.querySelector("#progress-wrap");
const progressBar = document.querySelector("#progress-bar");
const progressValue = document.querySelector("#progress-value");
const resultSubtitle = document.querySelector("#result-subtitle");
const resultOk = document.querySelector("#result-ok");
const resultSkip = document.querySelector("#result-skip");
const resultFail = document.querySelector("#result-fail");
const logList = document.querySelector("#log-list");
const openOutputButton = document.querySelector("#open-output");
const runtimeBadge = document.querySelector("#runtime-badge");

runtimeBadge.textContent = isTauri ? "Tauri ready" : "Browser preview";

function render() {
  renderFiles();
  renderPreview();
  renderSettings();
}

function renderFiles() {
  fileList.innerHTML = "";
  queueSubtitle.textContent = `${state.files.length} 个文件等待处理`;

  if (state.files.length === 0) {
    const empty = document.createElement("div");
    empty.className = "log-item muted";
    empty.textContent = isTauri ? "队列为空" : "浏览器预览无法读取真实路径，请在 Tauri 中运行。";
    fileList.appendChild(empty);
    return;
  }

  state.files.forEach((file, index) => {
    const row = document.createElement("div");
    row.className = `file-row ${index === state.selectedIndex ? "active" : ""} ${statusClass(file.status)}`;
    row.addEventListener("click", () => {
      state.selectedIndex = index;
      render();
    });

    row.innerHTML = `
      <div class="file-name" title="${escapeHtml(file.path)}">${escapeHtml(file.name)}</div>
      <div class="file-meta">${escapeHtml(file.format)}</div>
      <div class="file-meta">${escapeHtml(file.size)}</div>
      <div class="status-pill ${statusClass(file.status)}">${escapeHtml(file.status)}</div>
      <button class="remove-file" type="button" aria-label="移除 ${escapeHtml(file.name)}">x</button>
    `;

    row.querySelector(".remove-file").addEventListener("click", (event) => {
      event.stopPropagation();
      state.files.splice(index, 1);
      state.selectedIndex = Math.max(0, Math.min(state.selectedIndex, state.files.length - 1));
      resetResults();
      render();
    });

    fileList.appendChild(row);
  });
}

function renderPreview() {
  const file = state.files[state.selectedIndex];
  if (!file) {
    previewSubtitle.textContent = "当前：无文件";
    statFormat.textContent = "-";
    statSize.textContent = "-";
    statMode.textContent = modeLabel();
    return;
  }

  previewSubtitle.textContent = `当前：${file.name}`;
  statFormat.textContent = file.format;
  statSize.textContent = file.size;
  statMode.textContent = modeLabel();
}

function renderSettings() {
  suffixField.value = suffixForMode();
  csvPanel.classList.toggle("hidden", !shouldShowCsvPanel());
  startButton.disabled = state.running || state.files.length === 0 || !isTauri;
  openOutputButton.disabled = !state.lastOutputPath || !isTauri;
  modeControl.querySelectorAll("button").forEach((button) => {
    button.classList.toggle("active", button.dataset.mode === state.mode);
  });
}

function shouldShowCsvPanel() {
  if (inputFormat.value === "csv") return true;
  if (inputFormat.value !== "auto") return false;
  return state.files.some((file) => file.format === "CSV");
}

function suffixForMode() {
  if (state.mode === "add") return "_gcj02";
  if (state.mode === "remove") return "_wgs84";
  return "_converted";
}

function modeLabel() {
  if (state.mode === "add") return "加偏";
  if (state.mode === "remove") return "纠偏";
  return "不偏移";
}

function statusClass(status) {
  if (status === "成功") return "done";
  if (status === "失败") return "error";
  return "";
}

modeControl.addEventListener("click", (event) => {
  const button = event.target.closest("button[data-mode]");
  if (!button || state.running) return;
  state.mode = button.dataset.mode;
  render();
});

inputFormat.addEventListener("change", renderSettings);
outputFormat.addEventListener("change", renderSettings);
csvNoHeader.addEventListener("change", renderSettings);

addFileButton.addEventListener("click", pickFiles);
dropZone.addEventListener("click", pickFiles);
dropZone.addEventListener("keydown", (event) => {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    pickFiles();
  }
});

clearFilesButton.addEventListener("click", () => {
  if (state.running) return;
  state.files = [];
  state.selectedIndex = 0;
  resetResults();
  render();
});

fileInput.addEventListener("change", () => {
  const picked = Array.from(fileInput.files || []);
  addBrowserPreviewFiles(picked);
  fileInput.value = "";
});

startButton.addEventListener("click", startConversion);
openOutputButton.addEventListener("click", openLastOutputDirectory);

async function pickFiles() {
  if (state.running) return;

  if (!isTauri) {
    fileInput.click();
    return;
  }

  try {
    const selected = await tauri.dialog.open({
      multiple: true,
      filters: [
        {
          name: "Track files",
          extensions: ["csv", "gpx", "kml", "gdb"],
        },
      ],
    });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    addPaths(paths);
  } catch (error) {
    showError(`选择文件失败：${stringifyError(error)}`);
  }
}

function addPaths(paths) {
  const existing = new Set(state.files.map((file) => file.path));
  paths.filter(Boolean).forEach((path) => {
    if (existing.has(path)) return;
    const name = baseName(path);
    const format = detectFormat(name);
    state.files.push({
      path,
      name,
      format,
      size: "待转换",
      status: format === "GDB" ? "跳过" : "等待",
    });
    existing.add(path);
  });

  if (state.files.length > 0) {
    state.selectedIndex = state.files.length - 1;
  }
  resetResults();
  render();
}

function addBrowserPreviewFiles(files) {
  files.forEach((file) => {
    state.files.push({
      path: file.name,
      name: file.name,
      format: detectFormat(file.name),
      size: formatSize(file.size),
      status: "预览",
    });
  });
  state.selectedIndex = Math.max(0, state.files.length - 1);
  resetResults();
  render();
}

async function startConversion() {
  if (state.running || state.files.length === 0 || !isTauri) return;

  state.running = true;
  state.lastOutputPath = null;
  resultSubtitle.textContent = "转换中";
  logList.innerHTML = "";
  progressWrap.classList.add("visible");
  progressWrap.setAttribute("aria-hidden", "false");
  setProgress(15);
  state.files = state.files.map((file) => ({ ...file, status: "转换中" }));
  render();

  try {
    const result = await tauri.core.invoke("convert_files", {
      request: {
        files: state.files.map((file) => file.path),
        inputFormat: inputFormat.value,
        outputFormat: outputFormat.value,
        mode: state.mode,
        csvLatColumn: csvLatColumn.value.trim() || null,
        csvLonColumn: csvLonColumn.value.trim() || null,
        csvNoHeader: csvNoHeader.checked,
      },
    });
    setProgress(100);
    applyResult(result);
  } catch (error) {
    setProgress(100);
    showError(`转换失败：${stringifyError(error)}`);
    state.files = state.files.map((file) => ({ ...file, status: "失败" }));
    resultOk.textContent = "0";
    resultSkip.textContent = "0";
    resultFail.textContent = String(state.files.length);
    resultSubtitle.textContent = "转换失败";
  } finally {
    state.running = false;
    render();
  }
}

function applyResult(result) {
  resultOk.textContent = String(result.ok || 0);
  resultSkip.textContent = String(result.skipped || 0);
  resultFail.textContent = String(result.failed || 0);
  resultSubtitle.textContent = "转换完成";
  logList.innerHTML = "";

  const byInput = new Map((result.logs || []).map((log) => [log.input, log]));
  state.files = state.files.map((file) => {
    const log = byInput.get(file.path);
    if (!log) return { ...file, status: "失败" };
    if (log.status === "ok") return { ...file, status: "成功", size: `${log.points} 点` };
    if (log.status === "skipped") return { ...file, status: "跳过" };
    return { ...file, status: "失败" };
  });

  (result.logs || []).forEach((log) => {
    const item = document.createElement("div");
    item.className = `log-item ${log.status === "ok" ? "ok" : log.status === "skipped" ? "muted" : "error"}`;
    item.textContent = log.status === "ok"
      ? `${baseName(log.input)} -> ${log.output}，${log.message}`
      : `${baseName(log.input)}：${log.message}`;
    logList.appendChild(item);
    if (log.output && !state.lastOutputPath) {
      state.lastOutputPath = log.output;
    }
  });

  if (!logList.hasChildNodes()) {
    logList.innerHTML = '<div class="log-item muted">没有转换结果</div>';
  }
}

async function openLastOutputDirectory() {
  if (!state.lastOutputPath || !isTauri) return;
  const dir = dirName(state.lastOutputPath);
  try {
    await tauri.opener.openPath(dir);
  } catch (error) {
    showError(`打开目录失败：${stringifyError(error)}`);
  }
}

function resetResults() {
  state.lastOutputPath = null;
  resultSubtitle.textContent = "等待转换";
  resultOk.textContent = "0";
  resultSkip.textContent = "0";
  resultFail.textContent = "0";
  logList.innerHTML = '<div class="log-item muted">转换日志会显示在这里</div>';
  progressWrap.classList.remove("visible");
  progressWrap.setAttribute("aria-hidden", "true");
  setProgress(0);
}

function setProgress(value) {
  progressBar.style.width = `${value}%`;
  progressValue.textContent = `${value}%`;
}

function showError(message) {
  logList.innerHTML = "";
  const item = document.createElement("div");
  item.className = "log-item error";
  item.textContent = message;
  logList.appendChild(item);
}

function detectFormat(name) {
  const ext = name.split(".").pop().toUpperCase();
  if (["CSV", "GPX", "KML", "GDB"].includes(ext)) return ext;
  return "未知";
}

function formatSize(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "未知";
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function baseName(path) {
  return String(path).split(/[\\/]/).pop() || String(path);
}

function dirName(path) {
  const text = String(path);
  const index = Math.max(text.lastIndexOf("/"), text.lastIndexOf("\\"));
  return index > -1 ? text.slice(0, index) : ".";
}

function stringifyError(error) {
  if (typeof error === "string") return error;
  if (error?.message) return error.message;
  return JSON.stringify(error);
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

render();
