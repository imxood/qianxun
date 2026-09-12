import React, { useCallback, useEffect, useState } from 'react';
import {
  BackHandler,
  StatusBar,
  StyleSheet,
  useColorScheme,
  View,
} from 'react-native';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';
import { AddSheet } from './src/screens/AddSheet';
import { HomeScreen } from './src/screens/HomeScreen';
import { ScannerScreen } from './src/screens/ScannerScreen';
import { WorkspaceScreen } from './src/screens/WorkspaceScreen';
import { palette } from './src/theme';
import type { Connection } from './src/types';
import { isValidPairUrl } from './src/validate';
import { useConnections } from './src/useConnections';
import type { Notice } from './src/useConnections';

type AppView = { type: 'home' } | { type: 'workspace'; connId: string };

function App(): React.JSX.Element {
  const scheme = useColorScheme();
  const colors = palette(scheme === 'dark');
  const connectionsApi = useConnections();
  const [view, setView] = useState<AppView>({ type: 'home' });
  const [scanning, setScanning] = useState(false);
  const [adding, setAdding] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);

  // Android 返回键：扫码 > 表单 > 工作台 > 系统默认。
  useEffect(() => {
    const subscription = BackHandler.addEventListener(
      'hardwareBackPress',
      () => {
        if (scanning) {
          setScanning(false);
          return true;
        }
        if (adding) {
          setAdding(false);
          return true;
        }
        if (view.type === 'workspace') {
          setView({ type: 'home' });
          return true;
        }
        return false;
      },
    );
    return () => subscription.remove();
  }, [scanning, adding, view.type]);

  /** 扫码识别成功：入库 → 关取景层/表单 → 首页 notice。 */
  const handleCode = useCallback(
    async (text: string): Promise<'invalid' | 'added'> => {
      if (!isValidPairUrl(text)) {
        return 'invalid';
      }
      const result = await connectionsApi.addConnection(text, '');
      setNotice({ kind: result.ok ? 'ok' : 'error', text: result.message });
      setScanning(false);
      setAdding(false);
      return 'added';
    },
    [connectionsApi],
  );

  const openWorkspace = useCallback(
    (conn: Connection) => {
      connectionsApi.touchConnection(conn.id);
      setView({ type: 'workspace', connId: conn.id });
    },
    [connectionsApi],
  );

  const workspaceConn =
    view.type === 'workspace'
      ? connectionsApi.connections.find(conn => conn.id === view.connId)
      : undefined;

  return (
    <SafeAreaProvider>
      <SafeAreaView
        style={[styles.root, { backgroundColor: colors.bg }]}
        edges={['top', 'bottom']}
      >
        <StatusBar
          barStyle={scheme === 'dark' ? 'light-content' : 'dark-content'}
        />
        <View style={[styles.body, { backgroundColor: colors.bg }]}>
          {workspaceConn ? (
            <WorkspaceScreen
              colors={colors}
              conn={workspaceConn}
              onExit={() => setView({ type: 'home' })}
            />
          ) : (
            <HomeScreen
              colors={colors}
              connections={connectionsApi.connections}
              ready={connectionsApi.ready}
              notice={notice}
              onOpen={openWorkspace}
              onOpenAdd={() => setAdding(true)}
              onRename={connectionsApi.renameConnection}
              onUpdateUrl={connectionsApi.updateConnectionUrl}
              onRemove={connectionsApi.removeConnection}
            />
          )}
        </View>
        {adding && (
          <AddSheet
            colors={colors}
            onClose={() => setAdding(false)}
            onAdd={connectionsApi.addConnection}
            onScan={() => setScanning(true)}
          />
        )}
        {scanning && (
          <ScannerScreen
            colors={colors}
            onCancel={() => setScanning(false)}
            onCode={handleCode}
          />
        )}
      </SafeAreaView>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  body: { flex: 1 },
});

export default App;
