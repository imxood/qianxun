"""出包全链：构建 release(分 ABI) + debug -> 发布 -> 验证 -> 打印下载链接。

2026-11-05：把「构建 + 发布 + 验证」串进一个进程，避免分多步时中途醒来
状态丢失；每步的产物（APK / 发布目录 / 验证结论）都落在磁盘，可复查。

用法：
    python scripts/build_and_publish.py            # 构建 + 发布 + 验证
    python scripts/build_and_publish.py --skip-build   # 只发布+验证
"""
from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APK_DIR = ROOT / "build" / "app" / "outputs" / "flutter-apk"
SERVE_DIR = Path(r"D:\Temp\qx-apk")
PY = sys.executable


def flutter_exe() -> str:
    """Flutter 可执行文件的绝对路径。

    ⚠️ Windows 上 PATH 里的是 `flutter.BAT`（**大写扩展名**）。Python 的
    `subprocess` 走 `CreateProcess`，它**不会**自己补 `.bat`/`.cmd` 扩展，
    传 `"flutter"` 会直接 `FileNotFoundError: 系统找不到指定的文件`——
    在 PowerShell 里能跑、换个宿主就炸的典型坑。
    """
    # 1) 优先从 android/local.properties 的 flutter.sdk 推导
    props = ROOT / "android" / "local.properties"
    if props.is_file():
        for line in props.read_text(encoding="utf-8").splitlines():
            if line.strip().startswith("flutter.sdk"):
                sdk = line.split("=", 1)[1].strip().replace("\\\\", "\\")
                exe = Path(sdk) / "bin" / "flutter.bat"
                if exe.is_file():
                    return str(exe)
    # 2) 退到 PATH 里的真实文件
    from shutil import which

    for name in ("flutter.BAT", "flutter.bat", "flutter"):
        found = which(name)
        if found:
            return found
    return "flutter"


def run(cmd: list[str], cwd: Path | None = None, timeout: int = 1800) -> int:
    print(f"\n$ {' '.join(cmd)}", flush=True)
    started = time.time()
    proc = subprocess.run(
        cmd,
        cwd=str(cwd or ROOT),
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )
    out = (proc.stdout or "") + (proc.stderr or "")
    for line in out.splitlines()[-25:]:
        print("   ", line[:200])
    print(f"   -> exit {proc.returncode}  ({time.time() - started:.0f}s)", flush=True)
    return proc.returncode


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def dart_symbols(apk: Path) -> dict[str, int]:
    """扫 APK 里的 Dart 符号。

    debug 包是 JIT（kernel_blob.bin），符号明文可见；release 包是 AOT
    （libapp.so），字符串被压缩成非 UTF-8 形式，扫不出来——所以 release
    只能靠「与旧包 libapp.so 的哈希不同」来间接证明代码变了。
    """
    found: dict[str, int] = {}
    with zipfile.ZipFile(apk) as z:
        for name in z.namelist():
            if not (
                name.endswith("kernel_blob.bin")
                or (name.endswith(".so") and "libapp" in name)
            ):
                continue
            data = z.read(name)
            for kw in (
                b"WebViewFileChooser",
                b"setOnShowFileSelector",
                b"FileSelectorMode",
                b"customLayerReady",
            ):
                n = data.count(kw)
                if n:
                    found[kw.decode()] = found.get(kw.decode(), 0) + n
    return found


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-build", action="store_true", help="跳过构建，只发布+验证")
    args = ap.parse_args()

    if not args.skip_build:
        flutter = flutter_exe()
        # 1) release 分 ABI（手机装的就是它）
        if run([flutter, "build", "apk", "--release", "--split-per-abi"]) != 0:
            return 1
        # 2) debug（含调试器，供排障）
        if run([flutter, "build", "apk", "--debug"]) != 0:
            return 1

    # 3) 发布（脚本自带新鲜度闸门：陈旧产物会被拒绝发布）
    if run([PY, "-X", "utf8", "scripts/publish_apk.py", "--no-serve"]) != 0:
        print("发布失败（新鲜度闸门拦下了陈旧产物？）", file=sys.stderr)
        return 1

    # 4) 验证：发布目录里的每个包都要是「今天的」且元数据正确
    print("\n=== 发布结果验证 ===", flush=True)
    ok = True
    for apk in sorted(SERVE_DIR.glob("*.apk")):
        st = apk.stat()
        age_min = (time.time() - st.st_mtime) / 60
        digest = sha256(apk)[:16]
        with zipfile.ZipFile(apk) as z:
            libs = [n for n in z.namelist() if n.endswith(".so") and "libapp" in n]
            app_hash = hashlib.sha256(z.read(libs[0])).hexdigest()[:12] if libs else "-"
        print(
            f"  {apk.name:34} {st.st_size / 1024 / 1024:6.1f} MB  "
            f"age={age_min:4.1f}min  sha={digest}  libapp={app_hash}"
        )
        if age_min > 30:
            print(f"     ⚠️ 产物比源码旧 {age_min:.0f} 分钟——不该出现", file=sys.stderr)
            ok = False
        syms = dart_symbols(apk)
        if syms:
            print(f"     Dart 符号: {syms}")

    # 5) HTTP 可达性（真机走的就是这个）
    print("\n=== HTTP 可达性 ===", flush=True)
    for ip in ("192.168.20.2", "192.168.5.24"):
        for apk in sorted(SERVE_DIR.glob("*-arm64.apk")):
            url = f"http://{ip}:8899/{apk.name}"
            try:
                req = urllib.request.Request(url, method="HEAD")
                with urllib.request.urlopen(req, timeout=5) as r:
                    size = r.headers.get("content-length")
                    print(f"  {url}  -> HTTP {r.status}  {size} B")
            except Exception as exc:  # noqa: BLE001
                print(f"  {url}  -> {type(exc).__name__}: {exc}", file=sys.stderr)
                ok = False

    # 6) aapt2 元数据（versionName / versionCode / targetSdk）
    sdk_dir = Path(r"D:\programs\android-sdk")
    aapts = sorted(sdk_dir.glob("build-tools/*/aapt2.exe"))
    if aapts:
        print("\n=== APK 元数据 ===", flush=True)
        # 门槛与 publish_apk.py 同源（单一规则来源，避免漂移）。
        sys.path.insert(0, str(ROOT / "scripts"))
        from publish_apk import MIN_VERSION_CODE  # noqa: PLC0415

        for apk in sorted(SERVE_DIR.glob("*.apk")):
            proc = subprocess.run(
                [str(aapts[-1]), "dump", "badging", str(apk)],
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            pkg = next(
                (l for l in proc.stdout.splitlines() if l.startswith("package:")), ""
            )
            print("  ", pkg[:130])
            code = None
            for token in pkg.split():
                if token.startswith("versionCode="):
                    code = int(token.split("=", 1)[1].strip("'"))
                    break
            if code is None:
                print(f"     ⚠️ 读不到 versionCode", file=sys.stderr)
                ok = False
            elif code < MIN_VERSION_CODE:
                print(
                    f"     ⚠️ versionCode={code} < {MIN_VERSION_CODE}，"
                    f"手机上会报「已安装更高版本」而装不上",
                    file=sys.stderr,
                )
                ok = False
            else:
                print(f"     versionCode={code} >= {MIN_VERSION_CODE}  ✅")

    print("\n下载页: http://192.168.20.2:8899/")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
