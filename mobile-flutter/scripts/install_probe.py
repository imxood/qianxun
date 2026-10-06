"""把探针装进**千寻内置 DSH 真正使用的** mobile-access 目录。

踩过的坑（都写在这里，免得下次再踩）
-----------------------------------
千寻有两套路径，**都不是 ~/.dsh**：

1. `C:\\Users\\maxu\\.qianxun\\`            —— 千寻的根
2. `C:\\Users\\maxu\\.qianxun\\dsh-home\\`  ← **DSH 真正用的 home**

而装插件时我先用了 `DSH_HOME=C:\\Users\\maxu\\.qianxun`，于是：
- 插件装到了 `.qianxun\\profiles\\web\\node_modules\\dsh-mobile`
- setup 生成了 `.qianxun\\mobile-access\\`
但**运行时读的是 `dsh-home\\mobile-access\\`**（空目录）。

正确的 home 由运行中的进程命令行确定：
  node .../dsh/lib/bin.js --profile web --no-open --host 127.0.0.1 --port 0
它的数据根是 `dsh-home`（与 `dsh-runtime` 同级）。

所以本脚本**两边都放**：
- 主：`dsh-home/mobile-access/mobile.js`（真正生效的）
- 备：`.qianxun/mobile-access/mobile.js`（以防 home 判定不同）

并保留 setup.json 存在的一侧（若 setup 只在一侧存在，说明那一侧才是
setup 认可的位置 —— 我们据此判断，不靠猜）。

用法：python scripts/install_probe.py [--home <path>]
"""
from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
PROBE = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter\scripts\probe_mobile_js.js"
)

# 候选 home，按「越可能越前」排。理由写在上面注释里。
CANDIDATE_HOMES = [
    QIANXUN / "dsh-home",     # 运行中进程实际使用
    QIANXUN,                   # 我第一次装错的地方，留在备选
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--home", default="", help="手动指定 DSH home")
    args = ap.parse_args()

    homes = [Path(args.home)] if args.home else CANDIDATE_HOMES

    if not PROBE.is_file():
        print(f"探针不存在：{PROBE}")
        return 1

    # 先看哪一侧已经有 setup.json —— 那就是 setup 认可的位置
    print("== 探测哪个 home 是活的 ==")
    live: Path | None = None
    for home in homes:
        access = home / "mobile-access"
        setup = access / "setup.json"
        has_setup = setup.is_file()
        print(f"  {access}  setup.json={has_setup}")
        if has_setup and live is None:
            live = home

    if live is None:
        print("\n两侧都没有 setup.json —— 插件可能没在正确的 profile 里 setup。")
        print("先在运行 DSH 的那个 home 上跑：")
        print("  dsh --profile web exec dsh-mobile setup --address <ip>")
        return 1

    print(f"\n活的是：{live}")

    for home in homes:
        access = home / "mobile-access"
        access.mkdir(parents=True, exist_ok=True)
        target = access / "mobile.js"
        backup = access / "mobile.js.orig"
        if target.is_file() and not backup.is_file():
            shutil.copy2(target, backup)
            print(f"\n已备份 {target.name} → {backup.name}")
        shutil.copy2(PROBE, target)
        text = target.read_text(encoding="utf-8")
        ok = "window.dshMobile.register" in text
        print(f"写入 {target}（{target.stat().st_size} B）自检 register={'✓' if ok else '✗'}")
        if not ok:
            return 1

    # 报告 run 状态，供下一步判断要不要重启
    running = any(
        "dsh" in (p.name or "").lower() or "bin.js" in str(p)
        for p in [QIANXUN]
    )
    print("\n== 后续 ==")
    print("1. DSH 若已在跑且是 profile web，插件改动通常需重启才装载")
    print("2. 重启：dsh --profile web（千寻桌面版会自动拉起，一般不用手动）")
    print("3. 手机进 DSH 页 → 右上角出现探针浮层")
    print("\n恢复默认：删除任一侧的 mobile.js（.orig 是留底）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
