import type { CapacitorConfig } from '@capacitor/cli';

/**
 * 千寻移动壳配置（R2）。
 *
 * 壳本体只承载一个「网关地址」页（www/）：首次启动粘贴千寻配对链接
 * （http://<EasyTier-IP>:17400/qx-gate?token=…）存 localStorage，
 * 之后直接跳转。DSH Web 自适应手机布局，无需专门移动页。
 *
 * androidScheme 用 https；明文 HTTP 仅对 EasyTier 网段放行
 * （见 README：networkSecurityConfig 模板，装壳时贴进 AndroidManifest）。
 */
const config: CapacitorConfig = {
  appId: 'com.qianxun.mobile',
  appName: '千寻',
  webDir: 'www',
  android: {
    allowMixedContent: false,
  },
  server: {
    androidScheme: 'https',
    // 壳页自己就是入口；网关 URL 由 www/index.html 在运行时接管。
    hostname: 'app.qianxun.local',
    // 网关地址是 http://<IP>:17400（EasyTier IP 因组网而异）——不放行
    // 应用内导航的话 location.replace 会弹系统选择器跳到外部浏览器
    // （真机实测）。Capacitor 7 从 server.allowNavigation 读取，
    // HostMask 语义下 '*' 匹配任意主机。
    allowNavigation: ['*'],
  },
};

export default config;
