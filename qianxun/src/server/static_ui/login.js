// 千寻 VPS 登录页 — Stage 5 vanilla JS, 无框架.
//
// 提交 → POST /api/auth/login
// 成功 → 把 JWT 存 localStorage, 调 qxvps.showApp() 切到主界面 (app.js 提供)
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
        const text = await res.text();
        showMsg("登录失败: " + (text || res.statusText), "error");
        return;
      }

      const data = await res.json();
      if (!data || !data.token) {
        showMsg("服务器返回格式异常", "error");
        return;
      }

      // 存 token, 切到主界面 (Stage 6c: 不再跳 /api/health 占位)
      window.qxvps.setToken(data.token);
      showMsg("登录成功", "ok");
      setTimeout(() => window.qxvps.showApp(), 200);
    } catch (err) {
      showMsg("网络错误: " + (err && err.message ? err.message : err), "error");
    } finally {
      submitBtn.disabled = false;
      submitBtn.textContent = "登录";
    }
  });
})();
