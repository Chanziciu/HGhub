#!/usr/bin/env node
/**
 * 把仓库里跟踪的移动端图标同步进生成的 Android / iOS 工程。
 *
 * 为什么需要：
 *   `src-tauri/gen/` 不入库，CI 和换机器都会重新 `tauri android init` / `tauri ios init`，
 *   而 init 只会写 Tauri 模板自带的默认图标（那个青黄「8」），**不会**读
 *   `src-tauri/icons/android/`、`src-tauri/icons/ios/`。于是本地手动拷过图标的人看着正常，
 *   CI 打出来的包却是默认图标 —— 典型的「works on my machine」。
 *
 * 用法：
 *   node scripts/sync-mobile-icons.mjs
 *   （tauri.conf.json 的 beforeBuildCommand 已自动调用，一般不用手动跑）
 */
import { cpSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauri = join(root, "src-tauri");

/* 安卓：icons/android 是整套 res 目录结构，直接覆盖 */
sync("安卓", join(tauri, "icons", "android"),
  join(tauri, "gen", "android", "app", "src", "main", "res"), "tauri android init");

/* iOS：icons/ios 的文件名与模板 AppIcon.appiconset 一一对应，Contents.json 用模板自带的 */
sync("iOS", join(tauri, "icons", "ios"),
  join(tauri, "gen", "apple", "Assets.xcassets", "AppIcon.appiconset"), "tauri ios init");

function sync(label, src, dest, init) {
  if (!existsSync(src)) {
    console.log(`[icons] ${label}跳过：缺少 ${src}`);
    return;
  }
  if (!existsSync(dest)) {
    console.log(`[icons] ${label}跳过：工程还没生成（先跑 ${init}）`);
    return;
  }
  cpSync(src, dest, { recursive: true, force: true });
  console.log(`[icons] ${label}图标已同步 → ${dest}`);
}
