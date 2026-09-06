/**
 * OSC 7 的 file:// URL → Windows 路径。覆盖三种在用格式：
 * - `file:///C:/x/y`（PowerShell 钩子，无主机名三斜杠）
 * - `file://localhost/C:/x/y`（显式 localhost）
 * - `file://HOSTNAME/C:/x/y`（nushell：真主机名，非 localhost）
 * 路径部分可能是 `/C:/x`（cygwin 风格前导斜杠）或 `C:/x`。非法输入返回 null。
 */
export function oscPathToWindows(data: string): string | null {
  if (!data.startsWith('file://')) return null;
  let rest = data.slice('file://'.length);
  if (rest.startsWith('localhost/')) {
    rest = rest.slice('localhost/'.length);
  } else if (!rest.startsWith('/')) {
    // 带主机名形式（file://HOST/…）：剥到第一个 '/'。主机名段不含
    // '/'，且合法盘符场景该 '/' 之后紧跟「盘符:」——其它（如远程
    // file://host/share）不剥，保持保守。
    const slash = rest.indexOf('/');
    if (slash > 0 && rest[slash + 1] !== undefined && rest[slash + 2] === ':') {
      rest = rest.slice(slash);
    } else {
      return null;
    }
  }
  let path = rest.startsWith('/') ? rest.slice(1) : rest;
  try {
    path = decodeURIComponent(path);
  } catch {
    // 非法百分号编码：按原样使用。
  }
  return path.replaceAll('/', '\\');
}

/**
 * OSC 9;9（ConEmu「设置当前工作目录」）→ Windows 路径。
 * xterm 的 handler 注册在 identifier 9 上，data 为 `9;9;"D:\path"`。
 * nushell 默认 osc9_9=true（osc7 反而是关的），这是它上报 cwd 的主通道。
 */
export function osc99ToWindows(data: string): string | null {
  if (!data.startsWith('9;9;')) return null;
  let path = data.slice('9;9;'.length).trim();
  if (path.length >= 2 && path.startsWith('"') && path.endsWith('"')) {
    path = path.slice(1, -1);
  }
  return path.length > 0 ? path : null;
}

/**
 * OSC 633（VS Code shell 集成）→ 从属性里取 Cwd。data 形如
 * `P;Cwd=file:///D:/x`（也可能混其它属性，分号分隔）。nushell 默认开。
 */
export function osc633CwdToWindows(data: string): string | null {
  if (!data.startsWith('P;')) return null;
  for (const part of data.slice(2).split(';')) {
    const eq = part.indexOf('=');
    if (eq <= 0 || part.slice(0, eq).toLowerCase() !== 'cwd') continue;
    let value = part.slice(eq + 1);
    if (value.startsWith('file:///')) value = value.slice('file:///'.length);
    else if (value.startsWith('file://')) value = value.slice('file://'.length);
    try {
      value = decodeURIComponent(value);
    } catch {
      // 非法百分号编码：按原样使用。
    }
    return value.replaceAll('/', '\\');
  }
  return null;
}
