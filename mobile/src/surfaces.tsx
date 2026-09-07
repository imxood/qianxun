import React from 'react';
import { StyleSheet, Text, View } from 'react-native';
import { WebView } from 'react-native-webview';
import type { Connection } from './types';

export interface SurfaceProps {
  conn: Connection;
}

export interface SurfaceDef {
  id: string;
  title: string;
  component: React.ComponentType<SurfaceProps>;
}

/**
 * DSH 工作台 Surface：WebView 加载网关配对链接。
 * 配对 cookie 由 WebView 持久化（sharedCookiesEnabled），二次进入免配对。
 */
function DshSurface({ conn }: SurfaceProps) {
  return (
    <WebView
      source={{ uri: conn.url }}
      style={styles.flex}
      sharedCookiesEnabled
      domStorageEnabled
      javaScriptEnabled
      setSupportMultipleWindows={false}
      allowsBackForwardNavigationGestures
      applicationNameForUserAgent="QianxunMobile/0.1"
    />
  );
}

/**
 * 远程桌面 Surface（二期）：PC 端采集(xcap)+编码(MJPEG→硬编 H.264)+输入注入(enigo)
 * 经网关 /qx-mobile/desktop WS 推流；移动端在此 Surface 内渲染并回传触摸。
 * 与 DSH 工作台共用同一条配对连接——切换零重新鉴权，这就是「无缝」的落点。
 */
function DesktopSurface(_props: SurfaceProps) {
  return (
    <View style={styles.placeholderWrap}>
      <Text style={styles.placeholderTitle}>远程桌面</Text>
      <Text style={styles.placeholderBody}>即将推出</Text>
    </View>
  );
}

/**
 * Surface 注册表：新增能力（远程桌面、监控面板、文件速传…）= 在这里注册一个
 * component，WorkspaceScreen 的切换 pill 自动出现，无需改动壳的其它部分。
 */
export const SURFACES: readonly SurfaceDef[] = [
  { id: 'dsh', title: '工作台', component: DshSurface },
  { id: 'desktop', title: '远程桌面', component: DesktopSurface },
];

const styles = StyleSheet.create({
  flex: { flex: 1 },
  placeholderWrap: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    padding: 28,
    gap: 12,
  },
  placeholderTitle: { fontSize: 20, fontWeight: '700', opacity: 0.7 },
  placeholderBody: { fontSize: 13, textAlign: 'center', lineHeight: 20, opacity: 0.5 },
});
