import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { AppProvider } from "./store";
import { invoke } from "@tauri-apps/api/core";
import "./index.css";
import { initMedia, isIOS, mediaPlaybackProbe, logLine } from "./api";

/* 运行期错误统一落到 debug.log，便于自查 */
function report(kind: string, msg: string) {
  invoke("debug_log", { msg: `[${kind}] ${msg}` }).catch(() => {});
}
window.addEventListener("error", (e) => {
  report("error", `${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`);
});
window.addEventListener("unhandledrejection", (e) => {
  report("reject", String((e.reason && (e.reason.stack || e.reason.message)) || e.reason));
});

initMedia().then(async () => {
  const probe = isIOS ? await invoke<{ id?: number; ep?: number; baseline?: boolean } | null>("media_probe_config") : null;
  if (probe) {
    document.getElementById("root")!.textContent = "正在进行实机播放计时…";
    try { await logLine(`[probe] result ${await mediaPlaybackProbe(probe)}`); }
    catch (error) { await logLine(`[probe] failed ${String(error)}`); }
    document.getElementById("root")!.textContent = "实机计时完成";
    return;
  }
  ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <AppProvider>
      <App />
    </AppProvider>
  </React.StrictMode>,
  );
}).catch((error) => {
  report("startup", String(error));
  document.getElementById("root")!.textContent = "播放器初始化失败，请关闭后重新打开应用。";
});
