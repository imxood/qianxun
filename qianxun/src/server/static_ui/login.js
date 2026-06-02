// 千寻 VPS 登录页 — Stage 5 vanilla JS, 无框架.
//
// 提交 → POST /api/auth/login
// 成功 → 把 JWT 存 localStorage, 跳转到 /ui/  (Stage 6: 真实 dashboard)
// 失败 → 在 #msg 区域显示错误.
//
// 不接 refresh token / 401 自动刷新 — Stage 6+ 接.

(function () {
  "use strict";

  const form = document.getElementById("loginForm");
  const submitBtn = document.getElementById("submitBtn");
  const msgEl = document.getElementById("msg");

  function showMsg(text, kind) {
    msgEl.textContent = text;
    msgEl.className = "msg show " + (kind || "error");
  }

  function clearMsg() {
    msgEl.textContent = "";
    msgEl.className = "msg";
  }

  form.addEventListener("submit", async function (e) {
    e.preventDefault();
    clearMsg();

    const username = document.getElementById("username").value.trim();
    const password = document.getElementById("password").value;

    if (!username || !password) {
      showMsg("请输入用户名和密码", "error");
      return;
    }

    submitBtn.disabled = true;
    submitBtn.textContent = "登录中…";

    try {
      const res = await fetch("/api/auth/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username: username, password: password }),
      });

      if (!res.ok) {
        // 401/400/500 都按错误处理
        const text = await res.text();
        showMsg("登录失败: " + (text || res.statusText), "error");
        return;
      }

      const data = await res.json();
      if (!data || !data.token) {
        showMsg("服务器返回格式异常", "error");
        return;
      }

      // 存 token, 跳转到主页 (Stage 6 才是真正的 dashboard, 目前是占位)
      try {
        localStorage.setItem("qxvps_token", data.token);
      } catch (_) {
        // localStorage 不可用 (隐私模式等) — 不阻塞流程
      }
      showMsg("登录成功, 正在跳转…", "ok");
      // Stage 5 暂没 dashboard, 跳到 /api/health 确认 token (Stage 6 替换)
      setTimeout(function () {
        window.location.href = "/api/health";
      }, 600);
    } catch (err) {
      showMsg("网络错误: " + (err && err.message ? err.message : err), "error");
    } finally {
      submitBtn.disabled = false;
      submitBtn.textContent = "登录";
    }
  });
})();
