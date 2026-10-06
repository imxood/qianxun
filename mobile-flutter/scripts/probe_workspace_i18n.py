"""从 workspace 插件 chunk 提取 DSH 的**原生 i18n 文案与数据形状**。

为什么要抠这个
--------------
手机端要显示「未分组 / 已归档 / 置顶会话」这些分区，
就得知道 DSH 自己**怎么称呼它们** —— 随手起个名会出现两套词
（桌面写「未分组」、手机写「其他会话」），用户会以为是两回事。

实测（10,600,622 字节的 plugin 组合 chunk）：

    group.ungrouped          = Ungrouped / 未分组
    row.archived            = Archived  / 已归档
    menu.pinSession         = Pin session     / 置顶会话
    menu.unpinSession       = Unpin session   / 取消置顶
    menu.archiveSession     = Archive session / 归档会话
    menu.unarchiveSession   = Unarchive       / 取消归档
    menu.addWorkspace       = Add workspace…  / 添加工作区…
    defaultWorkspace.failed = Unable to create default workspace…

数据形状（同一 chunk 里的字段访问）：

    workspace.title       标题
    workspace.path        路径
    workspace.sessionIds  归属的会话 id 列表   ← **显式归属**
    workspace.phase       状态
    workspace.createdAt

⚠️ 这与 `<dsh_home>/storages/workspace.json` 的结构**完全一致**
（`tables.workspaces[id] = {path, title, sessionIds, createdAt, updatedAt}`），
交叉验证了注册表就是权威数据源。

用法：python scripts/probe_workspace_i18n.py
"""
from __future__ import annotations

import json
import re
import urllib.error
import urllib.request
from pathlib import Path

QIANXUN = Path(r"C:\Users\maxu\.qianxun")
GATEWAY = "http://192.168.20.2:23090"
OUT = Path(
    r"E:\develop\dsh-workspace\qianxun\mobile-flutter"
    r"\docs\probe\workspace-i18n.json"
)

# 只关心分区/行/菜单这三个前缀 —— 它们是手机端要复刻的措辞
PREFIXES = ("row.", "section.", "group.", "menu.")


def token() -> str:
    settings = json.loads((QIANXUN / "settings.json").read_text(encoding="utf-8"))
    return next(
        d for d in settings["remote"]["devices"] if not d.get("revoked")
    )["token"]


def get(path: str, timeout: int = 90) -> tuple[int, str]:
    req = urllib.request.Request(
        f"{GATEWAY}{path}", headers={"Cookie": f"qx_token={token()}"}
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read().decode("utf-8", "ignore")
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(200).decode("utf-8", "ignore")
    except (urllib.error.URLError, OSError) as exc:
        return 0, f"{type(exc).__name__}: {exc}"


def main() -> int:
    status, html = get("/")
    if status != 200:
        print(f"根页面 {status}")
        return 1
    big = next(
        (h for h in re.findall(r'<link[^>]*href="([^"]+)"', html)
         if "dsh-client-ui-workspace" in h),
        None,
    )
    if big is None:
        print("找不到含 dsh-client-ui-workspace 的 plugin 组合")
        return 1

    st, js = get("/" + big.replace("&amp;", "&").lstrip("./"))
    print(f"chunk {st}  {len(js)} 字节")

    # "key": "value" —— 压缩后英/中各一份，中文那份不含 { 占位符时可区分
    pairs: dict[str, list[str]] = {}
    for key, value in re.findall(
        r'"((?:row|section|group|menu)\.[A-Za-z0-9_]{2,40})"\s*:\s*"([^"]{0,80})"', js
    ):
        pairs.setdefault(key, [])
        if value not in pairs[key]:
            pairs[key].append(value)

    def is_cn(text: str) -> bool:
        return bool(re.search(r"[\u4e00-\u9fff]", text))

    i18n: dict[str, dict] = {}
    for key, values in pairs.items():
        en = next((v for v in values if not is_cn(v)), None)
        zh = next((v for v in values if is_cn(v)), None)
        i18n[key] = {"en": en, "zh": zh}

    print(f"\n=== 分区/行/菜单 类 i18n（{len(i18n)} 条）===")
    for key in sorted(i18n):
        entry = i18n[key]
        # 只打印与本次要做的分区/状态相关的
        if not any(h in key.lower() for h in (
            "ungroup", "archiv", "pin", "workspace", "session", "expand",
            "collapse", "group", "run", "wait", "idle", "done", "fail",
        )):
            continue
        print(f"  {key:<34} zh={entry['zh']!r:<26} en={entry['en']!r}")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({
        "_probe": {
            "note": (
                "DSH 工作区相关的原生 i18n 文案。手机端复刻分区时**必须用同一套措辞**，"
                "否则桌面写「未分组」手机写「其他会话」，用户会以为是两回事。"
            ),
            "source": "probe_workspace_i18n.py",
            "chunkBytes": len(js),
        },
        "i18n": i18n,
    }, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\n已写入 {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
