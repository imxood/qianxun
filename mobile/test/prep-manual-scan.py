# 删除真实网关卡片（让用户实扫走一次全新添加），保持扫码层打开。
import time

import uiautomator2 as u2

d = u2.connect()
d.app_start('com.qianxun.mobile')
time.sleep(2)
if not d(text='添加连接').exists:
    d.press('back')
    time.sleep(1)

# 关掉扫码层再操作列表
if d(text='✕ 关闭').exists:
    d(text='✕ 关闭').click()
    time.sleep(1)

dots = d(text='⋯')
print('⋯ 数量:', dots.count)
if dots.count > 1:
    dots[1].click()  # 真实网关卡在第二位
    time.sleep(1)
    d(text='删除连接').click()
    time.sleep(1)
    print('真实卡已删除, ⋯ 剩余:', d(text='⋯').count)
else:
    print('只有一张卡，无需删除')

# 重新打开扫码层，留给用户实扫
d(text='📷 扫码添加').click()
time.sleep(2.5)
print('扫码层已打开:', d(textContains='对准电脑端').wait(timeout=8))
