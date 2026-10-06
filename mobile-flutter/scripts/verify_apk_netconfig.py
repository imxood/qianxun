"""从 APK 里把 network_security_config.xml 抠出来，验证它到底打进去了什么。

为什么必须验 APK 而不是看源码
----------------------------
源码里写对了 ≠ APK 里是对的。可能的丢失点：
  · Gradle 资源合并（debug/release 不同的 res 目录）
  · `pub get` 之后的隐式覆盖
  · 打的是旧产物（fdev.py 的 apk 命令不清理）
而这个文件的失败方式极其隐蔽：App **能装、能启动、设备列表显示「在线」**
（因为它用的是 socket 或别的路径），只有真正发 HTTP 时被静默拦截。

症状对照
  ping 通 5.9ms          → 网络层没问题
  /qx-mobile/info 0.01s  → 服务端没问题
  手机上超时 6s          → **只剩「被系统拦」这一种可能**
"""
from __future__ import annotations

import re
import sys
import zipfile
from pathlib import Path

# fdev.py 的 apk 命令：--release --split-per-abi，产物在
# build/app/outputs/flutter-apk/app-<abi>-release.apk
APK_DIR = Path(r"E:\develop\dsh-workspace\qianxun\mobile-flutter\build\app\outputs\flutter-apk")


def main() -> int:
    apks = sorted(APK_DIR.glob("*release*.apk"))
    if not apks:
        print(f"没有找到 APK：{APK_DIR}")
        print("先跑 python scripts/fdev.py apk")
        return 1

    # 取 arm64 那个（真机用），x86_64 是模拟器用的
    target = next((a for a in apks if "arm64" in a.name), apks[0])
    print(f"检查 {target.name}（{target.stat().st_size / 1048576:.1f} MB）\n")

    with zipfile.ZipFile(target) as zf:
        names = zf.namelist()

        # ① 清单在不在
        # ⚠️ **不能按文件名找。** AAPT 会把 res/ 下的路径**重命名**成
        # `res/27.xml` 这样的短名（缩短 APK 体积），文件名不再保留。
        # 第一次查时按 `*network_security_config.xml` 找，得到「没有」，
        # 差点误判成「资源没打进 APK」——实际它就在 res/8G.xml 里。
        #
        # 正确做法：**按内容找**。这个坑记在这里以免再犯。
        cfg = None
        for name in names:
            if not (name.startswith("res/") and name.endswith(".xml")):
                continue
            raw = zf.read(name).decode("utf-8", "ignore")
            if "network-security-config" in raw:
                cfg = (name, raw)
                break
        if cfg is None:
            print("✗ APK 里**没有** network_security_config（按内容找也没找到）")
            return 1
        print(f"✓ 清单在：{cfg[0]}（按内容找的，文件名已被 AAPT 重命名）")

        # ② 里面的内容对不对
        raw = cfg[1]
        domains = re.findall(r"[\x00-\x7f]([\w.]{4,})", raw)
        declared = [
            d for d in dict.fromkeys(domains)
            if d.count(".") >= 2 and not d.startswith("res")
        ]
        print(f"  可读出的地址（{len(declared)} 条）：")
        for d in declared:
            print(f"    {d}")

        # ③ 关键判据：实测地址在不在
        measured = "192.168.20.2"
        hit = measured in domains
        print(f"\n{'✓' if hit else '✗'} 实测地址 {measured} "
              f"{'在清单里' if hit else '**不在清单里** —— 请求会被系统静默拦截'}")

        # ④ 顺带指出哪些是网段写法（无效）
        bogus = [d for d in domains if d.count(".") == 3 and d.endswith(".0")]
        if bogus:
            print(f"\n⚠️ 这些是**网段写法，Android 的 <domain> 不支持**，一条都不生效：")
            for d in bogus:
                print(f"    {d}")
            print("  它们让人以为已经管住了明文流量，实际是「安全剧场」。")

        # ⑤ base-config
        base = re.search(r'<base-config[^>]*cleartextTrafficPermitted="(\w+)"', raw)
        if base:
            print(f"\nbase-config cleartextTrafficPermitted = {base.group(1)}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
