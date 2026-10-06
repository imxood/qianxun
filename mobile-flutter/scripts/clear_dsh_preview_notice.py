"""清掉 DSH 的 Preview Notice —— 它会吞掉所有点击，挡住一切真机验证。

问题
----
DSH 0.2 会弹一个「Preview Notice」模态框（预览版声明）。
它每次热替换 custom.js 后页面重载就**再弹一次**，而它是模态的，
会吞掉所有点击 —— 于是「点工作区图标展开侧栏」这类操作根本点不动，
截图永远停在这个框上，验证无法进行。

⚠️ 这不是「绕过测试」，而是**移除一个与本任务无关的第三方弹窗**，
   就像关掉一个一直弹的更新提醒。DSH 本身的功能不受影响。

它存在哪
--------
未知（未在 bundle 里搜到关键字，可能是 localStorage / settings 命名空间
或按版本号判定）。所以这里**先探测再清**，而不是猜键名。

安全约定
--------
· 只写 `<dsh_home>/storages/` 与 `dsh-home/profiles/*/`，都是 DSH 自己的
  用户数据目录，**不碰 DSH 程序代码**。
· 清之前把命中的键名与原值打印出来，可回滚。
· 不删任何文件，只改具体的 storage 项。
"""
from __future__ import annotations

import json
import shutil
from datetime import datetime
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
DSH_HOME = QIANXUN / "dsh-home"
STAMP = datetime.now().strftime("%Y%m%d-%H%M%S")

# 可能存着 preview 标记的地方
CANDIDATES = [
    DSH_HOME / "storages" / "preferences.json",
    DSH_HOME / "storages" / "settings.json",
    DSH_HOME / "settings.yaml",
    DSH_HOME / "storages" / "user.json",
]

# 命中这些词就算相关（大小写不敏感）
HINTS = ("preview", "notice", "onboard", "welcome", "first-run", "firstRun")


def scan() -> list[tuple[Path, str, str]]:
    hits: list[tuple[Path, str, str]] = []
    for path in CANDIDATES:
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        try:
            data = json.loads(text)
        except json.JSONDecodeError:
            # YAML / 非 JSON，只按行扫
            for lineno, line in enumerate(text.splitlines(), 1):
                low = line.lower()
                if any(h.lower() in low for h in HINTS):
                    hits.append((path, f"line {lineno}", line.strip()[:100]))
            continue
        stack = [(data, "")]
        while stack:
            node, trail = stack.pop()
            if isinstance(node, dict):
                for key, value in node.items():
                    next_trail = f"{trail}.{key}" if trail else str(key)
                    if any(h.lower() in str(key).lower() for h in HINTS):
                        hits.append((path, next_trail, json.dumps(value, ensure_ascii=False)[:100]))
                    stack.append((value, next_trail))
            elif isinstance(node, list):
                for index, value in enumerate(node):
                    stack.append((value, f"{trail}[{index}]"))
    return hits


def main() -> int:
    print("=== 扫描 preview 相关存储 ===")
    hits = scan()
    if not hits:
        print("候选文件里没有 preview 相关键。")
        print("\n已扫描：")
        for path in CANDIDATES:
            print(f"  {'有' if path.is_file() else '无'}  {path}")
        print("\n说明：标记可能不在这些文件里（可能在 localStorage 或 IndexedDB）。")
        print("那种情况下改用另一条路：让 custom.js 在页面内直接关掉这个弹窗。")
        return 1

    for path, key, value in hits:
        print(f"  {path.name}  {key} = {value}")

    # 备份 + 逐个置 false
    for path in {h[0] for h in hits}:
        backup = path.with_suffix(path.suffix + f".bak-{STAMP}")
        shutil.copy2(path, backup)
        print(f"\n已备份 {path.name} → {backup.name}")
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            print(f"  ⚠️ {path.name} 不是 JSON，跳过（需手动处理）")
            continue

        changed = []

        def clear(node, trail=""):
            if isinstance(node, dict):
                for key in list(node):
                    low = str(key).lower()
                    full = f"{trail}.{key}" if trail else str(key)
                    value = node[key]
                    if any(h.lower() in low for h in HINTS):
                        if isinstance(value, bool):
                            node[key] = False
                            changed.append(f"{full} = False")
                        else:
                            changed.append(f"{full} = {value!r}  ← 非布尔，未改")
                    else:
                        clear(value, full)
            elif isinstance(node, list):
                for index, value in enumerate(node):
                    clear(value, f"{trail}[{index}]")

        clear(data)
        path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
        for item in changed:
            print(f"  已改 {item}")

    print("\n完成。重启 DSH 桌面版 / 手机 App 重新加载页面后生效。")
    print(f"回滚：把 *.bak-{STAMP} 改回原名")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
