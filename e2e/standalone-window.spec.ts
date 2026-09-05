import { test, expect } from './_fixture';

/**
 * 独立窗口 + 标签拖拽 e2e（WebView2 CDP，debug 构建 → ~/.qianxun_dev）。
 * 全程不启动 DSH、不写 PIN（无落盘污染）。
 */

test.describe('独立终端窗口', () => {
  test('分离独立窗口：新 page 出现并自动建一个标签', async ({ helper }) => {
    const standalone = await helper.spawnStandalone('terminal');
    await expect(standalone.locator('[data-testid="terminal-tab"]')).toHaveCount(1, {
      timeout: 10_000,
    });
    // 独立窗口标识（标题栏「独立窗口」角标）。
    await expect(standalone.getByText('独立窗口', { exact: true })).toBeVisible();
  });

  test('会话归属隔离：独立窗口的会话不出现在主窗清单', async ({ main, helper }) => {
    const standalone = await helper.spawnStandalone('terminal');
    await expect(standalone.locator('[data-testid="terminal-tab"]')).toHaveCount(1);

    // 独立窗口里再建一个，并取两个会话 id。
    await standalone.locator('[data-testid="terminal-new"]').click();
    await expect(standalone.locator('[data-testid="terminal-tab"]')).toHaveCount(2);
    const standaloneSessions = await helper.call<Array<{ id: number }>>(
      standalone,
      'terminal_sessions',
    );
    expect(standaloneSessions).toHaveLength(2);

    // 主窗（main 名下）看不到独立窗口的会话。
    const mainSessions = await helper.call<Array<{ id: number }>>(main, 'terminal_sessions');
    const mainIdSet = new Set(mainSessions.map((item) => item.id));
    for (const session of standaloneSessions) {
      expect(mainIdSet.has(session.id)).toBe(false);
    }
  });

  test('标签合并回主窗：转移后会话归属 main，主窗 UI 接管', async ({ main, helper }) => {
    const standalone = await helper.spawnStandalone('terminal');
    await expect(standalone.locator('[data-testid="terminal-tab"]')).toHaveCount(1);
    const sessions = await helper.call<Array<{ id: number; title: string; shell: string }>>(
      standalone,
      'terminal_sessions',
    );
    const target = sessions[0]!;

    // 独立窗口把标签转移给 main（等价「合并到主窗口」的 IPC 语义）。
    await helper.call(standalone, 'terminal_transfer', {
      id: target.id,
      target: 'main',
      title: 'e2e 合并标签',
      shell: target.shell,
      cwd: null,
      pinId: null,
    });

    // 主窗侧栏进终端页 → TerminalPage 挂载 → recoverSessions 接管。
    await main.locator('[data-testid="nav-terminal"]').click();
    await expect(
      main.locator('[data-testid="terminal-tab"]', { hasText: 'e2e 合并标签' }),
    ).toBeVisible({ timeout: 10_000 });

    // 清理：杀掉接管来的会话（避免残留进程）。
    const ids = await helper.call<Array<{ id: number }>>(main, 'terminal_sessions');
    for (const session of ids) {
      await helper.call(main, 'terminal_kill', { id: session.id });
    }
  });

  test('独立窗口关闭：名下会话被终结', async ({ main, helper }) => {
    const standalone = await helper.spawnStandalone('terminal');
    await expect(standalone.locator('[data-testid="terminal-tab"]')).toHaveCount(1);
    const sessions = await helper.call<Array<{ id: number }>>(standalone, 'terminal_sessions');
    const label = await standalone.evaluate<string>(
      () => (window as unknown as { __qx: { windowLabel: string } }).__qx.windowLabel,
    );

    // 强制关闭（绕过确认流，等价确认后的路径）→ Rust Destroyed 清理会话。
    // destroy 与 IPC 响应存在竞态：页面先消失时 evaluate 报 closed，忽略。
    await helper.call(standalone, 'window_force_close').catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      if (!message.includes('closed')) throw error;
    });
    await expect
      .poll(
        async () => {
          const rest = await main.evaluate<{ label: string }, Array<{ id: number }>>(
            ({ label: l }) => {
              const handle = (window as unknown as { __qx: { call: typeof Function } }).__qx;
              return handle.call('terminal_sessions', { label: l });
            },
            { label },
          );
          return rest;
        },
        { timeout: 10_000 },
      )
      .toEqual([]);
    void sessions;
  });

  test('独立 DSH 窗口：page 出现且渲染 DSH 页（不依赖 DSH 运行状态）', async ({ helper }) => {
    const standalone = await helper.spawnStandalone('dsh');
    // 独立窗口壳渲染（标题栏角标），且内容区是 DSH 页而非终端页。
    await expect(standalone.getByText('独立窗口', { exact: true })).toBeVisible({
      timeout: 10_000,
    });
    await expect(standalone.locator('[data-testid="terminal-tab"]')).toHaveCount(0);
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
});

test.describe('独立窗口标题栏', () => {
  test('独立窗口 ACL：最小化 / 最大化按钮可用（capability 覆盖验证）', async ({ helper }) => {
    const standalone = await helper.spawnStandalone('terminal');
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
    const standalone = await helper.spawnStandalone('terminal');
    await expect(standalone.getByText('独立窗口', { exact: true })).toBeVisible({
      timeout: 10_000,
    });

    const maximizedOf = () =>
      standalone.evaluate(() =>
        (window as unknown as { __qx: { isMaximized(): Promise<boolean> } }).__qx.isMaximized(),
      );
    expect(await maximizedOf()).toBe(false);

    const title = standalone.getByText('终端 · 千寻');
    const box = await title.boundingBox();
    expect(box).toBeTruthy();
    await standalone.mouse.dblclick(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await standalone.waitForTimeout(400);
    expect(await maximizedOf()).toBe(true);

    await standalone.mouse.dblclick(box!.x + box!.width / 2, box!.y + box!.height / 2);
    await standalone.waitForTimeout(400);
    expect(await maximizedOf()).toBe(false);
  });
});

test.describe('终端标签拖拽重排', () => {
  test('按住标签拖到末尾：顺序重排，会话不受影响', async ({ main }) => {
    // 主窗终端页：建 3 个标签。
    await main.locator('[data-testid="nav-terminal"]').click();
    await main.locator('[data-testid="terminal-new"]').click();
    await main.locator('[data-testid="terminal-new"]').click();
    await expect(main.locator('[data-testid="terminal-tab"]')).toHaveCount(3);

    // 用稳定的 tab id 断言顺序（标题文本会被 shell 的 OSC 标题改写）。
    const orderOf = () =>
      main
        .locator('[data-testid="terminal-tab"]')
        .evaluateAll((els) => els.map((el) => el.getAttribute('data-tab-id')));
    const before = await orderOf();
    expect(before).toHaveLength(3);

    // 拖第一个到第三个位置（移过第二个标签的中点即触发重排）。
    const tabs = main.locator('[data-testid="terminal-tab"]');
    const source = await tabs.first().boundingBox();
    const target = await tabs.nth(2).boundingBox();
    expect(source).toBeTruthy();
    expect(target).toBeTruthy();
    await main.mouse.move(source!.x + source!.width / 2, source!.y + source!.height / 2);
    await main.mouse.down();
    await main.mouse.move(target!.x + target!.width * 0.9, target!.y + target!.height / 2, {
      steps: 12,
    });
    await main.mouse.up();

    // 第一个到了末尾，其余相对顺序保持。
    const after = await orderOf();
    expect(after).toEqual([before![1], before![2], before![0]]);
  });

  test('原位点击不算拖拽：激活行为不受影响', async ({ main }) => {
    await main.locator('[data-testid="nav-terminal"]').click();
    await expect(main.locator('[data-testid="terminal-tab"]')).toHaveCount(1);
    await main.locator('[data-testid="terminal-new"]').click();
    await expect(main.locator('[data-testid="terminal-tab"]')).toHaveCount(2);

    // 点第二个（不带位移）→ 成为激活标签（aria-selected）。
    const tabs = main.locator('[data-testid="terminal-tab"]');
    await tabs.nth(1).click();
    await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
  });
});
