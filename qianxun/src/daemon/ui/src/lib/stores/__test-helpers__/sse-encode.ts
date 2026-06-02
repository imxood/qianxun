// 测试 helper — 把一组 SseEvent 对象编码成 SSE wire format
// 用法: encodeSseEvents([{type:'text_delta',...}]) → "data: {json}\n\n..."
//
// 跟 lib/sse/parser.ts 的 processSseChunk 对称.

export function encodeSseEvents(events: object[]): string {
	return events
		.map((e) => `data: ${JSON.stringify(e)}\n\n`)
		.join('');
}
