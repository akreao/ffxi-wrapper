// Same as desktop/ui/theme.js, plus ?theme=light|dark for screenshots.
(function () {
  function apply(theme) {
    if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
    else delete document.documentElement.dataset.theme;
  }
  let saved = null;
  try { saved = localStorage.getItem("theme"); } catch {}
  const q = new URLSearchParams(location.search).get("theme");
  apply(q || saved);
  window.setTheme = function (theme) {
    try {
      if (theme === "light" || theme === "dark") localStorage.setItem("theme", theme);
      else localStorage.removeItem("theme");
    } catch {}
    apply(theme);
  };
})();
