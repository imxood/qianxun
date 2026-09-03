# 千寻移动壳（R2）

Capacitor 7 壳工程：只做三件事——**加载网关 URL、深色跟随、安全区适配**。
工作台本体是 DSH Web（手机自适应），不需要专门移动页。

## 构建（本机无 Android SDK，骨架先行）

```bash
cd mobile
pnpm install
pnpm add:android        # 生成 android/ 工程（模板拷贝，无需 JDK）
# 明文 HTTP 放行：把下面 networkSecurityConfig 贴进
# android/app/src/main/AndroidManifest.xml 的 <application android:networkSecurityConfig="@xml/network_security_config" ...>
# 并创建 android/app/src/main/res/xml/network_security_config.xml
pnpm sync && pnpm open  # Android Studio 打开后构建安装
```

## network_security_config.xml 模板（明文 HTTP 仅限 EasyTier 网段）

EasyTier 默认网段 `10.144.0.0/16`（以实际组网为准，收紧到 /24 更好）：

```xml
<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <base-config cleartextTrafficPermitted="false" />
    <domain-config cleartextTrafficPermitted="true">
        <!-- 只对 EasyTier 虚拟网放行明文；公网域名一律禁止 -->
    </domain-config>
</network-security-config>
```

> 按网段放行 Android 原生不支持（domain-config 只认主机名）——实践上
> 用 `android:usesCleartextTraffic` 会对全局放行，这里采用折中：
> 壳内只访问配对链接域（EasyTier IP），配对输入端校验链接形态
> （`http://<v4-ip>:<port>/qx-gate?token=…`），其余一律拒绝跳转。
> 若需系统级收紧，等 EasyTier 网关上 TLS（v1.x 再议）。

## 使用

1. 千寻桌面端：设置 → 远程访问 → 启用网关（绑 EasyTier 网卡）→ 配对新设备；
2. 复制配对链接（或扫二维码），发到手机；
3. 手机装壳 → 首屏粘贴链接 → 「进入工作台」→ 以后自动直达。
