/**
 * 联网搜索纯函数内核（R001 / R001-TUN 修订）。
 *
 * 引擎 URL 模板 + moli 输出 → 结构化 items 的解析器。
 * 零依赖：被 qx-bridge（index.js）与 vitest（websearch.test.ts）共用，
 * 部署时随 index.js 一起落盘到插件目录。
 *
 * 引擎实证结论（2026-09-22，TUN/fake-ip 网络直连）：
 * - bing 网页版对无 cookie 客户端下发反爬空壳页（正文只有页脚链接）→
 *   bing 走 `format=rss` 通道；RSS **必须带 mkt/setlang 市场参数**，否则
 *   中文查询会掉进全球杂烩（各国语言微软社区帖）——实测 mkt=zh-CN&
 *   setlang=zh-hans 后结果全为高相关中文内容；
 * - duckduckgo html 版直连质量最好（默认引擎）；结果链接是
 *   `//duckduckgo.com/l/?uddg=<编码>` 跳转壳 → 解析时解码出真实 URL；
 * - baidu 对无 cookie 客户端硬反爬（读超时/空内容）→ 按产品决策移除。
 */

/** 内置引擎 URL 模板；Q 为 encodeURIComponent 后的查询词。 */
export const ENGINE_TEMPLATES = {
  bing: "https://www.bing.com/search?q={Q}&format=rss&mkt=zh-CN&setlang=zh-hans",
  duckduckgo: "https://html.duckduckgo.com/html/?q={Q}",
  searxng: "{E}/search?q={Q}&format=json",
};

/**
 * 直连 SSRF 守卫：真实内网段黑名单。
 *
 * 刻意不含 198.18.0.0/15（fake-ip 基准段）：TUN/fake-ip 网络下域名会解析
 * 成该段地址，必须放行直连、交给 TUN 接管转发——`--block-private-networks`
 * 的内置表把它当内网拦掉，正是「TUN 网络下 moli 全挂」的根因，因此改用
 * 本自定义表（守卫力等价：loopback/RFC1918/链路本地/CGNAT/ULA 全覆盖）。
 */
export const PRIVATE_BLOCK_CIDRS =
  "0.0.0.0/8,127.0.0.0/8,10.0.0.0/8,100.64.0.0/10,169.254.0.0/16,172.16.0.0/12,192.168.0.0/16,::1,fc00::/7,fe80::/10";

/**
 * 代理模式（可选逃生口）下发给 --http-no-proxy 的绕行表：私有目标绕开
 * 代理走直连，回到上面 block-cidrs 的守卫射程——否则代理会把
 * http://127.0.0.1:port 这类内网地址回环抓回来（守卫只管直连目标）。
 */
export const PRIVATE_NO_PROXY = `localhost,${PRIVATE_BLOCK_CIDRS}`;

/**
 * moli fetch 公共参数。默认直连 + 内网守卫；proxy 为非空且非 "off" 时
 * 追加 --http-proxy（代理侧解析域名），并把私有段加入 no-proxy 绕行表。
 */
export function buildFetchArgs({ proxy, timeoutMs }) {
  const args = [
    "fetch",
    "--dump",
    "markdown",
    "--block-cidrs",
    PRIVATE_BLOCK_CIDRS,
    "--timeout",
    String(timeoutMs),
  ];
  const normalized = String(proxy ?? "").trim();
  if (normalized && !/^off$/i.test(normalized)) {
    args.push("--http-proxy", normalized, "--http-no-proxy", PRIVATE_NO_PROXY);
  }
  return args;
}

/**
 * 拼搜索 URL。engine = 内置 kind（字符串）或 { kind, endpoint } 对象。
 * 非法输入抛错（调用方转用户可见错误）。
 */
export function buildSearchUrl(engine, query) {
  const kind = typeof engine === "string" ? engine : engine?.kind;
  const queryText = String(query ?? "").trim();
  if (!queryText) throw new Error("搜索词不能为空");
  if (typeof kind !== "string" || !(kind in ENGINE_TEMPLATES)) {
    throw new Error(`不支持的引擎类型：${String(kind)}`);
  }
  const encoded = encodeURIComponent(queryText);
  if (kind === "searxng") {
    const endpoint = typeof engine === "object" ? String(engine.endpoint ?? "") : "";
    if (!endpoint.startsWith("https://")) {
      throw new Error("searxng 引擎必须提供 https:// 实例地址");
    }
    return ENGINE_TEMPLATES.searxng
      .replace("{E}", endpoint.replace(/\/+$/, ""))
      .replace("{Q}", encoded);
  }
  return ENGINE_TEMPLATES[kind].replace("{Q}", encoded);
}

/** 结果行来源标记（engine id 或 kind）。 */
function normalizeItem(title, url, snippet, engineLabel) {
  return { title, url, snippet, engine: engineLabel };
}

/** URL 噪音过滤：搜索引擎自身跳转/账号/站内页/法务页脚不算结果。 */
const NOISE_PATTERNS = [
  /\/\/html\.duckduckgo\.com\/\/?$/i,
  /\/\/lite\.duckduckgo\.com/i,
  /duckduckgo\.com\/y\.js/i,
  /duckduckgo\.com\/l\/\//i,
  /bing\.com\/aclick/i,
  /bing\.com\/search\?/i,
  /google\.com\/search/i,
  /go\.microsoft\.com\/fwlink/i,
  /beian\.(miit|mps)\.gov\.cn/i,
  /\/accounts\./i,
  /javascript:/i,
];

function isNoise(url) {
  return NOISE_PATTERNS.some((pattern) => pattern.test(url));
}

/** 解码 DDG 跳转壳：//duckduckgo.com/l/?...uddg=<编码URL> → 真实 URL。 */
function unwrapDdgLink(url) {
  // uddg 常是第一个参数（?uddg=…），不做前置分隔符要求。
  const match = /uddg=([^&\s]+)/.exec(url);
  if (!match) return "";
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return "";
  }
}

/** Markdown 链接行 → { title, url, residue }；url 归一化失败返回 null。 */
function extractLink(line) {
  const match = /\[([^\]]{1,200})\]\((https?:\/\/[^)\s]{1,600}|\/\/[^)\s]{1,600})\)/.exec(line);
  if (!match) return null;
  // 嵌套方括号 = 图片/复合结构（如 DDG 的 favicon 行），不是结果链接。
  if (match[1].includes("[")) return null;
  const title = match[1].trim().replace(/^!/, "");
  let url = match[2];
  if (url.startsWith("//")) url = `https:${url}`;
  if (/duckduckgo\.com\/l\/\?/i.test(url)) {
    url = unwrapDdgLink(url);
    if (!url) return null;
  }
  if (!title || title === url) return null;
  if (isNoise(url)) return null;
  // residue：本行链接闭合括号之后的文本（行内摘要形态）。
  return { title, url, residue: line.slice(match.index + match[0].length) };
}

/** 文本像不像裸 URL（DDG 结果里的第二行是目标站 URL 文本）。 */
function looksLikeUrlText(text) {
  return /^[\w.-]+\.[a-z]{2,}(\/|$)/i.test(text.trim());
}

/**
 * 通用结果页解析（duckduckgo / searxng HTML / 兜底）。
 * 逐行扫描 [title](url) 链接，过滤噪音，按 URL 去重，超过 limit 截断。
 * 摘要：本行链接后的残余文本，或后续数行里的摘要行/裸文本。
 */
export function parseSearchMarkdown(markdown, engineLabel, limit = 10) {
  if (typeof markdown !== "string" || markdown.trim() === "") return [];
  const items = [];
  const seen = new Set();
  const lines = markdown.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const link = extractLink(line);
    if (!link) continue;
    if (seen.has(link.url.replace(/#.*$/, ""))) continue;
    // 摘要：本行链接之后的文本，或后续至多 4 行里的摘要行/裸文本。
    // DDG 的摘要形态是独立的链接行（文本为长句），标题行带 "## " 前缀。
    let snippet = link.residue
      .replace(/^[)\]]+/, "")
      .replace(/^[\s——\-–]+/, "")
      .trim();
    if (!snippet) {
      for (let ahead = 1; ahead <= 4; ahead += 1) {
        const next = lines[index + ahead];
        if (next === undefined) break;
        const text = next.trim();
        if (!text) continue;
        if (text.startsWith("#")) break; // 下一条结果的标题行
        if (!text.startsWith("[")) {
          snippet = text;
          break;
        }
        const inner = extractLink(text);
        if (inner && inner.title.length >= 16 && !looksLikeUrlText(inner.title)) {
          snippet = inner.title;
          break;
        }
        if (ahead >= 4) break;
      }
    }
    seen.add(link.url.replace(/#.*$/, ""));
    items.push(normalizeItem(link.title, link.url, snippet.slice(0, 300), engineLabel));
    if (items.length >= limit) break;
  }
  return items;
}

/** 解码 XML 基本实体（RSS 树段文本）。 */
function decodeXmlEntities(text) {
  return text
    .replace(/&#(\d+);/g, (_, code) => String.fromCodePoint(Number(code)))
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}

/** moli 对 XML 的 markdown dump 会在尾部渲染转义过的 XML 树，从此处切分。 */
const XML_TREE_MARKER = "This XML file does not appear";

/**
 * 解析 bing RSS（format=rss）输出的 XML 树段。
 * moli 会把 RSS 渲染成「压平文本 + 转义 XML 树」；压平文本丢失
 * URL/描述边界，只有树段是可靠结构：\<item\>…\<title\>/<link>/<description\>…。
 */
export function parseBingRss(markdown, engineLabel, limit = 10) {
  if (typeof markdown !== "string" || markdown.trim() === "") return [];
  const markerIndex = markdown.indexOf(XML_TREE_MARKER);
  const tree = markerIndex >= 0 ? markdown.slice(markerIndex) : markdown;
  const items = [];
  const seen = new Set();
  const itemPattern = /\\<item\\>([\s\S]*?)\\<\/item\\>/g;
  // 注意 new RegExp 字符串形态：\\\\ = 字面反斜杠；闭合 tag 的 `/` 前不再加反斜杠
  //（输入是 `\</title\>`，即 `\` `<` `/`，多写会变成期待 `<` 后还有 `\`）。
  const readField = (block, name) => {
    const pattern = new RegExp(`\\\\<${name}\\\\>([\\s\\S]*?)\\\\</${name}\\\\>`);
    const match = pattern.exec(block);
    return match ? decodeXmlEntities(match[1].trim()) : "";
  };
  let match;
  while ((match = itemPattern.exec(tree)) !== null && items.length < limit) {
    const block = match[1];
    const title = readField(block, "title");
    const url = readField(block, "link");
    const snippet = readField(block, "description");
    if (!title || !/^https?:\/\//i.test(url)) continue;
    const key = url.replace(/#.*$/, "");
    if (seen.has(key)) continue;
    seen.add(key);
    items.push(normalizeItem(title, url, snippet.slice(0, 300), engineLabel));
  }
  return items;
}

/**
 * 解析 searxng JSON 输出（format=json 端点 moli 原样落文本）。
 * 非 JSON 形态返回 null，调用方回落通用解析。
 */
export function parseSearxngJson(markdown, engineLabel, limit = 10) {
  if (typeof markdown !== "string") return null;
  const trimmed = markdown.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return null;
  let data;
  try {
    data = JSON.parse(trimmed);
  } catch {
    return null;
  }
  const results = Array.isArray(data) ? data : (data?.results ?? []);
  if (!Array.isArray(results)) return null;
  const items = [];
  const seen = new Set();
  for (const row of results) {
    const title = String(row?.title ?? "").trim();
    const url = String(row?.url ?? "").trim();
    if (!title || !/^https?:\/\//i.test(url)) continue;
    if (isNoise(url)) continue;
    if (seen.has(url)) continue;
    seen.add(url);
    items.push(normalizeItem(title, url, String(row?.content ?? "").slice(0, 300), engineLabel));
    if (items.length >= limit) break;
  }
  return items;
}

/** 把 items 汇总成 Markdown 列表（agent 与 UI 共用的最终形态）。 */
export function itemsToMarkdown(items) {
  if (items.length === 0) return "（无结果）";
  return items
    .map((item) => {
      const snippet = item.snippet ? ` —— ${item.snippet}` : "";
      return `- [${item.title}](${item.url})${snippet}`;
    })
    .join("\n");
}
