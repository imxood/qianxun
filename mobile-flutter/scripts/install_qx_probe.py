"""把千寻自有网关的 L3 探针装到 mobile-access/custom.js。

为什么是这里
------------
千寻**已经有**自己的移动定制层（`src-tauri/src/remote/mobile_ui.rs`，
标注为「方案 A」），它：
  - 在每个 DSH 页面 `</head>` 前注入 `qx-mobile-bootstrap.js`
  - 定义 `window.qxMobile.register(({ root, document, window }) => …)`
  - 轮询 `/qx-mobile/version` 的内容 hash，变化即热替换 custom.css / custom.js
  - 定制文件从 `<dsh_home>/mobile-access/custom.{css,js}` 读

所以契约是**千寻自己的**，与 dsh-mobile 的 `window.dshMobile` 完全不同：
  入口   window.qxMobile.register   （不是 dshMobile）
  上下文 { root, document, window } （**没有 request**，用 window.fetch 即可）
  文件   custom.js / custom.css     （不是 mobile.js / mobile.css）

这个方案比 dsh-mobile 那条路好得多：
  · 走千寻自己的远程 URL（23090），已有 qx_token 认证与配对流程
  · 不需要自签 CA / TLS 固定（明文 HTTP）
  · 不需要 mobile-access 的四层认证（Origin/Session/CSRF/CIDR）
  · 基础��施已在千寻仓库里，壳层只要「加载一个 WebView」

⚠️ 与 dsh-mobile 的定制文件**同目录共存**：dsh-mobile 用
`mobile.js` / `mobile.css`，千寻用 `custom.js` / `custom.css`。
两者不冲突（不同文件名、不同端点），但同一台电脑上两个插件的
DSH 页面各自生效。本探针只验千寻这条。
"""
from __future__ import annotations

import shutil
import sys
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
# paths.rs:91 dsh_home = data_dir/dsh-home；隔离模式下是 .qianxun/dsh-home
ACCESS = QIANXUN / "dsh-home" / "mobile-access"
PROBE = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter\scripts\probe_qx_mobile_js.js"
)


def main() -> int:
    if not ACCESS.is_dir():
        print(f"目录不存在：{ACCESS}")
        print("千寻的 mobile-access 由网关启动时自动创建（mobile_ui.rs:50）")
        print("先启动一次千寻桌面版")
        return 1
    if not PROBE.is_file():
        print(f"探针不存在：{PROBE}")
        return 1

    target = ACCESS / "custom.js"
    backup = ACCESS / "custom.js.orig"
    if target.is_file() and not backup.is_file():
        shutil.copy2(target, backup)
        print(f"已备份 custom.js → custom.js.orig（{target.stat().st_size} B）")
    elif target.is_file():
        print(f"custom.js 已存在（{target.stat().st_size} B），直接覆盖；备份在 custom.js.orig")

    shutil.copy2(PROBE, target)
    text = target.read_text(encoding="utf-8")
    print(f"探针已装入 {target}（{target.stat().st_size} B）")

    if "window.qxMobile.register" not in text:
        print("⚠️ 探针里没有 window.qxMobile.register —— 装上也不会执行")
        return 1
    print("自检通过：含 window.qxMobile.register")

    print("\n生效路径（实测依据 mobile_ui.rs）：")
    print("  · 页面里由网关注入的 qx-mobile-bootstrap.js 定义 window.qxMobile")
    print("  · 脚本从 /qx-mobile/custom.js?v=<hash> 拉取")
    print("  · hash 变了就热替换 —— 保存后**最多 5 秒**（页面可见时）自动生效")
    print("    （bootstrap.js:15 VISIBLE_MS = 5000）")
    print("\n查看方式：")
    print("  · 手机进 DSH 页 → 右上角浮层")
    print("  · 或电脑端 DevTools Console 的 [qx-l3-probe] 输出")
    print("\n恢复默认：删除 custom.js（.orig 是留底）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
