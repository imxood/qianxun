import { describe, expect, it } from "vitest";
import {
  buildFetchArgs,
  buildSearchUrl,
  itemsToMarkdown,
  parseBingRss,
  parseSearchMarkdown,
  parseSearxngJson,
  PRIVATE_BLOCK_CIDRS,
  PRIVATE_NO_PROXY,
} from "./websearch.js";
import type { SearchItem } from "./websearch.js";

/**
 * websearch 纯函数真测（R001 实施步骤 B / R001-TUN 修订）。
 * fixture 按真实 moli --dump markdown 输出形态提炼（2026-09-22 实测）：
 * - bing 网页版反爬空壳页只有页脚链接 → bing 改走 format=rss + XML 树解析；
 * - duckduckgo html 版结果链接是 //duckduckgo.com/l/?uddg=<编码> 跳转壳；
 * - 直连守卫用自定义 block-cidrs（不含 fake-ip 段 198.18.0.0/15）。
 */

describe("buildSearchUrl", () => {
  it("内置引擎替换查询占位并编码", () => {
    // bing 走 RSS 通道且必须带中国市场参数：无 mkt/setlang 时中文查询
    // 会掉进全球杂烩（实测各国语言微软社区帖）。
    expect(buildSearchUrl("bing", "rust 入门指南")).toBe(
      "https://www.bing.com/search?q=rust%20%E5%85%A5%E9%97%A8%E6%8C%87%E5%8D%97&format=rss&mkt=zh-CN&setlang=zh-hans",
    );
    expect(buildSearchUrl("duckduckgo", "a&b")).toBe(
      "https://html.duckduckgo.com/html/?q=a%26b",
    );
  });

  it("baidu 已移除：不再受支持", () => {
    expect(() => buildSearchUrl("baidu", "q")).toThrow(/不支持的引擎/);
  });

  it("searxng 用 endpoint 且去掉尾部斜杠", () => {
    expect(buildSearchUrl({ kind: "searxng", endpoint: "https://searx.example.com/" }, "q")).toBe(
      "https://searx.example.com/search?q=q&format=json",
    );
  });

  it("searxng 缺 https endpoint 抛错", () => {
    expect(() => buildSearchUrl({ kind: "searxng", endpoint: "http://x" }, "q")).toThrow(
      /https/,
    );
  });

  it("空查询与未知引擎抛错", () => {
    expect(() => buildSearchUrl("bing", "   ")).toThrow(/搜索词/);
    expect(() => buildSearchUrl("google", "q")).toThrow(/不支持的引擎/);
  });
});

describe("buildFetchArgs", () => {
  it("默认直连：block-cidrs 内网守卫（不含 fake-ip 段），无代理参数", () => {
    const base = [
      "fetch",
      "--dump",
      "markdown",
      "--block-cidrs",
      PRIVATE_BLOCK_CIDRS,
      "--timeout",
      "20000",
    ];
    expect(buildFetchArgs({ proxy: "", timeoutMs: 20_000 })).toEqual(base);
    expect(buildFetchArgs({ proxy: undefined, timeoutMs: 20_000 })).toEqual(base);
    expect(buildFetchArgs({ proxy: "off", timeoutMs: 20_000 })).toEqual(base);
    // fake-ip 段（198.18.0.0/15）绝不能进守卫表：TUN 下域名解析成该段，
    // 拦掉 = 联网全挂（本次修复的根因）。
    expect(PRIVATE_BLOCK_CIDRS).not.toContain("198.18");
    // 真内网段必须全覆盖（SSRF 守卫力等价 --block-private-networks）。
    for (const part of ["127.0.0.0/8", "10.0.0.0/8", "192.168.0.0/16", "169.254.0.0/16", "::1"]) {
      expect(PRIVATE_BLOCK_CIDRS).toContain(part);
    }
  });

  it("配置代理（可选逃生口）：--http-proxy + 私有段 no-proxy 绕行", () => {
    expect(buildFetchArgs({ proxy: "socks5://127.0.0.1:1080", timeoutMs: 20_000 })).toEqual([
      "fetch",
      "--dump",
      "markdown",
      "--block-cidrs",
      PRIVATE_BLOCK_CIDRS,
      "--timeout",
      "20000",
      "--http-proxy",
      "socks5://127.0.0.1:1080",
      "--http-no-proxy",
      PRIVATE_NO_PROXY,
    ]);
    expect(PRIVATE_NO_PROXY.startsWith("localhost,")).toBe(true);
  });
});

describe("parseSearchMarkdown（duckduckgo 实测形态）", () => {
  // 形态来源：2026-09-22 moli 直连抓 html.duckduckgo.com 的真实输出：
  // 结果标题是 "## [标题](//duckduckgo.com/l/?uddg=<编码>&rut=...)"，
  // 摘要是独立的跳转链接行（文本为长句），夹着 favicon/URL 文本行。
  const ddgFixture = [
    "[](/html/ \"DuckDuckGo\")",
    "",
    "All Regions Argentina China Japan US (English) Vietnam (en)",
    "",
    "Any Time Past Day Past Week Past Month Past Year",
    "",
    "## [Rust 教程 | 菜鸟教程](//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.runoob.com%2Frust%2Frust%2Dtutorial.html&rut=aa)",
    "",
    "[![](//external-content.duckduckgo.com/ip3/www.runoob.com.ico)](//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.runoob.com%2Frust%2Frust%2Dtutorial.html&rut=aa) [www.runoob.com/rust/rust-tutorial.html](//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.runoob.com%2Frust%2Frust%2Dtutorial.html&rut=aa)",
    "",
    "[**Rust** 教程 **Rust** 是由 Mozilla 主导开发的高性能编译型编程语言，遵循安全、并发、实用的设计原则。](//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.runoob.com%2Frust%2Frust%2Dtutorial.html&rut=aa)",
    "",
    "## [入门指南 - Rust 程序设计语言 中文版](//duckduckgo.com/l/?uddg=https%3A%2F%2Frustwiki.org%2Fzh%2DCN%2Fbook%2Fch01%2D00%2Dgetting%2Dstarted.html&rut=bb)",
    "",
    "[rustwiki.org/zh-CN/book/ch01-00-getting-started.html](//duckduckgo.com/l/?uddg=https%3A%2F%2Frustwiki.org%2Fzh%2DCN%2Fbook%2Fch01%2D00%2Dgetting%2Dstarted.html&rut=bb)",
    "",
    "[本章将教你安装 Rust 并编写第一个程序。](//duckduckgo.com/l/?uddg=https%3A%2F%2Frustwiki.org%2Fzh%2DCN%2Fbook%2Fch01%2D00%2Dgetting%2Dstarted.html&rut=bb)",
  ].join("\n");

  it("解码 uddg 跳转壳为真实 URL 并提取结果（旧解析器对此形态返回 0 条）", () => {
    const items: SearchItem[] = parseSearchMarkdown(ddgFixture, "duckduckgo", 10);
    expect(items.length).toBe(2);
    expect(items[0]).toMatchObject({
      title: "Rust 教程 | 菜鸟教程",
      url: "https://www.runoob.com/rust/rust-tutorial.html",
      engine: "duckduckgo",
    });
    expect(items[1].url).toBe("https://rustwiki.org/zh-CN/book/ch01-00-getting-started.html");
    // 摘要来自摘要链接行的长句文本（非 URL 文本、非标题行）。
    expect(items[0].snippet).toContain("Mozilla 主导开发");
  });

  it("旧形态（直链 + 行内摘要）继续兼容", () => {
    const legacy = [
      "[Rust 程序设计语言（中文版）](https://kaisery.github.io/trpl-zh-cn/) —— 官方中译教程",
      "[Rust 语言圣经](https://course.rs/) —— 面向实战的中文教程",
    ].join("\n");
    const items = parseSearchMarkdown(legacy, "duckduckgo", 10);
    expect(items.length).toBe(2);
    expect(items[0].snippet).toBe("官方中译教程");
  });

  it("limit 截断结果", () => {
    const items = parseSearchMarkdown(ddgFixture, "duckduckgo", 1);
    expect(items.length).toBe(1);
  });

  it("URL 去重（仅 hash 不同视为同一条）", () => {
    const md = [
      "[A](https://example.com/a)",
      "[A 复本](https://example.com/a#section)",
      "[B](https://example.com/b)",
    ].join("\n");
    const items = parseSearchMarkdown(md, "bing", 10);
    expect(items.length).toBe(2);
  });

  it("过滤搜索引擎自身噪音与法务页脚链接", () => {
    const md = [
      "[跳到内容](https://html.duckduckgo.com/)",
      "[广告位](https://duckduckgo.com/y.js?ad=1)",
      "[真结果](https://example.com/real)",
      "[Privacy](http://go.microsoft.com/fwlink/?LinkId=521839)",
      "[京ICP备10036305号-7](https://beian.miit.gov.cn)",
      "[京公网安备11010802047360号](https://beian.mps.gov.cn/#/query/webSearch?code=x)",
    ].join("\n");
    const items = parseSearchMarkdown(md, "duckduckgo", 10);
    expect(items.map((item) => item.url)).toEqual(["https://example.com/real"]);
  });

  it("空输入与无结果输入返回空数组", () => {
    expect(parseSearchMarkdown("", "bing")).toEqual([]);
    expect(parseSearchMarkdown("   \n  ", "bing")).toEqual([]);
    expect(parseSearchMarkdown("# 只有标题\n没有链接", "bing")).toEqual([]);
  });
});

describe("parseBingRss（format=rss 实测形态）", () => {
  // 形态来源：2026-09-22 moli 直连抓 bing format=rss 的真实输出尾部
  // （转义 XML 树段；压平文本段丢失 URL/描述边界，不可用）。
  const rssFixture = [
    "Bing: rust 入门http://www.bing.com:80/search?q=rust+%e5%85%a5%e9%97%a8Search results…（压平段，不可解析）",
    "",
    "This XML file does not appear to have any style information associated with it.",
    "",
    "\\<rss version=\"2.0\"\\>",
    "",
    "\\<channel\\>",
    "",
    "\\<title\\>Bing: rust 入门\\</title\\>",
    "",
    "\\<item\\>",
    "",
    "\\<title\\>Rust Programming Language\\</title\\>",
    "",
    "\\<link\\>https://rust-lang.org/\\</link\\>",
    "",
    "\\<description\\>Rust is blazingly fast &amp; memory-efficient.\\</description\\>",
    "",
    "\\<pubDate\\>Mon, 21 Sep 2026 14:03:00 GMT\\</pubDate\\>",
    "",
    "…",
    "",
    "\\</item\\>",
    "",
    "\\<item\\>",
    "",
    "\\<title\\>Rust 教程 | 菜鸟教程\\</title\\>",
    "",
    "\\<link\\>https://www.runoob.com/rust/rust-tutorial.html\\</link\\>",
    "",
    "\\<description\\>Rust 教程：由 Mozilla 主导开发的高性能编译型编程语言。\\</description\\>",
    "",
    "\\</item\\>",
    "",
    "\\</channel\\>",
    "",
    "\\</rss\\>",
  ].join("\n");

  it("从转义 XML 树段提取标题/链接/摘要并解码实体", () => {
    const items = parseBingRss(rssFixture, "bing", 10);
    expect(items.length).toBe(2);
    expect(items[0]).toMatchObject({
      title: "Rust Programming Language",
      url: "https://rust-lang.org/",
      snippet: "Rust is blazingly fast & memory-efficient.",
      engine: "bing",
    });
    expect(items[1].url).toBe("https://www.runoob.com/rust/rust-tutorial.html");
  });

  it("limit 截断与空输入", () => {
    expect(parseBingRss(rssFixture, "bing", 1).length).toBe(1);
    expect(parseBingRss("", "bing")).toEqual([]);
    // 无树段标记时全文兜底扫描。
    expect(parseBingRss("\\<item\\>\\<title\\>T\\</title\\>\\<link\\>https://a.example/\\</link\\>\\</item\\>", "bing", 10).length).toBe(1);
  });
});

describe("parseSearxngJson", () => {
  it("解析 searxng JSON API 输出", () => {
    const json = JSON.stringify({
      results: [
        { title: "Rust", url: "https://rust-lang.org", content: "官方站" },
        { title: "无链接条目", url: "", content: "应跳过" },
      ],
    });
    const items = parseSearxngJson(json, "searxng", 10);
    expect(items?.length).toBe(1);
    expect(items?.[0]).toMatchObject({ title: "Rust", url: "https://rust-lang.org" });
  });

  it("非 JSON 输入返回 null（调用方回落通用解析）", () => {
    expect(parseSearxngJson("<html>not json</html>", "searxng")).toBeNull();
  });
});

describe("itemsToMarkdown", () => {
  it("生成带摘要的列表（分隔符统一为全角破折号）", () => {
    const md = [
      "[标题一](https://a.example.com/) - 摘要一",
      "[标题二](https://b.example.com/)",
    ].join("\n");
    const items = parseSearchMarkdown(md, "bing", 10);
    // 解析剥掉行内分隔符，重建统一用 "——"（显式 \u2014，避免隐形差异）。
    expect(itemsToMarkdown(items)).toBe(
      "- [标题一](https://a.example.com/) \u2014\u2014 摘要一\n- [标题二](https://b.example.com/)",
    );
  });

  it("空结果输出占位文案", () => {
    expect(itemsToMarkdown([])).toBe("（无结果）");
  });
});
