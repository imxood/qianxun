import React, { useState } from 'react';
import {
  Clipboard,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import type { Palette } from '../theme';
import { radius, space, type } from '../theme';
import type { AddResult } from '../useConnections';

interface AddSheetProps {
  colors: Palette;
  onClose: () => void;
  onAdd: (url: string, name: string) => Promise<AddResult>;
  onScan: () => void;
}

/** 添加连接：iOS 页面式表单。扫码 → 打开取景层，识别成功自动回填。 */
export function AddSheet(props: AddSheetProps) {
  const { colors } = props;
  const [url, setUrl] = useState('');
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const [adding, setAdding] = useState(false);

  const submit = async () => {
    if (adding) {
      return;
    }
    setAdding(true);
    const result = await props.onAdd(url.trim(), name.trim());
    setAdding(false);
    if (result.ok) {
      props.onClose();
    } else {
      setError(result.message);
    }
  };

  const canSubmit = url.trim().length > 0;

  return (
    <View style={styles.root}>
      <Pressable
        style={styles.backdrop}
        onPress={props.onClose}
        accessibilityLabel="关闭"
      />
      <KeyboardAvoidingView
        style={styles.avoid}
        behavior={Platform.OS === 'android' ? undefined : 'padding'}
      >
        <View style={[styles.sheet, { backgroundColor: colors.card }]}>
          <View style={styles.grabber}>
            <View
              style={[
                styles.grabberBar,
                { backgroundColor: colors.tertiaryLabel },
              ]}
            />
          </View>
          <View style={styles.header}>
            <Pressable onPress={props.onClose} hitSlop={8}>
              <Text style={[type.body, { color: colors.tint }]}>取消</Text>
            </Pressable>
            <Text style={[type.headline, { color: colors.label }]}>
              添加连接
            </Text>
            <Pressable
              onPress={() => void submit()}
              hitSlop={8}
              disabled={!canSubmit || adding}
            >
              <Text
                style={[
                  type.body,
                  {
                    color:
                      canSubmit && !adding ? colors.tint : colors.tertiaryLabel,
                    fontWeight: '600',
                  },
                ]}
              >
                {adding ? '…' : '添加'}
              </Text>
            </Pressable>
          </View>

          <View style={[styles.form, { backgroundColor: colors.card }]}>
            <View style={[styles.fieldGroup, { backgroundColor: colors.bg }]}>
              <Text
                style={[
                  type.caption,
                  styles.fieldLabel,
                  { color: colors.secondaryLabel },
                ]}
              >
                配对链接
              </Text>
              <TextInput
                style={[
                  type.subheadline,
                  styles.fieldInput,
                  { color: colors.label },
                ]}
                placeholder="http://IP:端口/qx-gate?token=…"
                placeholderTextColor={colors.tertiaryLabel}
                value={url}
                onChangeText={text => {
                  setUrl(text);
                  setError('');
                }}
                autoCapitalize="none"
                autoCorrect={false}
                keyboardType="url"
                multiline
              />
            </View>
            <View style={[styles.fieldGroup, { backgroundColor: colors.bg }]}>
              <Text
                style={[
                  type.caption,
                  styles.fieldLabel,
                  { color: colors.secondaryLabel },
                ]}
              >
                备注（可选）
              </Text>
              <TextInput
                style={[
                  type.subheadline,
                  styles.fieldInput,
                  { color: colors.label },
                ]}
                placeholder="电脑名称"
                placeholderTextColor={colors.tertiaryLabel}
                value={name}
                onChangeText={setName}
                maxLength={30}
              />
            </View>
            {error !== '' && (
              <Text
                style={[type.footnote, styles.error, { color: colors.red }]}
              >
                {error}
              </Text>
            )}
          </View>

          <View style={[styles.actions, { backgroundColor: colors.card }]}>
            <Pressable
              accessibilityRole="button"
              android_ripple={{ color: colors.fill }}
              style={({ pressed }) => [
                styles.action,
                pressed && styles.pressed,
              ]}
              onPress={props.onScan}
            >
              <Text style={[type.body, { color: colors.tint }]}>扫码填写</Text>
            </Pressable>
            <View
              style={[
                styles.actionSeparator,
                { backgroundColor: colors.separator },
              ]}
            />
            <Pressable
              accessibilityRole="button"
              android_ripple={{ color: colors.fill }}
              style={({ pressed }) => [
                styles.action,
                pressed && styles.pressed,
              ]}
              onPress={() => {
                Clipboard.getString()
                  .then(text => {
                    if (text.trim()) {
                      setUrl(text.trim());
                      setError('');
                    }
                  })
                  .catch(() => setError('无法读取剪贴板'));
              }}
            >
              <Text style={[type.body, { color: colors.tint }]}>粘贴</Text>
            </Pressable>
          </View>

          <Text
            style={[
              type.caption,
              styles.footnote,
              { color: colors.tertiaryLabel },
            ]}
          >
            配对令牌等同电脑控制权，请妥善保管
          </Text>
        </View>
      </KeyboardAvoidingView>
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
  avoid: { justifyContent: 'flex-end' },
  sheet: {
    borderTopLeftRadius: radius.sheet,
    borderTopRightRadius: radius.sheet,
    paddingBottom: space.xl,
  },
  grabber: {
    alignItems: 'center',
    paddingTop: space.s,
    paddingBottom: space.xs,
  },
  grabberBar: { width: 36, height: 5, borderRadius: 2.5 },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: space.l,
    paddingVertical: space.m,
  },
  form: { paddingHorizontal: space.l, gap: space.m },
  fieldGroup: {
    borderRadius: radius.list,
    paddingHorizontal: space.l,
    paddingTop: space.xs,
    paddingBottom: space.s,
  },
  fieldLabel: { marginTop: space.xs },
  fieldInput: { paddingVertical: space.xs },
  error: { marginTop: -space.xs },
  actions: {
    marginTop: space.l,
    marginHorizontal: space.l,
    borderRadius: radius.list,
    overflow: 'hidden',
  },
  action: { minHeight: 50, alignItems: 'center', justifyContent: 'center' },
  actionSeparator: { height: StyleSheet.hairlineWidth },
  pressed: { opacity: 0.55 },
  footnote: {
    textAlign: 'center',
    marginTop: space.l,
    marginHorizontal: space.xl,
  },
});
