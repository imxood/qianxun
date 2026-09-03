#!/usr/bin/env node
/**
 * Android 壳配置注入（幂等，可重复运行）：
 * 1. 写 res/xml/network_security_config.xml：放行明文 HTTP——网关地址是
 *    EasyTier 虚拟网 IP（http://<ip>:17400），每台设备的 IP 由组网决定，
 *    network_security_config 只能按域名/IP 逐条放行、不支持网段，
 *    故用 base-config 全局放行明文；壳自身不向公网发起 HTTP（WebView
 *    只加载本地资产与用户粘贴的虚拟网地址），实际暴露面仍是虚拟网内。
 * 2. AndroidManifest <application> 注入 networkSecurityConfig 引用。
 * 用法：node scripts/apply-android-config.mjs（需已运行 cap add android）。
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import process from 'node:process';

const androidMain = join(
  dirname(fileURLToPath(import.meta.url)),
  '..',
  'android',
  'app',
  'src',
  'main',
);
const manifestPath = join(androidMain, 'AndroidManifest.xml');
const xmlDir = join(androidMain, 'res', 'xml');
const configPath = join(xmlDir, 'network_security_config.xml');

if (!existsSync(manifestPath)) {
  console.error('[apply-android-config] 未找到 AndroidManifest.xml —— 先运行 `pnpm add:android`。');
  process.exit(1);
}

// 1) 网络安全配置（幂等：内容一致则跳过）。
mkdirSync(xmlDir, { recursive: true });
const securityXml = `<?xml version="1.0" encoding="utf-8"?>
<!-- 千寻移动壳：放行明文 HTTP（网关地址为 EasyTier 虚拟网 IP，
     network_security_config 不支持按网段放行，见 scripts/apply-android-config.mjs 头注）。 -->
<network-security-config>
  <base-config cleartextTrafficPermitted="true">
    <trust-anchors>
      <certificates src="system" />
    </trust-anchors>
  </base-config>
</network-security-config>
`;
if (existsSync(configPath) && readFileSync(configPath, 'utf8') === securityXml) {
  console.log('[apply-android-config] network_security_config.xml 已是最新，跳过。');
} else {
  writeFileSync(configPath, securityXml);
  console.log('[apply-android-config] 已写入 res/xml/network_security_config.xml');
}

// 2) Manifest 注入 android:networkSecurityConfig（幂等）。
let manifest = readFileSync(manifestPath, 'utf8');
if (manifest.includes('android:networkSecurityConfig')) {
  console.log('[apply-android-config] Manifest 已含 networkSecurityConfig，跳过。');
} else {
  const updated = manifest.replace(
    /<application\b([^>]*)\/?>/,
    (match, attrs) =>
      `<application${attrs} android:networkSecurityConfig="@xml/network_security_config">`,
  );
  if (updated === manifest) {
    console.error('[apply-android-config] 未能定位 <application> 标签，Manifest 未修改。');
    process.exit(1);
  }
  manifest = updated;
  writeFileSync(manifestPath, manifest);
  console.log('[apply-android-config] 已在 Manifest 注入 networkSecurityConfig。');
}
