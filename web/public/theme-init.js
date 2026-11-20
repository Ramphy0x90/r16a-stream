// Applies the stored theme before first paint so the page never flashes the wrong colours.
// An external file, because the CSP forbids inline scripts. Dark is the default.
(function () {
  try {
    var t = localStorage.getItem('r16a-stream.theme');
    if (t === 'light' || t === 'dark' || t === 'system') {
      document.documentElement.setAttribute('data-theme', t);
    }
  } catch (e) {
    // Storage can be unavailable in private windows, the default theme then applies.
  }
})();
