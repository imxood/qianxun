import { sliceByByteOffsets } from '../../lib/ipc/contract';

/** 文件类型分类（决定结果行图标的颜色语义）。 */
export type FileKind = 'code' | 'doc' | 'image' | 'archive' | 'other';

const CODE = new Set([
  'ts',
  'tsx',
  'js',
  'jsx',
  'mjs',
  'cjs',
  'svelte',
  'vue',
  'rs',
  'go',
  'py',
  'java',
  'kt',
  'c',
  'h',
  'cpp',
  'hpp',
  'cs',
  'rb',
  'php',
  'swift',
  'sh',
  'ps1',
  'bat',
  'json',
  'yaml',
  'yml',
  'toml',
  'ini',
  'html',
  'css',
  'scss',
  'less',
  'sql',
  'proto',
]);
const DOC = new Set(['md', 'txt', 'pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'csv']);
const IMAGE = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'ico', 'bmp']);
const ARCHIVE = new Set(['zip', 'rar', '7z', 'gz', 'tar', 'bz2']);

export function fileKind(path: string): FileKind {
  const extension = path.split('.').pop()?.toLowerCase() ?? '';
  if (CODE.has(extension)) return 'code';
  if (DOC.has(extension)) return 'doc';
  if (IMAGE.has(extension)) return 'image';
  if (ARCHIVE.has(extension)) return 'archive';
  return 'other';
}

const KIND_CLASS: Record<FileKind, string> = {
  code: 'text-sky-500 dark:text-sky-400',
  doc: 'text-amber-500 dark:text-amber-400',
  image: 'text-fuchsia-500 dark:text-fuchsia-400',
  archive: 'text-orange-500 dark:text-orange-400',
  other: 'text-muted',
};

/** 结果行左侧的类型图标（颜色按扩展名分类）。 */
export function fileIconClass(path: string): string {
  return KIND_CLASS[fileKind(path)];
}

/** 通用文件轮廓图标。 */
export const FILE_ICON_PATH = 'M14 3H7a2 2 0 00-2 2v14a2 2 0 002 2h10a2 2 0 002-2V8zM14 3v5h5';

const encoder = new TextEncoder();

/** 相对路径拆成目录（含尾分隔符）与文件名。 */
export function splitPath(path: string): { directory: string; name: string } {
  const separator = path.lastIndexOf('/');
  return separator >= 0
    ? { directory: path.slice(0, separator + 1), name: path.slice(separator + 1) }
    : { directory: '', name: path };
}

/**
 * 把「整条相对路径」上的字节高亮区间拆成 目录（暗淡）+ 文件名（高亮）两段。
 * 后端给的 offsets 是 UTF-8 字节偏移，切到文件名时要先按目录的字节长度平移。
 */
export function splitHighlightedPath(
  path: string,
  offsets: Array<[number, number]>,
): { directory: string; name: string; nameOffsets: Array<[number, number]> } {
  const { directory, name } = splitPath(path);
  const nameStartByte = encoder.encode(directory).length;
  const nameBytes = encoder.encode(name).length;
  const nameOffsets = offsets
    .map(
      ([start, end]) =>
        [Math.max(0, start - nameStartByte), Math.max(0, end - nameStartByte)] as [number, number],
    )
    .filter(([start, end]) => end > 0 && start < nameBytes);
  return { directory, name, nameOffsets };
}

/** 文件名按高亮区间切段（复用 IPC 合同里的字节偏移工具）。 */
export function highlightName(name: string, nameOffsets: Array<[number, number]>) {
  return sliceByByteOffsets(name, nameOffsets);
}
