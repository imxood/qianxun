import { test, expect } from './_fixture';

/**
 * 独立窗口 e2e（WebView2 CDP，debug 构建 → ~/.qianxun_dev）。
 * 全程不启动 DSH（断言占位页或 iframe 的存在，不依赖 DSH 运行状态）。
 */

test.describe('独立 DSH 窗口', () => {
  test('分离独立窗口：新 page 出现并渲染 DSH 页', async ({ helper }) => {
    const standalone = await helper.spawnStandalone('dsh');
    // 独立窗口标识（标题栏「独立窗口」角标）。
    await expect(standalone.getByText('独立窗口', { exact: true })).toBeVisible({
      timeout: 10_000,
    });
    // DSH 未就绪时的占位或就绪的 iframe，二者必有其一。
    const hasPlaceholder = await standalone
      .getByText('DSH 未运行', { exact: false })
      .isVisible()
      .catch(() => false);
    const hasIframe = await standalone
      .locator('iframe[title="DSH"]')
      .isVisible()
      .catch(() => false);
    expect(hasPlaceholder || hasIframe).toBe(true);
  });

  test('独立窗口关闭：主窗侧栏项恢复（window://closed 回流）', async ({ main, helper }) => {
    const standalone = await helper.spawnStandalone('dsh');
    await expect(standalone.getByText('独立窗口', { exact: true })).toBeVisible();

    // 原生关闭（无终端域后无需确认流）→ Destroyed 广播 → 主窗 reattach。
    await standalone.getByRole('button', { name: '关闭' }).click();
    await expect
      .poll(async () => {
        const visible = await main
          .locator('[data-testid="nav-dsh"]')
          .isVisible()
          .catch(() => false);
        return visible;
      })
      .toBe(true);
  });
});

test.describe('独立窗口标题栏', () => {
  test('独立窗口 ACL：最小化 / 最大化按钮可用（capability 覆盖验证）', async ({ helper }) => {
    const standalone = await helper.spawnStandalone('dsh');
    // 精确匹配标题栏角标（空态提示文案里也含「独立窗口」字样）。
    await expect(standalone.getByText('独立窗口', { exact: true })).toBeVisible({
      timeout: 10_000,
    });

    const stateOf = () =>
      standalone.evaluate(() => {
        const qx = (window as unknown as { __qx: { isMaximized(): Promise<boolean> } }).__qx;
        return qx.isMaximized();
      });

    // maximize 按钮授权（allow-toggle-maximize）。JS API 静默吞错，
    // 这里直接 invoke 捕获拒绝原因。
    const toggleErr = await standalone.evaluate<string | null>(async () => {
      try {
        const internals = (
          window as unknown as {
            __TAURI_INTERNALS__: { invoke(cmd: string): Promise<unknown> };
          }
        ).__TAURI_INTERNALS__;
        await internals.invoke('plugin:window|toggle_maximize');
        return null;
      } catch (error) {
        return String(error);
      }
    });
    if (toggleErr) {
      console.log(`[qx-e2e] toggle_maximize 被拒：${toggleErr}`);
    }
    await standalone.waitForTimeout(400);
    expect(await stateOf()).toBe(true);
    await standalone.getByRole('button', { name: '最大化 / 还原' }).click();
    await standalone.waitForTimeout(400);
    expect(await stateOf()).toBe(false);
  });

  test('双击 header：窗口最大化 / 还原（internal_toggle_maximize 授权验证）', async ({
    helper,
  }) => {
    const standalone = await helper.spawnStandalone('dsh');
    await expect(standalone.getByText('独立窗口', { exact: true })).toBeVisible({
      timeout: 10_000,
    });

    const maximizedOf = () =>
      standalone.evaluate(() =>
        (window as unknown as { __qx: { isMaximized(): Promise<boolean> } }).__qx.isMaximized(),
      );
    expect(await maximizedOf()).toBe(false);

    const title = standalone.getByText('DSH · 千寻');
    const box = await title.boundingBox();
    expect(box).toBeTruthy();
    // 最大化带 OS 动画：轮询等待终态，不赌固定延时。
    await standalone.mouse.dblclick(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await expect.poll(maximizedOf, { timeout: 5_000 }).toBe(true);

    await standalone.mouse.dblclick(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await expect.poll(maximizedOf, { timeout: 5_000 }).toBe(false);
  });
});
