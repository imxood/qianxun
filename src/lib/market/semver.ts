/**
 * 宽松 semver 范围匹配（展示层专用）。
 *
 * 安装的硬门槛在 Rust 安装预检里（semver crate），这里的结论只影响展示：
 * 支持 `^ ~ >= > <= < =`、精确版本、x-range（1.x / 1.2）、`*`、`||` 与空格
 * AND；解析失败返回 null（展示为「未声明/无法解析」）。
 * prerelease 宽松：对当前版本同时试原样与去预发布两种形态（与 Rust 一致）。
 */

interface Version {
  major: number;
  minor: number;
  patch: number;
  pre: string[];
}

const VERSION_RE = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/u;

function parseVersion(text: string): Version | null {
  const match = VERSION_RE.exec(text.trim());
  if (!match) return null;
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    pre: match[4] ? match[4].split('.') : [],
  };
}

function comparePre(a: string[], b: string[]): number {
  if (a.length === 0 && b.length === 0) return 0;
  if (a.length === 0) return 1;
  if (b.length === 0) return -1;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i];
    const y = b[i];
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    const xNumeric = /^\d+$/u.test(x);
    const yNumeric = /^\d+$/u.test(y);
    if (xNumeric && yNumeric) {
      const delta = Number(x) - Number(y);
      if (delta !== 0) return delta;
    } else if (xNumeric !== yNumeric) {
      return xNumeric ? -1 : 1;
    } else if (x !== y) {
      return x < y ? -1 : 1;
    }
  }
  return 0;
}

function compareVersions(a: Version, b: Version): number {
  return a.major - b.major || a.minor - b.minor || a.patch - b.patch || comparePre(a.pre, b.pre);
}

/** 补全部分版本号（1.2 → 1.2.0）。 */
function fill(parsed: PartialVersion): Version {
  return {
    major: parsed.major,
    minor: parsed.minor ?? 0,
    patch: parsed.patch ?? 0,
    pre: parsed.pre ?? [],
  };
}

interface PartialVersion {
  major: number;
  minor?: number;
  patch?: number;
  pre?: string[];
}

const PARTIAL_RE = /^v?(\d+|x|X|\*)(?:\.(\d+|x|X|\*))?(?:\.(\d+|x|X|\*))?(?:-([0-9A-Za-z.-]+))?$/u;

function parsePartial(text: string): PartialVersion | null {
  const match = PARTIAL_RE.exec(text.trim());
  if (!match) return null;
  const segment = (value: string | undefined): number | undefined => {
    if (value === undefined) return undefined;
    if (value === 'x' || value === 'X' || value === '*') return undefined;
    return Number(value);
  };
  const major = segment(match[1]);
  if (major === undefined) return null;
  return {
    major,
    minor: segment(match[2]),
    patch: segment(match[3]),
    pre: match[4] ? match[4].split('.') : [],
  };
}

/** x-range/精确（无操作符子句）：* 全过；1.x 匹配主版本；1.2 匹配前两段。 */
function exactOrXRange(parsed: PartialVersion, version: Version): boolean {
  if (parsed.minor === undefined) return version.major === parsed.major;
  if (parsed.patch === undefined) {
    return version.major === parsed.major && version.minor === parsed.minor;
  }
  const target = fill(parsed);
  return compareVersions(version, target) === 0;
}

function caretUpper(parsed: PartialVersion): Version {
  if (parsed.major > 0) return { major: parsed.major + 1, minor: 0, patch: 0, pre: [] };
  if ((parsed.minor ?? 0) > 0) {
    return { major: 0, minor: (parsed.minor ?? 0) + 1, patch: 0, pre: [] };
  }
  return { major: 0, minor: 0, patch: (parsed.patch ?? 0) + 1, pre: [] };
}

function satisfiesClause(clause: string, version: Version): boolean {
  const match = /^(\^|~|>=|<=|>|<|=)?\s*(.+)$/u.exec(clause.trim());
  if (!match) return true;
  const op = match[1] ?? '';
  const parsed = parsePartial(match[2] ?? '');
  if (!parsed) throw new Error(`无法解析的范围子句：${clause}`);
  const lower = fill(parsed);
  switch (op) {
    case '':
    case '=':
      return exactOrXRange(parsed, version);
    case '^':
      return (
        compareVersions(version, lower) >= 0 && compareVersions(version, caretUpper(parsed)) < 0
      );
    case '~':
      return (
        compareVersions(version, lower) >= 0 &&
        compareVersions(version, { ...lower, minor: lower.minor + 1, patch: 0 }) < 0
      );
    case '>=':
      return compareVersions(version, lower) >= 0;
    case '>':
      return compareVersions(version, lower) > 0;
    case '<=':
      return compareVersions(version, lower) <= 0;
    case '<':
      return compareVersions(version, lower) < 0;
    default:
      throw new Error(`未知操作符：${op}`);
  }
}

/**
 * 宽松判定：`true` 命中、`false` 不命中、`null` 范围无法解析。
 */
export function rangeMatches(range: string, version: string): boolean | null {
  const parsed = parseVersion(version);
  if (!parsed) return null;
  const releaseForm: Version = { ...parsed, pre: [] };
  const alternatives = range
    .split('||')
    .map((part) => part.trim())
    .filter((part) => part !== '');
  if (alternatives.length === 0) return true;
  const test = (target: Version): boolean => {
    try {
      return alternatives.some((alternative) =>
        alternative
          .split(/[\s,]+/u)
          .filter((clause) => clause !== '')
          .every((clause) => satisfiesClause(clause, target)),
      );
    } catch {
      return false;
    }
  };
  try {
    return test(parsed) || test(releaseForm);
  } catch {
    return null;
  }
}
