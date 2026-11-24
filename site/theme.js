(() => {
  const storageKey = 'subtitlenow-site-theme';
  const systemTheme = window.matchMedia('(prefers-color-scheme: dark)');
  let preference;
  try {
    const stored = localStorage.getItem(storageKey);
    if (stored === 'light' || stored === 'dark') preference = stored;
  } catch { /* The switch also works when browser storage is unavailable. */ }

  function applyTheme(theme) {
    document.documentElement.dataset.theme = theme;
    document.querySelector('meta[name="theme-color"]')?.setAttribute(
      'content', theme === 'dark' ? '#0b1220' : '#f4f4f7'
    );
    document.querySelectorAll('[data-theme-choice]').forEach(button => {
      button.setAttribute('aria-pressed', String(button.dataset.themeChoice === theme));
    });
  }

  // Run before styles load to avoid flashing the opposite saved palette.
  applyTheme(preference ?? (systemTheme.matches ? 'dark' : 'light'));
  systemTheme.addEventListener('change', event => {
    if (!preference) applyTheme(event.matches ? 'dark' : 'light');
  });
  document.addEventListener('DOMContentLoaded', () => {
    document.querySelectorAll('[data-theme-choice]').forEach(button => {
      button.addEventListener('click', () => {
        preference = button.dataset.themeChoice;
        applyTheme(preference);
        try { localStorage.setItem(storageKey, preference); } catch { /* Session-only. */ }
      });
    });
    applyTheme(document.documentElement.dataset.theme);
    document.querySelector('.theme-switch').hidden = false;
  });
})();
