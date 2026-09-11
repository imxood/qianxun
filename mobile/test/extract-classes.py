# 从 dsh-mobile 源码与 DSH 审批组件里提取真实 className，供皮肤 CSS 精确选目标。
import re
from pathlib import Path

targets = {
    'mobile-layout': Path(r'E:\develop\dsh-workspace\dsh-mobile\src\mobile-layout.ts'),
}
approval = list(Path(r'C:\Users\maxu\.qianxun\dsh-runtime\node_modules\.pnpm').glob(
    '@deepseek-ai+dsh-client-ui-approval*/node_modules/@deepseek-ai/dsh-client-ui-approval/dist/*.js'))
if approval:
    targets['approval'] = approval[0]

for name, path in targets.items():
    if not path.exists():
        print(f'== {name}: MISSING {path}')
        continue
    raw = path.read_text(encoding='utf-8', errors='ignore')
    classes = set()
    for m in re.finditer(r'className\s*[:=]\s*["\'`]([^"\'`]{2,120})["\'`]', raw):
        for token in m.group(1).split():
            if token not in ('{', '}'):
                classes.add(token)
    # 模板字符串里的条件类名 ${...} 剔除
    classes = {c for c in classes if '$' not in c}
    print(f'== {name} ({len(classes)} classes) ==')
    for c in sorted(classes)[:80]:
        print(' ', c)
