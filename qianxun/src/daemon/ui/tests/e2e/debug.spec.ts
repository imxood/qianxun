// Smoke test: open _ui/ and dump content
import { test, expect } from '@playwright/test';

test('debug _ui load 2', async ({ page }) => {
	const logs: string[] = [];
	page.on('console', (msg) => logs.push(`[${msg.type()}] ${msg.text()}`));
	page.on('pageerror', (err) => logs.push(`[pageerror] ${err.message}`));
	page.on('framenavigated', (f) => logs.push(`[nav] ${f.url()}`));
	page.on('request', (req) => {
		if (req.resourceType() === 'document') logs.push(`[req-doc] ${req.url()}`);
	});
	page.on('response', (res) => {
		if (res.request().resourceType() === 'document') logs.push(`[res-doc] ${res.url()} ${res.status()}`);
	});

	const response = await page.goto('http://127.0.0.1:23913/ui/', { waitUntil: 'networkidle', timeout: 30_000 });
	console.log('STATUS:', response?.status());
	console.log('URL:', page.url());

	// Dump body
	const html = await page.content();
	console.log('HTML LENGTH:', html.length);

	// Wait a bit for redirect
	await page.waitForTimeout(2000);
	console.log('URL AFTER WAIT:', page.url());
	const html2 = await page.content();
	console.log('HTML2 LENGTH:', html2.length);
	console.log('HTML2 FIRST 1000:', html2.substring(0, 1000));

	for (const l of logs) console.log('LOG:', l);

	// Try to find token input
	const tokens = await page.locator('[data-testid="token-input"]').count();
	console.log('TOKEN INPUT COUNT:', tokens);
});
