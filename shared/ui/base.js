/* portable-kit 共享前端工具库：统一 invoke / DOM / 错误文本 / toast / 主题。
   三个 app 共用，避免重复实现。通过 window.PKUI 暴露。 */
window.PKUI = (() => {
  const invoke = window.__TAURI__?.core?.invoke;
  const listen = window.__TAURI__?.event?.listen;
  const $ = (id) => document.getElementById(id);
  const errorText = (e) => (typeof e === 'string' ? e : e?.message || String(e));
  const systemDark = window.matchMedia('(prefers-color-scheme: dark)');

  let toastTimer = null;
  function toast(message, type = 'info', duration = 2600) {
    const el = $('toast');
    if (!el) return;
    clearTimeout(toastTimer);
    el.textContent = message;
    el.className = `toast show${type === 'error' ? ' error' : ''}`;
    toastTimer = setTimeout(() => { el.className = 'toast'; }, duration);
  }

  function applyTheme(theme) {
    const dark = theme === 'dark' || (theme === 'system' && systemDark.matches);
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  }

  function requireInvoke() {
    if (!invoke) throw new Error('未检测到 Tauri 运行环境。');
    return invoke;
  }

  return { invoke, listen, $, errorText, toast, applyTheme, systemDark, requireInvoke };
})();
