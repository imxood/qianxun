# 千寻移动壳（R2）

Capacitor 7 壳工程：只做三件事——**加载网关 URL、深色跟随、安全区适配**。
工作台本体是 DSH Web（手机自适应），不需要专门移动页。

## 构建（工具链在 `D:\programs`，免 Android Studio）

本机工具链（v0.2 起已装，重装时按此布局）：

- JDK 21（Temurin）→ `D:\programs\java\jdk-21`，`JAVA_HOME` 指向它；
- Android cmdline-tools → `D:\programs\android\sdk\cmdline-tools\latest`，`ANDROID_HOME` 指向 `D:\programs\android\sdk`；
- SDK 组件：`platform-tools`、`platforms;android-35`、`build-tools;35.0.0`；`platform-tools` 加入 PATH（adb）。

构建命令：

```bash
cd mobile
pnpm install --ignore-workspace   # mobile 独立于工作区根 lockfile
pnpm add:android                  # 生成 android/ 工程 + 自动注入明文放行配置（幂等脚本）
# 首次或依赖变化后：pnpm sync
# 出包（JAVA_HOME/ANDROID_HOME 未进用户环境时，临时设置后运行）：
#   $env:JAVA_HOME='D:\programs\java\jdk-21'; $env:ANDROID_HOME='D:\programs\android\sdk'
android\gradlew.bat assembleDebug # 产物 android/app/build/outputs/apk/debug/app-debug.apk
# 真机安装：adb install -r app-debug.apk
```

## 明文 HTTP 放行（已脚本化）

`scripts/apply-android-config.mjs`（`pnpm add:android` 末尾自动执行，可单独
`pnpm apply:config` 重放）：

- 写 `res/xml/network_security_config.xml`：base-config 放行明文——网关地址是
  EasyTier 虚拟网 IP（每台设备 IP 由组网决定），network_security_config 只认
  主机名/单 IP、不支持网段，故全局放行；壳自身不向公网发起 HTTP，实际暴露面
  仍是虚拟网内；
- Manifest `<application>` 注入 `android:networkSecurityConfig` 引用。

## 使用

1. 千寻桌面端：侧栏「远程」→ 启用网关（绑 EasyTier 网卡）→ 配对新设备；
2. 复制配对链接（或扫二维码），发到手机；
3. 手机装壳 → 首屏粘贴链接 → 「进入工作台」→ 以后自动直达。
