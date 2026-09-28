import { describe, expect, it } from 'vitest';

import { getActivePanel, panelStorageKey } from './FloatingPanel.svelte';

describe('panelStorageKey', () => {
  it('统一拼接 qx.mario.panel. 前缀', () => {
    expect(panelStorageKey('hud')).toBe('qx.mario.panel.hud');
    expect(panelStorageKey('mario/panel-1')).toBe('qx.mario.panel.mario/panel-1');
  });
});

describe('getActivePanel', () => {
  it('模块初始(未创建任何面板实例)时无 active 面板', () => {
    expect(getActivePanel()).toBeNull();
  });
});
