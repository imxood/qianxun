import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import StandaloneWindow from './StandaloneWindow.svelte';
import OverlayPage from './features/shots/OverlayPage.svelte';
import PinWindow from './features/shots/PinWindow.svelte';
import { standaloneView, WINDOW_LABEL } from './lib/windowEnv';
import { call } from './lib/ipc';
import { getCurrentWindow } from '@tauri-apps/api/window';

const target = document.getElementById('app');
if (!target) {
  // 挂载点由 index.html 提供；它缺失意味着页面被改坏，直接显式失败。
  throw new Error('挂载点 #app 不存在');
}

// hash 路由分流：主窗 / 截屏覆盖窗 / 贴图窗 / 独立窗口 共用同一 bundle。
// 覆盖窗 URL 形如 index.html#/overlay?monitor=0&path=…
// 贴图窗 URL 形如 index.html#/pin?path=…
// 独立窗口 URL 形如 index.html#/standalone/terminal（或 /dsh）。
const hash = window.location.hash;
let app;
if (hash.startsWith('#/overlay')) {
  app = mount(OverlayPage, { target });
} else if (hash.startsWith('#/pin')) {
  app = mount(PinWindow, { target });
} else if (standaloneView() !== null) {
  app = mount(StandaloneWindow, { target });
} else {
  app = mount(App, { target });
}

// 应用已挂载：撤掉 index.html 的静态启动屏（splash 是 #app 的兄弟节点，
// Svelte 不会动它，必须显式移除；此后首帧直接呈现界面）。
document.getElementById('qx-splash')?.remove();

// e2e/调试句柄：Playwright 经 CDP 在任意 webview 里执行 IPC 用。
// 生产无副作用（只是把已有的 call 挂上 window）。
(window as unknown as Record<string, unknown>).__qx = {
  call,
  windowLabel: WINDOW_LABEL,
  windowPos: () => getCurrentWindow().outerPosition(),
  isMaximized: () => getCurrentWindow().isMaximized(),
};

export default app;
