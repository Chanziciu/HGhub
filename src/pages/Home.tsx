import { useCallback, useEffect, useRef, useState } from "react";
import Feed from "../components/Feed";
import { api, platform, APP_VERSION, startupFeed } from "../api";
import { invoke } from "@tauri-apps/api/core";
import type { VideoCard } from "../types";
import { useStore } from "../store";
import { ISearch, IVol, IVolMute } from "../icons";

export default function Home({ active, bottomInset }: { active: boolean; bottomInset: number }) {
  const { muted, setMuted, setTab, toast, fs } = useStore();
  const [items, setItems] = useState<VideoCard[]>(startupFeed);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [displayInfo, setDisplayInfo] = useState("");
  const inspectDisplay = async () => {
    const rect = document.getElementById("root")!.getBoundingClientRect();
    const native = await invoke("repair_mobile_layout").catch((error) => String(error));
    setDisplayInfo(JSON.stringify({ version: APP_VERSION, screen: [screen.width, screen.height], available: [screen.availWidth, screen.availHeight], viewport: [innerWidth, innerHeight], visualViewport: window.visualViewport && [visualViewport!.width, visualViewport!.height, visualViewport!.offsetTop, visualViewport!.scale], root: [rect.x, rect.y, rect.width, rect.height], native, userAgent: navigator.userAgent }, null, 2));
  };
  const [done, setDone] = useState(false);
  const loadingRef = useRef(false);

  const load = useCallback(async (p: number) => {
    if (loadingRef.current || done) return;
    loadingRef.current = true;
    setLoading(true);
    setError("");
    try {
      const res = await api.feed(p, 10);
      const list = res?.items || [];
      setItems((prev) => {
        const seen = new Set(prev.map((x) => x.id));
        return [...prev, ...list.filter((x) => !seen.has(x.id))];
      });
      setPage(p);
      if (!list.length || (res?.pagination && p >= res.pagination.pages)) setDone(true);
    } catch (e) {
      const message = (e as Error).message || "加载失败";
      setError(message);
      if (!items.length) toast(message, "info");
    } finally {
      loadingRef.current = false;
      setLoading(false);
    }
  }, [done, toast, items.length]);

  useEffect(() => { load(1); /* eslint-disable-next-line */ }, []);

  const ids = items.map((v) => v.id);

  return (
    <div className="absolute inset-0 bg-black">
      {displayInfo && <div className="absolute inset-0 z-[90] bg-bg p-5 flex flex-col gap-3 text-on" style={{ paddingTop: "calc(var(--sat, 0px) + 12px)" }}>
        <div className="flex justify-between"><strong>显示诊断</strong><button onClick={() => setDisplayInfo("")}>关闭</button></div>
        <p className="text-xs text-mut">请将本页截图发送，便于定位底部空白。</p>
        <pre className="scroll-y text-xs whitespace-pre-wrap break-all">{displayInfo}</pre>
      </div>}
      {items.length ? (
        <Feed videoIds={ids} mode="home" active={active} bottomInset={bottomInset}
          onNearEnd={() => { if (!loading && !done) load(page + 1); }} />
      ) : (
        <div className="absolute inset-0 flex flex-col gap-4 items-center justify-center p-8 text-center text-white">
          {error ? <>
            <p>暂时无法加载首页</p>
            <p className="text-xs text-white/65 break-all">{error}</p>
            <button className="rounded-full bg-primary text-on-primary px-5 py-2" onClick={() => load(1)}>重新加载</button>
          </> : <><div className="w-9 h-9 spinner" /><p className="text-xs text-white/65">正在加载…</p></>}
        </div>
      )}

      {/* 移动端：静音 + 搜索 */}
      {platform === "ios" && <button className="absolute left-3 z-30 rounded-full bg-black/60 px-3 py-2 text-xs text-white" style={{ top: "calc(var(--sat, 0px) + 6px)" }} onClick={inspectDisplay}>显示诊断</button>}
      {platform !== "windows" && !fs && (
        <div className="absolute inset-x-0 z-20 flex items-center px-3 pointer-events-none" style={{ top: "calc(var(--sat, 0px) + 6px)" }}>
          <div className="ml-auto flex items-center gap-2">
            <button onClick={() => setMuted((m) => !m)}
              className="pointer-events-auto w-9 h-9 rounded-full bg-black/40 flex items-center justify-center text-white/90 pressable elev-1">
              {muted ? <IVolMute size={17} /> : <IVol size={17} />}
            </button>
            <button onClick={() => setTab("search")}
              className="pointer-events-auto w-9 h-9 rounded-full bg-black/40 flex items-center justify-center text-white/90 pressable elev-1">
              <ISearch size={18} />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
