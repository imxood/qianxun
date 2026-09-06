import { describe, expect, it } from 'vitest';
import { oscPathToWindows } from './osc';

describe('OSC 7 路径解析', () => {
  it('PowerShell 钩子格式（三斜杠无主机名）', () => {
    expect(oscPathToWindows('file:///C:/Users/maxu')).toBe('C:\\Users\\maxu');
  });

  it('显式 localhost 格式', () => {
    expect(oscPathToWindows('file://localhost/C:/Users/maxu')).toBe('C:\\Users\\maxu');
  });

  it('nushell 格式（真主机名）剥掉主机段', () => {
    expect(oscPathToWindows('file://DESKTOP-ABC/C:/Users/maxu')).toBe('C:\\Users\\maxu');
  });

  it('百分号编码与空格', () => {
    expect(oscPathToWindows('file:///C:/a%20b/%E4%B8%AD%E6%96%87')).toBe('C:\\a b\\中文');
  });

  it('非 file 头与远程共享路径返回 null', () => {
    expect(oscPathToWindows('not-a-file-url')).toBeNull();
    // 主机名段之后不是盘符（如 file://host/share）：保守拒绝。
    expect(oscPathToWindows('file://host/share/dir')).toBeNull();
  });
});
