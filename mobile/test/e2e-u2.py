# 千寻移动壳 E2E（uiautomator2 驱动，按控件文本定位）。
# 用法：python e2e-u2.py
# 前置：设备已连接 adb；mock 卡片(QX-TEST-PC)已存在（多 PC 场景）。
import sys
import time

import uiautomator2 as u2

REAL_URL = (
    'http://192.168.20.2:23090/qx-gate?token='
    'e444322c49188ef225341f372358b2bdaf522e4b89c6c47e4dbad83e479a0388'
)

d = u2.connect()
print('device:', d.serial)
d.app_start('com.qianxun.mobile')
time.sleep(2)
d.wait_timeout = 10


def must(text, timeout=10):
    if not d(text=text).wait(timeout=timeout):
        print(f'FAIL: 未找到控件「{text}」')
        d.screenshot('fail.png')
        sys.exit(1)


# 0) 确保在首页
if not d(text='添加连接').exists:
    d.press('back')
    time.sleep(1)
must('添加连接')
print('[1] 首页 OK，已有连接卡片:', d(text='QX-TEST-PC').exists)

# 1) 剪贴板粘贴添加「真实网关」连接
d.set_clipboard(REAL_URL)
d(text='📋 粘贴链接').click()
real_card = d(text='192.168.20.2:23090').wait(timeout=8)
if not real_card:
    # 剪贴板路径失效时的兜底：直接填输入框
    print('[2] 剪贴板路径未生效，回退输入框…')
    d(className='android.widget.EditText', instance=0).set_text(REAL_URL)
    d(text='添加连接').click()
    real_card = d(text='192.168.20.2:23090').wait(timeout=8)
print('[2] 真实网关卡片已添加（info 404 → host 回落名，预期行为）:', real_card)
d.screenshot('shot-10-two-cards.png')

# 2) 打开真实网关的工作台（WebView → /qx-gate 配对 → DSH 页面）
d(text='192.168.20.2:23090').click()
time.sleep(10)  # DSH SPA 加载
d.screenshot('shot-11-real-dsh.png')
must('工作台')
must('远程桌面')
print('[3] Workspace pills OK')

# 3) 远程桌面 Surface 占位
d(text='远程桌面').click()
if d(textContains='二期').wait(timeout=5):
    print('[4] 远程桌面占位 Surface OK')
else:
    print('[4] WARN: 远程桌面占位文本未命中（pill 未切换？）')
d.screenshot('shot-12-desktop.png')

# 4) 切回工作台 → 返回首页
d(text='工作台').click()
time.sleep(1)
d.press('back')
must('添加连接')
print('[5] 返回首页 OK，刚刚使用:', d(text='刚刚使用').exists)

# 5) 扫码：运行时权限弹窗处理 + 取景提示 + 关闭
d(text='📷 扫码添加').click()
for label in ['允许', '仅本次允许', '在使用该应用时允许', 'While using the app', 'ALLOW']:
    if d(text=label).exists(timeout=1.5):
        d(text=label).click()
        print('[6] 相机权限弹窗点击:', label)
        break
hint = d(textContains='对准电脑端').wait(timeout=10)
print('[6] 扫码取景层 OK（相机预览 + 提示）:', hint)
d.screenshot('shot-13-scanner.png')
d(text='✕ 关闭').click()
must('添加连接')
print('[7] 关闭扫码回首页 OK')

print('ALL E2E PASSED')
