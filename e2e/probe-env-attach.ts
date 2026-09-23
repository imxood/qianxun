/**
 * 环境页验证(attach 版):假定 pnpm dev 已起(CDP 10222 + vite 5190 就绪),
 * 直接连接,不做任何进程管理。
 */
import { chromium } from '@playwright/test';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname, '..');

async function main(): Promise<void> {
  const browser = await chromium.connectOverCDP('http://127.0.0.1:10222');
  const main = browser.contexts()[0]?.pages()[0];
  if (!main) throw new Error('无 page');
  await main.waitForFunction(
    () => Boolean((window as unknown as { __qx?: unknown }).__qx),
    undefined,
    {
      timeout: 30_000,
    },
  );

  // 回环境页(keep-alive 下可能在别的页)
  await main.locator('[data-testid="nav-env"]').click();
  await main.waitForSelector('[data-testid="laya-line"]', { timeout: 20_000 });
  await new Promise((r) => setTimeout(r, 1500));

  const layaLine = await main.locator('[data-testid="laya-line"]').innerText();
  const qwenLine = await main.locator('[data-testid="qwen-line"]').innerText();
  const moliLine = await main.locator('[data-testid="moli-line"]').innerText();
  console.log(`[env-e2e] laya: ${layaLine}`);
  console.log(`[env-e2e] qwen: ${qwenLine}`);
  console.log(`[env-e2e] moli: ${moliLine}`);

  await main.screenshot({ path: path.join(ROOT, '.tmp', 'env-page-dark.png') });

  // Laya 配置展开
  await main.locator('button', { hasText: '配置' }).first().click();
  await new Promise((r) => setTimeout(r, 300));
  await main.screenshot({ path: path.join(ROOT, '.tmp', 'env-page-config.png') });
  await main.locator('button', { hasText: '收起' }).first().click();

  console.log('[env-e2e] ✓ 环境页验证完成');
  process.exit(0);
}

void main();
