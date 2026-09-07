import { TextStyle } from 'react-native';

/** iOS 语义色（HIG）：深浅双态成对定义，组件内禁止散落 hex。 */
export interface Palette {
  bg: string; // systemGroupedBackground
  card: string; // secondarySystemGroupedBackground
  label: string;
  secondaryLabel: string;
  tertiaryLabel: string;
  separator: string;
  fill: string; // secondarySystemFill
  tint: string; // systemBlue
  green: string; // systemGreen
  red: string; // systemRed
}

export function palette(dark: boolean): Palette {
  if (dark) {
    return {
      bg: '#000000',
      card: '#1C1C1E',
      label: '#FFFFFF',
      secondaryLabel: 'rgba(235,235,245,0.6)',
      tertiaryLabel: 'rgba(235,235,245,0.3)',
      separator: 'rgba(84,84,88,0.6)',
      fill: 'rgba(120,120,128,0.24)',
      tint: '#0A84FF',
      green: '#30D158',
      red: '#FF453A',
    };
  }
  return {
    bg: '#F2F2F7',
    card: '#FFFFFF',
    label: '#000000',
    secondaryLabel: 'rgba(60,60,67,0.6)',
    tertiaryLabel: 'rgba(60,60,67,0.3)',
    separator: 'rgba(60,60,67,0.29)',
    fill: 'rgba(120,120,128,0.12)',
    tint: '#007AFF',
    green: '#34C759',
    red: '#FF3B30',
  };
}

/** 系统字体字阶（iOS Text Styles 对齐）。 */
export const type = {
  largeTitle: { fontSize: 34, lineHeight: 41, fontWeight: '700' } as TextStyle,
  title3: { fontSize: 20, lineHeight: 25, fontWeight: '400' } as TextStyle,
  headline: { fontSize: 17, lineHeight: 22, fontWeight: '600' } as TextStyle,
  body: { fontSize: 17, lineHeight: 22, fontWeight: '400' } as TextStyle,
  subheadline: { fontSize: 15, lineHeight: 20, fontWeight: '400' } as TextStyle,
  footnote: { fontSize: 13, lineHeight: 18, fontWeight: '400' } as TextStyle,
  caption: { fontSize: 12, lineHeight: 16, fontWeight: '400' } as TextStyle,
};

export const space = {
  xs: 4,
  s: 8,
  m: 12,
  l: 16,
  xl: 20,
  xxl: 24,
} as const;

export const radius = { list: 10, sheet: 16, control: 8 } as const;
