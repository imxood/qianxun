import { describe, expect, it } from 'vitest';
import { fileKind, highlightName, splitHighlightedPath, splitPath } from './fileIcon';

describe('fileKind', () => {
  it('按扩展名分桶；大小写不敏感', () => {
    expect(fileKind('src/main.ts')).toBe('code');
    expect(fileKind('README.MD')).toBe('doc');
    expect(fileKind('photo.PNG')).toBe('image');
    expect(fileKind('release.zip')).toBe('archive');
    expect(fileKind('foo')).toBe('other');
  });

  it('无扩展名文件 → other', () => {
    expect(fileKind('Makefile')).toBe('other');
  });
});

describe('splitPath', () => {
  it('正斜杠路径：拆成目录 + 文件名', () => {
    const { directory, name } = splitPath('src/lib/ipc/index.ts');
    expect(directory).toBe('src/lib/ipc/');
    expect(name).toBe('index.ts');
  });

  it('反斜杠路径（Windows）：同样拆分', () => {
    const { directory, name } = splitPath('src\\lib\\ipc\\index.ts');
    expect(directory).toBe('src\\lib\\ipc\\');
    expect(name).toBe('index.ts');
  });

  it('混合分隔符：取最右的分隔符位置', () => {
    const { directory, name } = splitPath('src/lib\\ipc/index.ts');
    expect(directory).toBe('src/lib\\ipc/');
    expect(name).toBe('index.ts');
  });

  it('无分隔符：目录为空，文件名就是原路径', () => {
    expect(splitPath('main.rs')).toEqual({ directory: '', name: 'main.rs' });
  });

  it('单分隔符结尾：目录 = 路径前缀，文件名 = 空', () => {
    const { directory, name } = splitPath('foo/bar/');
    expect(directory).toBe('foo/bar/');
    expect(name).toBe('');
  });
});

describe('splitHighlightedPath', () => {
  it('高亮区间全部在文件名内 → nameOffsets 平移到 0 起点', () => {
    const { directory, name, nameOffsets } = splitHighlightedPath(
      'src/lib/ipc/index.ts',
      // "src/lib/ipc/" = 12 字节；"index" 落在 [12, 17)；"ts" 落在 [18, 20)
      [
        [12, 17],
        [18, 20],
      ],
    );
    expect(directory).toBe('src/lib/ipc/');
    expect(name).toBe('index.ts');
    expect(nameOffsets).toEqual([
      [0, 5],
      [6, 8],
    ]);
  });

  it('高亮区间在目录内 → 被过滤掉', () => {
    const { nameOffsets } = splitHighlightedPath('src/lib/ipc/index.ts', [
      [0, 3], // "src" 在目录里
    ]);
    expect(nameOffsets).toEqual([]);
  });

  it('高亮区间跨目录+文件名 → 仅保留落在文件名内的部分', () => {
    const { nameOffsets } = splitHighlightedPath('src/lib/ipc/index.ts', [
      [10, 14], // 跨目录边界（[12] 是 i），"ndex" 落到文件名内
    ]);
    // [10-12, 14-12] = [-2, 2] → max(0,-2)=0, max(0,2)=2
    // nameBytes=9, 0<9 保留
    expect(nameOffsets).toEqual([[0, 2]]);
  });
});

describe('highlightName', () => {
  it('空高亮区间 → 单段原字符串', () => {
    expect(highlightName('hello', [])).toEqual([{ text: 'hello', matched: false }]);
  });

  it('单段高亮', () => {
    expect(highlightName('hello world', [[0, 5]])).toEqual([
      { text: 'hello', matched: true },
      { text: ' world', matched: false },
    ]);
  });
});
