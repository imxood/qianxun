// 千寻 VPS Web UI — 主应用 (Stage 6c vanilla JS, 无框架).
//
// 状态机: localStorage.qxvps_token 存在 → 切到 app; 缺失 → 切到 login.
// app: 3 栏 (项目 / 会话 / 聊天) + 流式 SSE 消费.
//
// 模块:
//   1. fetchWithAuth()       — 统一加 Bearer token
//   2. token 管理             — localStorage 读写 + 401 重登
//   3. 项目列表               — GET /api/projects
//   4. 会话管理               — Stage 6c 简化: 一项目一会话, 点 "新建" 调 daemon /v1/chat/session
//   5. 聊天流 (SSE 消费)      — fetch + ReadableStream, 按 `\n\n` 切帧, 解析 data: 行
//   6. markdown 渲染          — Stage 6c 简化: 仅简单转义 + 换行, 真正 markdown-it 留 Stage 7

(function () {
  "use strict";

  // ─── 1. fetchWithAuth ────────────────────────────────────

  /// 统一封装 fetch: 自动加 Authorization: Bearer <token>.
  /// 401 时清 token + 跳回登录页.
  async function fetchWithAuth(url, opts) {
    opts = opts || {};
    opts.headers = Object.assign({}, opts.headers || {});
    const token = getToken();
    if (token) {
      opts.headers["Authorization"] = "Bearer " + token;
    }
    if (opts.body && typeof opts.body === "object" && !(opts.body instanceof FormData)) {
      opts.headers["Content-Type"] = "application/json";
      opts.body = JSON.stringify(opts.body);
    }
    const res = await fetch(url, opts);
    if (res.status === 401) {
      clearToken();
      showLogin("登录已过期, 请重新登录");
      throw new Error("401 unauthorized");
    }
    return res;
  }

  // ─── 2. token 管理 ─────────────────────────────────────

  const TOKEN_KEY = "qxvps_token";
  const DAEMON_URL_KEY = "qxvps_daemon_url";

  function getToken() {
    try { return localStorage.getItem(TOKEN_KEY) || ""; } catch (_) { return ""; }
  }
  function setToken(t) {
    try { localStorage.setItem(TOKEN_KEY, t); } catch (_) {}
  }
  function clearToken() {
    try { localStorage.removeItem(TOKEN_KEY); } catch (_) {}
  }
  function getDaemonUrl() {
    try {
      return localStorage.getItem(DAEMON_URL_KEY) || "http://127.0.0.1:23900";
    } catch (_) {
      return "http://127.0.0.1:23900";
    }
  }
  function setDaemonUrl(u) {
    try { localStorage.setItem(DAEMON_URL_KEY, u); } catch (_) {}
  }

  // ─── 屏幕切换 ────────────────────────────────────────

  function showLogin(msg) {
    document.getElementById("app").classList.remove("active");
    document.getElementById("login").classList.add("active");
    if (msg) {
      const m = document.getElementById("msg");
      m.textContent = msg;
      m.className = "msg show error";
    }
  }
  function showApp() {
    document.getElementById("login").classList.remove("active");
    document.getElementById("app").classList.add("active");
    // 渲染 daemon URL
    document.getElementById("daemonUrl").value = getDaemonUrl();
    // 启动: 拉项目列表
    loadProjects();
  }

  // ─── 3. 项目列表 (侧栏) ──────────────────────────────

  let activeProject = null;
  let activeSession = null;
  let sessions = {}; // project_id -> [session_id, ...]

  async function loadProjects() {
    const list = document.getElementById("projectList");
    list.innerHTML = '<div class="list-empty">加载中…</div>';
    try {
      const res = await fetchWithAuth("/api/projects");
      if (!res.ok) {
        list.innerHTML = '<div class="list-empty">加载失败 (' + res.status + ')</div>';
        return;
      }
      const projects = await res.json();
      if (!projects.length) {
        list.innerHTML = '<div class="list-empty">暂无项目</div>';
        return;
      }
      list.innerHTML = "";
      projects.forEach((p) => {
        const el = document.createElement("div");
        el.className = "list-item";
        el.dataset.id = p.id;
        el.innerHTML = '<span>' + escapeHtml(p.name) + '</span><span class="meta">' + (p.archived ? '已归档' : '') + '</span>';
        el.addEventListener("click", () => selectProject(p));
        list.appendChild(el);
      });
      // 默认选中第一个
      if (projects[0]) selectProject(projects[0]);
    } catch (e) {
      list.innerHTML = '<div class="list-empty">网络错误</div>';
    }
  }

  function selectProject(p) {
    activeProject = p;
    // 高亮
    document.querySelectorAll("#projectList .list-item").forEach((el) => {
      el.classList.toggle("active", el.dataset.id === p.id);
    });
    // 更新中栏
    document.getElementById("sessionsTitle").textContent = p.name + " · 会话";
    // 渲染已存在的 session
    renderSessions();
    // 重置 chat 面板
    activeSession = null;
    document.getElementById("chatTitle").textContent = "未选择会话";
    document.getElementById("chatMeta").textContent = "";
    document.getElementById("messages").innerHTML = '<div class="msg-bubble system">从左侧选一个项目, 然后点击 "新建会话" 开始对话</div>';
  }

  function renderSessions() {
    const list = document.getElementById("sessionList");
    const items = sessions[activeProject.id] || [];
    if (!items.length) {
      list.innerHTML = '<div class="list-empty">点击 "新建会话" 开始</div>';
      return;
    }
    list.innerHTML = "";
    items.forEach((s) => {
      const el = document.createElement("div");
      el.className = "list-item";
      el.dataset.id = s.id;
      const short = s.id.length > 14 ? s.id.slice(0, 12) + "…" : s.id;
      el.innerHTML = '<span>' + escapeHtml(short) + '</span><span class="meta">' + new Date(s.created).toLocaleTimeString() + '</span>';
      if (activeSession && activeSession.id === s.id) el.classList.add("active");
      el.addEventListener("click", () => selectSession(s));
      list.appendChild(el);
    });
  }

  // ─── 4. 会话管理 (Stage 6c 简化) ──────────────────────

  async function newSession() {
    if (!activeProject) return;
    const btn = document.getElementById("newSessionBtn");
    btn.disabled = true;
    try {
      // 调 daemon /v1/chat/session — Stage 6c 通过 web 直连 daemon
      // daemon 返回 { session_id }, 我们包一层本地元数据
      const res = await fetch(getDaemonUrl() + "/v1/chat/session", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      if (!res.ok) {
        appendSystem("创建会话失败: " + res.status + " (确认 daemon 在 " + getDaemonUrl() + ")");
        return;
      }
      const data = await res.json();
      const session = {
        id: data.session_id,
        created: Date.now(),
      };
      if (!sessions[activeProject.id]) sessions[activeProject.id] = [];
      sessions[activeProject.id].unshift(session);
      renderSessions();
      selectSession(session);
    } catch (e) {
      appendSystem("创建会话失败: " + e.message);
    } finally {
      btn.disabled = false;
    }
  }

  function selectSession(s) {
    activeSession = s;
    document.getElementById("chatTitle").textContent = "会话 " + s.id.slice(0, 12);
    document.getElementById("chatMeta").textContent = s.id;
    // 高亮
    document.querySelectorAll("#sessionList .list-item").forEach((el) => {
      el.classList.toggle("active", el.dataset.id === s.id);
    });
    // 清空消息 (Stage 6c 简化: 不做持久化)
    document.getElementById("messages").innerHTML = '<div class="msg-bubble system">新会话 — 输入消息后回车发送</div>';
  }

  // ─── 5. 聊天流 (SSE 消费) ────────────────────────────

  let isStreaming = false;

  function appendMessage(role, text) {
    const messages = document.getElementById("messages");
    const div = document.createElement("div");
    div.className = "msg-bubble " + role;
    div.innerHTML = '<div class="role">' + role + '</div><div class="text">' + escapeHtml(text) + '</div>';
    messages.appendChild(div);
    scrollToBottom();
    return div;
  }
  function appendSystem(text) {
    appendMessage("system", text);
  }
  function appendStreaming(text) {
    const messages = document.getElementById("messages");
    let last = messages.querySelector(".msg-bubble.assistant.streaming");
    if (!last) {
      last = document.createElement("div");
      last.className = "msg-bubble assistant streaming";
      last.innerHTML = '<div class="role">assistant <span class="streaming-indicator"></span></div><div class="text"></div>';
      messages.appendChild(last);
    }
    last.querySelector(".text").textContent += text;
    scrollToBottom();
  }
  function finalizeStreaming() {
    const messages = document.getElementById("messages");
    const last = messages.querySelector(".msg-bubble.assistant.streaming");
    if (last) last.classList.remove("streaming");
  }

  function scrollToBottom() {
    const m = document.getElementById("messages");
    m.scrollTop = m.scrollHeight;
  }

  async function sendMessage() {
    if (isStreaming) return;
    const ta = document.getElementById("composer");
    const text = ta.value.trim();
    if (!text) return;
    if (!activeSession) {
      appendSystem("请先选择或新建一个会话");
      return;
    }
    ta.value = "";
    appendMessage("user", text);
    isStreaming = true;
    document.getElementById("sendBtn").disabled = true;

    const url = getDaemonUrl() + "/v1/chat/session/" + encodeURIComponent(activeSession.id) + "/prompt";
    try {
      const res = await fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ messages: [{ role: "user", content: text }] }),
      });
      if (!res.ok || !res.body) {
        appendSystem("请求失败: " + res.status);
        isStreaming = false;
        document.getElementById("sendBtn").disabled = false;
        return;
      }
      // 读 SSE 流: 按 chunk 累计 buffer, 遇到 \n\n 切帧
      const reader = res.body.getReader();
      const decoder = new TextDecoder("utf-8");
      let buffer = "";
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        // 切完整帧
        let idx;
        while ((idx = buffer.indexOf("\n\n")) !== -1) {
          const frame = buffer.slice(0, idx);
          buffer = buffer.slice(idx + 2);
          handleSseFrame(frame);
        }
      }
      // 流结束: 残留 buffer 当作最后一帧
      if (buffer.trim()) handleSseFrame(buffer);
      finalizeStreaming();
    } catch (e) {
      appendSystem("网络错误: " + e.message);
    } finally {
      isStreaming = false;
      document.getElementById("sendBtn").disabled = false;
    }
  }

  /// 解析单个 SSE 帧 (可能包含多行). 仅取 `data:` 行, 忽略 `event:` / 注释 / 空行.
  function handleSseFrame(frame) {
    const lines = frame.split("\n");
    for (const raw of lines) {
      const line = raw.replace(/\r$/, "");
      if (!line || line.startsWith(":")) continue;
      const m = line.match(/^data:\s?(.*)$/);
      if (!m) continue;
      const payload = m[1];
      if (!payload) continue;
      try {
        const ev = JSON.parse(payload);
        handleSseEvent(ev);
      } catch (_) {
        // 忽略非 JSON 行 (心跳等)
      }
    }
  }

  /// 分发 SseEvent 12 类型 (与 shared-contract §3.2 严格一致).
  function handleSseEvent(ev) {
    switch (ev.type) {
      case "message_start":
        // 已经在 stream 开始时显示 streaming 占位, 不额外动作
        break;
      case "content_block_start":
        // 文本块开始
        break;
      case "text_delta":
        appendStreaming(ev.text || "");
        break;
      case "thinking_delta":
        appendStreaming("[thinking] " + (ev.text || ""));
        break;
      case "tool_use_delta":
      case "tool_use_complete":
        appendStreaming("\n[tool: " + (ev.name || "?") + "]\n");
        break;
      case "tool_result":
        appendStreaming("\n[result: " + (ev.is_error ? "ERROR " : "") + (ev.content || "").slice(0, 200) + "]\n");
        break;
      case "content_block_stop":
      case "message_delta":
        // 收尾信号, 等 message_stop 统一 finalize
        break;
      case "usage":
        // token 计数: 简化不显示
        break;
      case "message_stop":
        finalizeStreaming();
        break;
      case "error":
        appendSystem("错误: " + (ev.message || ev.code || "未知"));
        finalizeStreaming();
        break;
    }
  }

  // ─── 工具: HTML 转义 ────────────────────────────────

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
    }[c]));
  }

  // ─── 初始化 ─────────────────────────────────────────

  document.addEventListener("DOMContentLoaded", () => {
    // 登录表单 (login.js 也绑了同一个 form; 这里再加一层拦截是 noop, 保留)
    // daemon URL 设置
    const urlInput = document.getElementById("daemonUrl");
    urlInput.addEventListener("change", () => setDaemonUrl(urlInput.value.trim()));
    // 新建会话
    document.getElementById("newSessionBtn").addEventListener("click", newSession);
    // 登出
    document.getElementById("logoutBtn").addEventListener("click", () => {
      clearToken();
      showLogin();
    });
    // 发送按钮
    document.getElementById("sendBtn").addEventListener("click", sendMessage);
    // 输入框: Enter 发送 / Shift+Enter 换行
    const ta = document.getElementById("composer");
    ta.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        sendMessage();
      }
    });
    // 启动: 根据 token 决定显示哪个屏幕
    if (getToken()) showApp();
    // (login.js 也会触发 showApp; 这是双保险, 后者 noop 因为 form listener 先跑)
  });

  // 暴露给 login.js 复用 + 单测 (Stage 6c: 暴露 fetchWithAuth 便于验证 Authorization header)
  window.qxvps = { setToken, showApp, showLogin, fetchWithAuth, getToken };
})();
