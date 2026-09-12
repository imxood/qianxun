# 千寻移动端（React Native）

千寻的 Android App：**多 PC 连接管理**（扫码/粘贴配对、自动命名、在线状态、随时切换）

- **Workspace 多 Surface 骨架**（工作台 / 远程桌面——同一配对连接，切换零重新鉴权）。

技术栈：React Native 0.87（新架构）+ react-native-webview（DSH 工作台 Surface）

- react-native-camera-kit（原生扫码）+ 原生剪贴板/AsyncStorage。

```
mobile/
  App.tsx / src/        RN 壳（连接管理、扫码、Workspace+Surface 注册表）
  android/              生成的 Android 工程（gradle 已配国内镜像与本机 SDK 版本）
  test/mock-gateway.mjs 验证用 mock 网关（node 零依赖）
  examples/             网关定制层示例（custom.css/js，与壳技术无关）
```

## 连接与配对

1. 千寻桌面端：设置 → 远程访问 → 启用网关（绑 EasyTier 网卡）→ 配对新设备；
2. 手机 App「扫码添加」对准桌面端二维码，或「粘贴链接」/ 手输；
3. 添加时自动读 `/qx-mobile/info` 识别**电脑主机名**命名，列表实时显示在线状态；
4. 点卡片进入工作台（DSH Web，WebView 内 cookie 持久化，二次直达）。

每台电脑一条连接卡片：重命名 / 更新配对链接（token 轮换后免删重建）/ 删除。
不同电脑 = 不同源（EasyTier IP），cookie 各自独立。

## 远程桌面（二期规划）

Workspace 底部 pill 已预留「远程桌面」Surface（`src/surfaces.tsx` 注册表）：
PC 端 xcap 采集 + 编码 + enigo 输入注入，经网关 `/qx-mobile/desktop` WS 推流；
移动端在同一配对连接上渲染与操作，与 DSH 工作台无缝切换。

## 构建（本机已验证：JDK = Android Studio JBR，SDK = D:\programs\Android\Sdk）

```bash
cd mobile
npm install --registry=https://registry.npmmirror.com
cd android && ..\gradlew assembleRelease    # 或根目录 npx react-native run-android
```

- `android/build.gradle` 已把 compileSdk/buildTools/NDK 对齐本机已有版本
  （36 / 36.1.0 / 28.2），`local.properties` 指向本机 SDK——不触发 SDK 下载；
- gradle 发行版与 maven 依赖走腾讯/阿里镜像（wrapper + repositories 已配）；
- 产物：`android/app/build/outputs/apk/release/app-release.apk`（debug 签名）。

## 安全注记

- 配对令牌等同电脑控制权：仅存于本机 AsyncStorage，勿分享链接；
- 明文 HTTP：manifest 以 `usesCleartextTraffic` 占位符放行（EasyTier 网段折中，
  gradle 里置 true；网关 TLS 化后收紧为 false）；
- 相机权限仅用于扫码配对，运行时向用户请求。
