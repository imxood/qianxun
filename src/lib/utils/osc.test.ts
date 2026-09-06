import { describe, expect, it } from 'vitest';
import { osc633CwdToWindows, osc99ToWindows, oscPathToWindows } from './osc';

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

  it('OSC 9;9（nushell/ConEmu 工作目录）', () => {
    expect(osc99ToWindows('9;9;"D:\\develop\\git\\maxu"')).toBe('D:\\develop\\git\\maxu');
    expect(osc99ToWindows('9;9;D:\\develop')).toBe('D:\\develop');
    expect(osc99ToWindows('9;9;')).toBeNull();
    // 进度条等同族序列（9;4）不误吞。
    expect(osc99ToWindows('9;4;3;50')).toBeNull();
  });

  it('OSC 633（VS Code 集成 Cwd 属性）', () => {
    expect(osc633CwdToWindows('P;Cwd=file:///D:/develop/git')).toBe('D:\\develop\\git');
    expect(osc633CwdToWindows('P;Cwd=D:\\x')).toBe('D:\\x');
    expect(osc633CwdToWindows('P;Prompt=xxx;Cwd=file:///C:/a%20b')).toBe('C:\\a b');
    expect(osc633CwdToWindows('A;D:\\x')).toBeNull();
    expect(osc633CwdToWindows('P;Cwd=')).toBe('');
  });
});
