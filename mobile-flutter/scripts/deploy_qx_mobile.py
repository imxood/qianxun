"""把 qx-mobile/ 下的定制层部署到千寻的 mobile-access 目录。

为什么是独立目录而不是直接改千寻仓库
------------------------------------
千寻的定制层（`src-tauri/src/remote/mobile_ui.rs`）已经实现了一切机制：
注入 bootstrap、轮询 hash、热替换、ETag/304、鉴权、大小上限。
**我们不该改它** —— 那是上游基础设施，且改了会让千寻与官方分叉。

正确的分工是：
  千寻仓库    机制（不动）
  qx-mobile/  我们的定制内容（本目录，可版本控制、可评审）
  部署        复制到 <dsh_home>/mobile-access/

这样：
  · 定制内容进 git，能 review、能回滚、能 diff
  · 不用重编译千寻（保存后 5 秒热生效）
  · 千寻升级时我们的定制不会冲突（只依赖已公开的 /qx-mobile 契约）

用法
----
  python scripts/deploy_qx_mobile.py            # 部署
  python scripts/deploy_qx_mobile.py --check    # 只检查差异，不写
  python scripts/deploy_qx_mobile.py --home <路径>  # 指定 DSH home

⚠️ 本脚本只写 custom.css / custom.js 两个文件，**不碰** devices.json、
   凭据、TLS 证书。
"""
from __future__ import annotations

import argparse
import hashlib
import shutil
import sys
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
# paths.rs:91 —— 隔离模式下 dsh_home = <data_dir>/dsh-home
# settings.json 的 dsh.home = "isolated"，所以是 .qianxun/dsh-home
SOURCE = Path(r"E:\develop\dsh-workspace\qianxun\mobile-flutter\qx-mobile")

# mobile_ui.rs:31 的上限 —— 超出网关会返 413
CSS_LIMIT = 512 * 1024
JS_LIMIT = 1024 * 1024

FILES = ["custom.css", "custom.js"]


def short_hash(data: bytes) -> str:
    """与 mobile_ui.rs:236 short_hash_of 同样的算法：sha256 前 16 位 hex。

    保持一致的意义：本脚本可以提前算出网关会算出的 hash，
    于是能报「部署后 hash 将变成 X」用于交叉核对。
    """
    return hashlib.sha256(data).hexdigest()[:16]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--home", default="", help="DSH home 目录")
    ap.add_argument("--check", action="store_true", help="只检查，不写")
    args = ap.parse_args()

    home = Path(args.home) if args.home else QIANXUN / "dsh-home"
    access = home / "mobile-access"

    if not SOURCE.is_dir():
        print(f"定制层源目录不存在：{SOURCE}")
        return 1
    if not access.is_dir():
        print(f"目标目录不存在：{access}")
        print("先启动一次千寻桌面版（网关启动时自动创建该目录，mobile_ui.rs:50）")
        return 1

    print(f"源   {SOURCE}")
    print(f"目标 {access}\n")

    changed = False
    for name in FILES:
        src = SOURCE / name
        dst = access / name
        if not src.is_file():
            print(f"  ○ {name} 源文件不存在，跳过（网关会用占位注释）")
            continue

        data = src.read_bytes()
        limit = CSS_LIMIT if name.endswith(".css") else JS_LIMIT
        if len(data) > limit:
            print(f"  ✗ {name} 超出网关上限 {len(data)} > {limit} 字节，会被拒 413")
            return 1

        incoming = short_hash(data)
        if dst.is_file():
            current = short_hash(dst.read_bytes())
        else:
            current = "<不存在>"

        same = current == incoming
        marker = "=" if same else "≠"
        print(f"  {marker} {name:<12} {len(data):>6} B  hash {current} → {incoming}")

        if same:
            continue
        changed = True
        if args.check:
            continue

        # 备份只做一次（保留最初状态，便于对比「我们改了什么」）
        backup = dst.with_suffix(dst.suffix + ".orig")
        if dst.is_file() and not backup.is_file():
            shutil.copy2(dst, backup)
            print(f"      已备份 → {backup.name}")
        shutil.copy2(src, dst)
        print(f"      已写入 {dst}")

    if args.check:
        print("\n(--check：未写入)")
        return 0
    if not changed:
        print("\n无变化。")
    else:
        print("\n完成。生效方式：")
        print("  · 手机已打开 DSH 页 → **最多 5 秒**自动生效（bootstrap.js:15）")
        print("  · 页面隐藏时 30 秒一轮；切回前台立刻刷（visibilitychange）")
        print("  · 也可以手机上进 DSH 页强制下拉刷新")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
