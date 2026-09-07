import React, { useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import type { Palette } from '../theme';
import { space, type } from '../theme';
import type { Connection } from '../types';
import { SURFACES } from '../surfaces';

interface WorkspaceScreenProps {
  colors: Palette;
  conn: Connection;
  onExit: () => void;
}

/** 连接工作台：同一配对连接上的多个 Surface（导航栏 + 底部 Surface 栏）。 */
export function WorkspaceScreen({ colors, conn, onExit }: WorkspaceScreenProps) {
  const [activeId, setActiveId] = useState(SURFACES[0].id);
  const active = SURFACES.find(surface => surface.id === activeId) ?? SURFACES[0];
  const ActiveSurface = active.component;

  return (
    <View style={styles.root}>
      <View style={[styles.nav, { backgroundColor: colors.card, borderBottomColor: colors.separator }]}>
        <Pressable accessibilityRole="button" hitSlop={8} onPress={onExit} style={styles.back}>
          <Text style={[type.body, styles.backGlyph, { color: colors.tint }]}>‹</Text>
          <Text style={[type.body, { color: colors.tint }]}>连接</Text>
        </Pressable>
        <Text style={[type.headline, styles.title, { color: colors.label }]} numberOfLines={1}>
          {conn.name}
        </Text>
        <View style={styles.back} />
      </View>

      <View style={styles.surface}>
        <ActiveSurface conn={conn} />
      </View>

      <View style={[styles.tabBar, { backgroundColor: colors.card, borderTopColor: colors.separator }]}>
        {SURFACES.map(surface => {
          const selected = surface.id === activeId;
          return (
            <Pressable
              key={surface.id}
              accessibilityRole="tab"
              accessibilityState={{ selected }}
              style={({ pressed }) => [styles.tab, pressed && styles.pressed]}
              onPress={() => setActiveId(surface.id)}
            >
              <Text
                style={[
                  type.caption,
                  { color: selected ? colors.tint : colors.secondaryLabel, fontWeight: selected ? '600' : '400' },
                ]}
              >
                {surface.title}
              </Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  nav: {
    flexDirection: 'row',
    alignItems: 'center',
    height: 52,
    paddingHorizontal: space.s,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  back: { flexDirection: 'row', alignItems: 'center', minWidth: 72, paddingLeft: space.xs },
  backGlyph: { fontSize: 26, marginTop: -2 },
  title: { flex: 1, textAlign: 'center' },
  surface: { flex: 1 },
  tabBar: {
    flexDirection: 'row',
    borderTopWidth: StyleSheet.hairlineWidth,
    minHeight: 49,
    paddingBottom: 2,
  },
  tab: { flex: 1, alignItems: 'center', justifyContent: 'center', minHeight: 44 },
  pressed: { opacity: 0.55 },
});
