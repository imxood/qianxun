import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

// 控制台是单 bundle 的纯 SPA（docs/20 §7.4 决策 1）：没有 SSR、没有路由库，
// 三页由 App.svelte 里的状态机切换。
const target = document.getElementById('app');
if (!target) throw new Error('#app 缺失');

export default mount(App, { target });
