// vitest setup — 注册 @testing-library/svelte 的自动 cleanup
// 让每个 test 结束后自动 unmount + 清 DOM, 避免状态污染.
import '@testing-library/svelte/vitest';
