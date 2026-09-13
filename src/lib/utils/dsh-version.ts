/** 从安装说明符（如 `@deepseek-ai/dsh@0.1.5-rc.1`）提取精确版本号。 */
export function pinnedDshVersion(spec: string): string {
  const at = spec.lastIndexOf('@');
  return at >= 0 ? spec.slice(at + 1) : spec;
}
