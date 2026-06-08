// qianxun-desktop/src/lib/utils/id.ts

let counter = 0;

export function genId(prefix: string): string {
	counter++;
	return `${prefix}_${Date.now().toString(36)}_${counter.toString(36)}`;
}
