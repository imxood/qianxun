import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  AppState,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import type { Palette } from '../theme';
import { space, type } from '../theme';
import { probeInfo } from '../probe';
import type { Connection } from '../types';
import type { Notice } from '../useConnections';
import { hostOf } from '../validate';

interface HomeScreenProps {
  colors: Palette;
  connections: Connection[];
  ready: boolean;
  notice: Notice | null;
  onOpen: (conn: Connection) => void;
  onOpenAdd: () => void;
  onRename: (id: string, name: string) => void;
  onUpdateUrl: (id: string, url: string) => { ok: boolean; message: string };
  onRemove: (id: string) => void;
}

type DotState = 'on' | 'off' | 'unknown';

type Menu = { conn: Connection; mode: 'menu' | 'rename' | 'update' } | null;

export function HomeScreen(props: HomeScreenProps) {
  const { colors, connections, ready, notice } = props;
  const [dots, setDots] = useState<Record<string, DotState>>({});
  const [menu, setMenu] = useState<Menu>(null);
  const [menuName, setMenuName] = useState('');
  const [menuUrl, setMenuUrl] = useState('');
  const appState = useRef(AppState.currentState);

  const sorted = [...connections].sort(
    (a, b) =>
      (b.lastUsedAt || 0) - (a.lastUsedAt || 0) ||
      (b.addedAt || 0) - (a.addedAt || 0),
  );

  const refreshStatuses = useCallback(() => {
    if (appState.current !== 'active') {
      return;
    }
    sorted.forEach((conn, index) => {
      setTimeout(() => {
        void probeInfo(conn).then(info => {
          setDots(prev => ({ ...prev, [conn.id]: info ? 'on' : 'off' }));
        });
      }, index * 120);
    });
  }, [connections]);

  useEffect(() => {
    refreshStatuses();
    const interval = setInterval(refreshStatuses, 20_000);
    const sub = AppState.addEventListener('change', next => {
      appState.current = next;
      if (next === 'active') {
        refreshStatuses();
      }
    });
    return () => {
      clearInterval(interval);
      sub.remove();
    };
  }, [refreshStatuses]);

  const statusText = (state: DotState | undefined) =>
    state === 'on' ? '在线' : state === 'off' ? '离线' : '…';

  return (
    <View style={styles.flex}>
      <View style={styles.nav}>
        <Text style={[type.largeTitle, { color: colors.label }]}>连接</Text>
        <Pressable
          accessibilityLabel="添加连接"
          accessibilityRole="button"
          hitSlop={12}
          style={[styles.plus, { backgroundColor: colors.fill }]}
          onPress={props.onOpenAdd}
        >
          <Text style={[styles.plusGlyph, { color: colors.tint }]}>+</Text>
        </Pressable>
      </View>

      <ScrollView
        style={styles.flex}
        contentContainerStyle={styles.content}
        keyboardShouldPersistTaps="handled"
      >
        {!ready ? null : sorted.length === 0 ? (
          <View style={styles.empty}>
            <Text style={[type.headline, { color: colors.secondaryLabel }]}>
              未添加连接
            </Text>
            <Text style={[type.footnote, { color: colors.tertiaryLabel }]}>
              点右上角 + 添加电脑
            </Text>
          </View>
        ) : (
          <View style={[styles.list, { backgroundColor: colors.card }]}>
            {sorted.map((conn, index) => {
              const state = dots[conn.id];
              return (
                <Pressable
                  key={conn.id}
                  accessibilityLabel={`${conn.name}，${statusText(state)}`}
                  android_ripple={{ color: colors.fill }}
                  style={({ pressed }) => [
                    styles.row,
                    pressed && styles.pressed,
                  ]}
                  onPress={() => props.onOpen(conn)}
                >
                  <View
                    style={[
                      styles.dot,
                      {
                        backgroundColor:
                          state === 'on'
                            ? colors.green
                            : state === 'off'
                              ? colors.red
                              : colors.tertiaryLabel,
                      },
                    ]}
                  />
                  <View style={styles.rowMeta}>
                    <Text
                      style={[type.headline, { color: colors.label }]}
                      numberOfLines={1}
                    >
                      {conn.name}
                    </Text>
                    <Text
                      style={[type.footnote, { color: colors.secondaryLabel }]}
                      numberOfLines={1}
                    >
                      {hostOf(conn.url)} · {statusText(state)}
                    </Text>
                  </View>
                  <Pressable
                    accessibilityLabel="更多操作"
                    accessibilityRole="button"
                    hitSlop={8}
                    style={({ pressed }) => [
                      styles.more,
                      pressed && styles.pressed,
                    ]}
                    onPress={() => {
                      setMenuName(conn.name);
                      setMenuUrl(conn.url);
                      setMenu({ conn, mode: 'menu' });
                    }}
                  >
                    <Text
                      style={[
                        styles.moreGlyph,
                        { color: colors.secondaryLabel },
                      ]}
                    >
                      ⋯
                    </Text>
                  </Pressable>
                  {index < sorted.length - 1 && (
                    <View
                      style={[
                        styles.separator,
                        { backgroundColor: colors.separator },
                      ]}
                    />
                  )}
                </Pressable>
              );
            })}
          </View>
        )}

        {notice && (
          <Text
            style={[
              type.footnote,
              styles.notice,
              {
                color:
                  notice.kind === 'ok' ? colors.secondaryLabel : colors.red,
              },
            ]}
          >
            {notice.text}
          </Text>
        )}
      </ScrollView>

      {menu !== null && (
        <View style={styles.overlay}>
          <Pressable style={styles.backdrop} onPress={() => setMenu(null)} />
          <View style={[styles.sheet, { backgroundColor: colors.card }]}>
            {menu.mode === 'menu' && (
              <>
                <Text
                  style={[
                    type.footnote,
                    styles.sheetHeader,
                    { color: colors.secondaryLabel },
                  ]}
                  numberOfLines={1}
                >
                  {menu.conn.name}
                </Text>
                <MenuItem
                  colors={colors}
                  label="重命名"
                  onPress={() => setMenu({ conn: menu.conn, mode: 'rename' })}
                />
                <MenuItem
                  colors={colors}
                  label="更新链接"
                  onPress={() => setMenu({ conn: menu.conn, mode: 'update' })}
                />
                <MenuItem
                  colors={colors}
                  label="删除连接"
                  destructive
                  onPress={() => {
                    props.onRemove(menu.conn.id);
                    setMenu(null);
                  }}
                />
                <View
                  style={[
                    styles.sheetCancelGap,
                    { backgroundColor: colors.bg },
                  ]}
                />
                <MenuItem
                  colors={colors}
                  label="取消"
                  bold
                  onPress={() => setMenu(null)}
                  cancel
                />
              </>
            )}
            {menu.mode === 'rename' && (
              <>
                <Text
                  style={[
                    type.footnote,
                    styles.sheetHeader,
                    { color: colors.secondaryLabel },
                  ]}
                >
                  名称
                </Text>
                <View
                  style={[styles.fieldWrap, { backgroundColor: colors.bg }]}
                >
                  <TextInput
                    style={[
                      type.body,
                      styles.textInput,
                      { color: colors.label },
                    ]}
                    value={menuName}
                    onChangeText={setMenuName}
                    maxLength={30}
                    autoFocus
                  />
                </View>
                <MenuItem
                  colors={colors}
                  label="保存"
                  bold
                  onPress={() => {
                    props.onRename(menu.conn.id, menuName);
                    setMenu(null);
                  }}
                />
                <MenuItem
                  colors={colors}
                  label="取消"
                  onPress={() => setMenu(null)}
                  cancel
                />
              </>
            )}
            {menu.mode === 'update' && (
              <>
                <Text
                  style={[
                    type.footnote,
                    styles.sheetHeader,
                    { color: colors.secondaryLabel },
                  ]}
                >
                  配对链接
                </Text>
                <View
                  style={[styles.fieldWrap, { backgroundColor: colors.bg }]}
                >
                  <TextInput
                    style={[
                      type.subheadline,
                      styles.textInput,
                      { color: colors.label },
                    ]}
                    value={menuUrl}
                    onChangeText={setMenuUrl}
                    autoCapitalize="none"
                    autoCorrect={false}
                    keyboardType="url"
                    multiline
                  />
                </View>
                <MenuItem
                  colors={colors}
                  label="保存"
                  bold
                  onPress={() => {
                    const result = props.onUpdateUrl(
                      menu.conn.id,
                      menuUrl.trim(),
                    );
                    if (result.ok) {
                      setMenu(null);
                    }
                  }}
                />
                <MenuItem
                  colors={colors}
                  label="取消"
                  onPress={() => setMenu(null)}
                  cancel
                />
              </>
            )}
          </View>
        </View>
      )}
    </View>
  );
}

function MenuItem(props: {
  colors: Palette;
  label: string;
  destructive?: boolean;
  bold?: boolean;
  cancel?: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      android_ripple={{ color: props.colors.fill }}
      style={({ pressed }) => [styles.menuItem, pressed && styles.pressed]}
      onPress={props.onPress}
    >
      <Text
        style={[
          type.body,
          {
            color: props.destructive ? props.colors.red : props.colors.tint,
            fontWeight: props.bold ? '600' : '400',
          },
        ]}
      >
        {props.label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  nav: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: space.l,
    paddingTop: space.s,
    paddingBottom: space.s,
  },
  plus: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: 'center',
    justifyContent: 'center',
  },
  plusGlyph: { fontSize: 24, fontWeight: '400', marginTop: -2 },
  content: { paddingHorizontal: space.l, paddingBottom: space.xxl },
  list: { borderRadius: 10, overflow: 'hidden' },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.m,
    paddingHorizontal: space.l,
    minHeight: 64,
  },
  pressed: { opacity: 0.55 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  rowMeta: { flex: 1, minWidth: 0, paddingVertical: space.m, gap: 2 },
  more: {
    width: 36,
    height: 36,
    alignItems: 'center',
    justifyContent: 'center',
  },
  moreGlyph: { fontSize: 22, fontWeight: '600' },
  separator: {
    position: 'absolute',
    left: 48,
    right: 0,
    bottom: 0,
    height: StyleSheet.hairlineWidth,
  },
  empty: { alignItems: 'center', gap: space.s, paddingTop: 96 },
  notice: { textAlign: 'center', marginTop: space.l },
  overlay: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    justifyContent: 'flex-end',
  },
  backdrop: {
    position: 'absolute',
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    backgroundColor: 'rgba(0,0,0,0.4)',
  },
  sheet: {
    borderTopLeftRadius: 16,
    borderTopRightRadius: 16,
    paddingBottom: 24,
  },
  sheetHeader: { textAlign: 'center', paddingVertical: space.m },
  sheetCancelGap: { height: space.s, marginTop: space.s },
  menuItem: {
    minHeight: 56,
    alignItems: 'center',
    justifyContent: 'center',
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: 'rgba(120,120,128,0.2)',
  },
  fieldWrap: {
    marginHorizontal: space.l,
    borderRadius: 10,
    marginBottom: space.s,
  },
  textInput: { padding: space.m },
});
