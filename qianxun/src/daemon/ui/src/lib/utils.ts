// Stage 7a 通用工具 — cn, type helpers
// 与 qianxun-desktop/src/lib/utils.ts 保持一致, 但 Web Console 路径独立.
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]): string {
	return twMerge(clsx(inputs));
}

// 任意 HTML 元素的 ref 绑定 helper
export type WithElementRef<T, U extends HTMLElement = HTMLElement> = T & {
	ref?: U | null;
};
