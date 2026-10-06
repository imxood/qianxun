"""临时把一个探针脚本装成 custom.js，并保留当前版本为 .v2。

用途
----
`custom.js` 跑在 DSH 页面里，所以有些问题只能在**页面里**探：
  · DSH 真实渲染出来的 DOM 结构（工作区树长什么样、用什么锚点）
  · 页面加载后哪些 data-* 真的存在（bundle 里搜不到，只能读 DOM）

而这类探针**不该留在 qx-mobile/ 里** —— 它们是一次性的、依赖当时的
页面状态。所以这个脚本不把它们收进版本控制，只负责装与还原。

用法
----
  python scripts/deploy_probe.py install <脚本路径>   # 装为 custom.js
  python scripts/deploy_probe.py restore            # 还原为 custom.js.v2
  python scripts/deploy_probe.py status             # 看当前装的是哪个

⚠️ 只碰 custom.js 一个文件。custom.css / devices.json / tls 一律不动。
"""
from __future__ import annotations

import hashlib
import shutil
import sys
from pathlib import Path

ACCESS = Path(r"C:\Users\maxu\.qianxun\dsh-home\mobile-access")
CUSTOM_JS = ACCESS / "custom.js"
BACKUP = ACCESS / "custom.js.v2"


def short_hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:16]


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 1
    cmd = sys.argv[1]

    if cmd == "status":
        if not CUSTOM_JS.is_file():
            print("custom.js 不存在")
            return 1
        data = CUSTOM_JS.read_bytes()
        print(f"当前 custom.js  {len(data):>6} B  hash {short_hash(data)}")
        if BACKUP.is_file():
            b = BACKUP.read_bytes()
            print(f"备份 custom.js.v2 {len(b):>6} B  hash {short_hash(b)}")
        return 0

    if cmd == "restore":
        if not BACKUP.is_file():
            print(f"没有备份 {BACKUP}，无法还原")
            return 1
        shutil.copy2(BACKUP, CUSTOM_JS)
        data = CUSTOM_JS.read_bytes()
        print(f"已还原 custom.js  {len(data)} B  hash {short_hash(data)}")
        print("手机页面上最多 5 秒后自动生效")
        return 0

    if cmd == "install":
        if len(sys.argv) < 3:
            print("用法：deploy_probe.py install <脚本路径>")
            return 1
        source = Path(sys.argv[2])
        if not source.is_file():
            print(f"找不到 {source}")
            return 1

        # 先备份当前版本（只在还没有备份时），否则探针会把备份覆盖掉
        if CUSTOM_JS.is_file() and not BACKUP.is_file():
            shutil.copy2(CUSTOM_JS, BACKUP)
            print(f"当前版本已备份 → {BACKUP.name}")

        shutil.copy2(source, CUSTOM_JS)
        data = CUSTOM_JS.read_bytes()
        print(f"已装 {source.name} → custom.js  {len(data)} B  hash {short_hash(data)}")

        if "qxMobile.register" not in data.decode("utf-8", "ignore"):
            print("⚠️ 装上的脚本里没有 qxMobile.register —— 装了也不会执行")
            return 1
        print("自检 ✓ 含 qxMobile.register")
        print("\n手机进 DSH 页 → 点开 DSH 自己的工作区侧栏 → 看浮层")
        print("还原：python scripts/deploy_probe.py restore")
        return 0

    print(f"未知命令 {cmd}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
