import { Camera } from 'react-native-camera-kit';
import React, { useEffect, useRef, useState } from 'react';
import {
  PermissionsAndroid,
  Pressable,
  StyleSheet,
  Text,
  Vibration,
  View,
} from 'react-native';
import type { Palette } from '../theme';
import { space, type } from '../theme';

interface ScannerScreenProps {
  colors: Palette;
  onCancel: () => void;
  /** 识别到一段文本：返回 'invalid' 则继续扫码（提示后重试）。 */
  onCode: (text: string) => Promise<'invalid' | 'added'>;
}

const HINT_DEFAULT = '对准配对二维码';
const FRAME = 248;
const ARM = 30;

export function ScannerScreen(props: ScannerScreenProps) {
  const [permission, setPermission] = useState<
    'pending' | 'granted' | 'denied'
  >('pending');
  const [hint, setHint] = useState(HINT_DEFAULT);
  const busy = useRef(false);
  const hintTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );

  useEffect(() => {
    PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.CAMERA)
      .then(state =>
        setPermission(
          state === PermissionsAndroid.RESULTS.GRANTED ? 'granted' : 'denied',
        ),
      )
      .catch(() => setPermission('denied'));
    return () => clearTimeout(hintTimer.current);
  }, []);

  const handleRead = (raw: unknown): void => {
    if (busy.current) {
      return;
    }
    const text = String(
      (raw as { nativeEvent?: { codeStringValue?: string } })?.nativeEvent
        ?.codeStringValue ?? '',
    ).trim();
    if (!text) {
      return;
    }
    busy.current = true;
    void Promise.resolve(props.onCode(text)).then(result => {
      if (result === 'invalid') {
        setHint('不是配对二维码');
        clearTimeout(hintTimer.current);
        hintTimer.current = setTimeout(() => setHint(HINT_DEFAULT), 1600);
        busy.current = false;
        return;
      }
      Vibration.vibrate(40);
      busy.current = false;
    });
  };

  return (
    <View style={styles.root}>
      {permission === 'granted' ? (
        <Camera
          style={StyleSheet.absoluteFill}
          scanBarcode
          showFrame={false}
          onReadCode={handleRead}
        />
      ) : (
        <View style={styles.center}>
          <Text style={[type.subheadline, styles.dimText]}>
            {permission === 'pending'
              ? '正在请求相机权限…'
              : '相机权限未开启，可在系统设置中允许'}
          </Text>
        </View>
      )}
      {permission === 'granted' && (
        <>
          <View style={[styles.dim, styles.dimTop]} pointerEvents="none" />
          <View style={[styles.dim, styles.dimBottom]} pointerEvents="none" />
          <View
            style={[styles.frameSide, styles.frameTop]}
            pointerEvents="none"
          />
          <View
            style={[styles.frameSide, styles.frameBottom]}
            pointerEvents="none"
          />
          <FrameCorners />
          <Text style={[type.footnote, styles.hint]} pointerEvents="none">
            {hint}
          </Text>
        </>
      )}
      <Pressable
        accessibilityLabel="关闭扫码"
        accessibilityRole="button"
        style={({ pressed }) => [styles.close, pressed && styles.pressed]}
        onPress={props.onCancel}
      >
        <Text style={styles.closeGlyph}>✕</Text>
      </Pressable>
    </View>
  );
}

/** 相机取景四角括号（系统相机样式）。 */
function FrameCorners() {
  const corner = {
    position: 'absolute' as const,
    width: ARM,
    height: ARM,
    borderColor: '#FFFFFF',
  };
  const half = FRAME / 2;
  return (
    <View pointerEvents="none">
      <View
        style={[
          corner,
          {
            borderTopWidth: 3,
            borderLeftWidth: 3,
            borderTopLeftRadius: 16,
            left: '50%',
            top: '42%',
            marginLeft: -half,
            marginTop: -half,
          },
        ]}
      />
      <View
        style={[
          corner,
          {
            borderTopWidth: 3,
            borderRightWidth: 3,
            borderTopRightRadius: 16,
            left: '50%',
            top: '42%',
            marginLeft: half - ARM,
            marginTop: -half,
          },
        ]}
      />
      <View
        style={[
          corner,
          {
            borderBottomWidth: 3,
            borderLeftWidth: 3,
            borderBottomLeftRadius: 16,
            left: '50%',
            top: '42%',
            marginLeft: -half,
            marginTop: half - ARM,
          },
        ]}
      />
      <View
        style={[
          corner,
          {
            borderBottomWidth: 3,
            borderRightWidth: 3,
            borderBottomRightRadius: 16,
            left: '50%',
            top: '42%',
            marginLeft: half - ARM,
            marginTop: half - ARM,
          },
        ]}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    backgroundColor: '#000',
  },
  center: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    padding: space.xl,
  },
  dimText: { color: 'rgba(255,255,255,0.85)', textAlign: 'center' },
  dim: {
    position: 'absolute',
    left: 0,
    right: 0,
    height: '21%',
    backgroundColor: 'rgba(0,0,0,0.4)',
  },
  dimTop: { top: 0 },
  dimBottom: { bottom: 0 },
  frameSide: {
    position: 'absolute',
    left: '50%',
    top: '42%',
    width: FRAME,
    height: FRAME,
    marginLeft: -FRAME / 2,
    marginTop: -FRAME / 2,
    borderRadius: 16,
  },
  frameTop: { borderColor: 'transparent' },
  frameBottom: { borderColor: 'transparent' },
  hint: {
    position: 'absolute',
    left: 0,
    right: 0,
    top: '64%',
    textAlign: 'center',
    color: 'rgba(255,255,255,0.92)',
  },
  close: {
    position: 'absolute',
    right: space.l,
    top: space.m,
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: 'rgba(118,118,128,0.3)',
    alignItems: 'center',
    justifyContent: 'center',
  },
  closeGlyph: { color: '#FFFFFF', fontSize: 18 },
  pressed: { opacity: 0.55 },
});
