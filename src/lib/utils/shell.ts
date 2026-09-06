/** 标签默认标题 = 解析后的 shell 名（pwsh / powershell / 自定义路径基名）。 */
export function shellTitle(shell: string): string {
  if (shell === 'auto') return '终端';
  const name = shell.replaceAll('\\', '/').split('/').pop() ?? shell;
  return name.replace(/\.exe$/i, '');
}

/** 路径的末段文件夹名（标签默认标题）：C:\Users\maxu → maxu；C:\ → C:。 */
export function folderName(cwd: string): string {
  const trimmed = cwd.replaceAll('\\', '/').replace(/\/+$/, '');
  return trimmed.split('/').pop() || trimmed;
}
