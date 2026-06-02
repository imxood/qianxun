// 简体中文 — 10 个起步 key. Stage 5 扩展到完整翻译树 (common.* / chat.* / settings.*).

export default {
	// app
	"app.title": "千寻",
	// connection
	"connection.connected": "已连接",
	"connection.degraded": "Daemon 不可达",
	"connection.reconnecting": "正在重新连接 Daemon",
	"connection.offline": "Daemon 未启动",
	// input
	"input.placeholder": "输入消息, Enter 发送, Shift+Enter 换行",
	"input.send": "发送",
	// message
	"message.thinking": "千寻正在思考...",
	// action
	"retry": "立即重试",
	// error
	"error.network": "网络错误, 请检查 Daemon 是否在运行",
} as const;

export type ZhCNMessages = typeof import("./zh-CN").default;
