import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { IPC_COMMANDS } from './contract';

/**
 * 合同比对测试（架构文档 §5）：解析 Rust 源码里全部 #[tauri::command]
 * 函数名，与 contract.ts 的 IPC_COMMANDS 比对；同时确认 lib.rs 的
 * generate_handler! 注册清单覆盖全部命令。任何一侧漂移都让门禁失败。
 */

const RUST_SRC = join(import.meta.dirname, '..', '..', '..', 'src-tauri', 'src');

function listRustFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      out.push(...listRustFiles(full));
    } else if (entry.endsWith('.rs')) {
      out.push(full);
    }
  }
  return out;
}

function commandFns(source: string): string[] {
  const names: string[] = [];
  // 匹配 `#[tauri::command]` 之后（可跨越 pub、async 等修饰）的第一个 fn。
  const pattern = /#\[tauri::command\][^]*?\bfn\s+([a-z0-9_]+)/g;
  for (const match of source.matchAll(pattern)) {
    const name = match[1];
    if (name) names.push(name);
  }
  return names;
}

function handlerRegistrations(source: string): string[] {
  const block = /generate_handler!\[([^]+?)\]/.exec(source);
  if (!block || block[1] === undefined) return [];
  return [...block[1].matchAll(/([a-z0-9_]+)\s*,/g)]
    .map((m) => m[1]!)
    .filter((name) => name.includes('_'));
}

describe('IPC 合同比对', () => {
  const rustFiles = listRustFiles(RUST_SRC);
  const rustCommands = new Set<string>(
    rustFiles.flatMap((file) => commandFns(readFileSync(file, 'utf8'))),
  );
  const contractCommands = new Set<string>(IPC_COMMANDS);

  it('Rust 命令与 contract.ts 完全一致', () => {
    const missingInTs = [...rustCommands].filter((name) => !contractCommands.has(name));
    const missingInRs = [...contractCommands].filter((name) => !rustCommands.has(name));
    expect(
      {
        rustCommands: [...rustCommands].sort(),
        missingInTs,
        missingInRs,
      },
      `合同漂移：Rust 有而 TS 缺 ${JSON.stringify(missingInTs)}；TS 有而 Rust 缺 ${JSON.stringify(missingInRs)}`,
    ).toEqual({
      rustCommands: [...contractCommands].sort(),
      missingInTs: [],
      missingInRs: [],
    });
  });

  it('generate_handler 注册覆盖全部命令', () => {
    const lib = readFileSync(join(RUST_SRC, 'lib.rs'), 'utf8');
    const registered = new Set(handlerRegistrations(lib));
    const unregistered = [...contractCommands].filter((name) => !registered.has(name));
    expect(unregistered, `未注册进 generate_handler：${JSON.stringify(unregistered)}`).toEqual([]);
  });
});

/**
 * 字段级合同比对：扫描 Rust 端每个 `#[derive(Serialize)] #[serde(rename_all = "camelCase")]`
 * struct 的 `pub <name>:` 字段集合，与 TS 端同名 interface（PascalCase）的字段集合做
 * 双向集合比对。任何一侧改名 / 漏字段都会让门禁失败——守住 docs/07 §2.3「rename_all
 * 不作用字段名」一类 silent 漂移。
 *
 * 已知宽松点：
 * - `#[serde(skip)]` 字段不导出（前端不应看到）；
 * - `Option<T>` 字段两端一致时必出现，TS 端为 `?: T | null`；
 * - 嵌入子结构字段（`Vec<GrepHit>` / `BTreeMap<...>`）只比较本层，不递归。
 * - 字段名约定：Rust `next_file_offset` → TS `nextFileOffset`（由 `rename_all`
 *   转换），所以 TS 端按 camelCase 提取。
 */

/**
 * 进入字段级门禁的类型。
 *
 * 覆盖原则：**凡是跨 IPC 边界传递的结构体都应该在此登记**。漏登记的类型
 * 改名/加字段时不会有任何门禁失败——这正是 docs/18 B4 要补的盲区
 * （此前 `Settings` 这个全应用最核心的类型都不在名单里）。
 */
const RESPONSE_TYPES = [
  'SearchOpen',
  'SearchStatus',
  'FilesPage',
  'GrepPage',
  'GrepProgress',
  'FileHit',
  'GrepHit',
  'GrepOptions',
  'DriveInfo',
  'MoliStatus',
  // 情报域
  'IntelTopicStatus',
  'FeedItem',
  'Article',
  'DigestProgress',
  // TopicDay 的 `items: null` 是「未抓 vs 抓了但空」的唯一载体，必须守住
  'IntelTopicDay',
  // 设置域：全应用最核心的类型，此前完全无字段级保护
  'Settings',
  'SettingsPatch',
  // 插件市场域
  'MarketInstalled',
  'SyncItemResult',
  // 磁盘扫描域：rename_all_fields 历史事故恰好发生在这个域
  'DiskEntry',
  'DiskHome',
] as const;

/**
 * 两侧命名不一致的映射：TS interface 名 → Rust struct 名。
 * 登记在此的类型同样受门禁保护，只是比对时用 Rust 侧的真名。
 */
const RUST_NAME_ALIASES: Record<string, string> = {
  // TS 侧带域前缀（Intel*），Rust 侧 struct 省略前缀
  // Rust: struct TopicDay（intel/commands.rs），TS: IntelTopicDay
  IntelTopicDay: 'TopicDay',
  // Rust: struct InstalledBundle（market/mod.rs），TS: MarketInstalled
  MarketInstalled: 'InstalledBundle',
};

/**
 * Rust 侧没有同名 struct、但 TS 侧仍需声明的类型。
 *
 * `SettingsPatch` 是**纯前端形状**：Rust 的 settings_update 收
 * `serde_json::Value` 再用 `merge()` 递归深合并（settings.rs:749），
 * 不存在对应的具名 struct，因此无法做字段级比对。它能做的门禁是
 * 「顶层键名必须是 Settings 的子集」——见下方子集校验。
 */
const TS_ONLY_TYPES = new Set(['SettingsPatch']);

type StructInfo = { name: string; fields: Set<string>; block: string };

function parseRustStructs(source: string): StructInfo[] {
  const structs: StructInfo[] = [];
  // 匹配 #[derive(... Serialize 或 Deserialize ...)] ... pub struct Name { ... } 块
  const pattern =
    /#\[derive\([\s\S]*?(?:Serialize|Deserialize)[\s\S]*?\)\][\s\S]*?(?:pub\s+)?struct\s+([A-Z][A-Za-z0-9]*)\s*\{([\s\S]*?)\n\}/g;
  for (const match of source.matchAll(pattern)) {
    const name = match[1];
    const body = match[2];
    if (!name || body === undefined) continue;
    const fields = new Set<string>();
    for (const fieldMatch of body.matchAll(
      /(?:#\[serde\([^)]*\)\][\s\S]*?)?pub\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*:/g,
    )) {
      const fieldName = fieldMatch[1];
      if (!fieldName) continue;
      const prefix = fieldMatch[0].slice(0, fieldMatch[0].indexOf('pub'));
      if (/#\[serde\(\s*skip\b/.test(prefix)) continue;
      fields.add(toCamelCase(fieldName));
    }
    structs.push({ name, fields, block: body });
  }
  return structs;
}

function toCamelCase(name: string): string {
  return name.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());
}

function parseTsInterfaces(source: string): Map<string, Set<string>> {
  const out = new Map<string, Set<string>>();
  const pattern = /export\s+interface\s+([A-Z][A-Za-z0-9]*)\s*\{([\s\S]*?)\n\}/g;
  for (const match of source.matchAll(pattern)) {
    const name = match[1];
    const body = match[2];
    if (!name || body === undefined) continue;
    const fields = new Set<string>();
    // 只取**顶层**字段：interface 体里成员固定缩进 2 空格，嵌套对象
    // （如 `tools?: { moli?: … }` 的内层）缩进更深。不做这一层过滤时，
    // 嵌套键会被误当成顶层字段，让 SettingsPatch 的子集校验假报。
    for (const fieldMatch of body.matchAll(/^ {2}([a-zA-Z_][a-zA-Z0-9_]*)\s*[:?]/gm)) {
      const fieldName = fieldMatch[1];
      if (fieldName) fields.add(fieldName);
    }
    out.set(name, fields);
  }
  return out;
}

describe('IPC 字段级合同比对', () => {
  const rustFiles = listRustFiles(RUST_SRC);
  const rustStructs = rustFiles.flatMap((file) => parseRustStructs(readFileSync(file, 'utf8')));
  const contractTs = readFileSync(join(import.meta.dirname, 'contract.ts'), 'utf8');
  const tsInterfaces = parseTsInterfaces(contractTs);

  for (const typeName of RESPONSE_TYPES) {
    it(`${typeName}: Rust 与 TS 字段名集合一致`, () => {
      // 纯 TS 形状（Rust 收 Value 无具名 struct）：退化为子集校验。
      if (TS_ONLY_TYPES.has(typeName)) {
        const ts = tsInterfaces.get(typeName);
        const settings = tsInterfaces.get('Settings');
        expect(ts, `contract.ts 应声明 interface ${typeName}`).toBeDefined();
        expect(settings, '子集校验需要 Settings 作为基准').toBeDefined();
        // patch 的每个顶层键都必须是 Settings 的真实字段：
        // 写错键名会被 Rust 的 merge 静默忽略，前端以为改了其实没改。
        const unknown = [...(ts ?? [])].filter((field) => !settings!.has(field));
        expect(
          unknown,
          `${typeName} 含 Settings 上不存在的键（会被 Rust 静默忽略）：${JSON.stringify(unknown)}`,
        ).toEqual([]);
        return;
      }
      // 两侧命名不一致时按别名查 Rust struct（见 RUST_NAME_ALIASES）。
      const rustName = RUST_NAME_ALIASES[typeName] ?? typeName;
      const rs = rustStructs.find((s) => s.name === rustName);
      const ts = tsInterfaces.get(typeName);
      // Rust 侧没找到 → 不阻断（可能 struct 不在 src-tauri/src 而是 crates/fff-core）
      // 但 TS 侧必须有；若都没有则跳过。
      if (!rs && !ts) return;
      if (!rs) {
        throw new Error(
          `Rust 找不到 #[derive(Serialize)] struct ${rustName}；TS 端仍声明 ${JSON.stringify([...(ts ?? [])])}`,
        );
      }
      if (!ts) {
        throw new Error(
          `contract.ts 没有 interface ${typeName}；Rust 端仍有字段 ${JSON.stringify([...rs.fields])}`,
        );
      }
      const missingInTs = [...rs.fields].filter((f) => !ts.has(f));
      const missingInRs = [...ts].filter((f) => !rs.fields.has(f));
      expect(
        { missingInTs, missingInRs },
        `${typeName} 字段漂移：Rust 有而 TS 缺 ${JSON.stringify(missingInTs)}；TS 有而 Rust 缺 ${JSON.stringify(missingInRs)}`,
      ).toEqual({ missingInTs: [], missingInRs: [] });
    });
  }
});
