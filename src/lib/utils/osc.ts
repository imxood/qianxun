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
