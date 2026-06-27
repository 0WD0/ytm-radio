# Zen Browser Login

Zen Browser is Firefox-derived and exposes the same WebDriver BiDi automation
shape that ytm-radio already uses for Firefox login. Supporting Zen should not
introduce a separate browser-cookie database path or a copied-header workflow.
Those alternatives would duplicate authentication handling outside the browser
protocol boundary.

The helper now recognizes Zen by its common executable names and application
paths, including `zen`, `zen-bin`, `zen-browser`, and `Zen Browser.app`. Explicit
`ytm-radio-helper-login-browser` values can use `zen`, and default browser
detection can accept desktop entries or app bundles that point at Zen.

Zen uses the Firefox-compatible WebDriver BiDi path and the same operational
constraints as Firefox. If an existing Zen process was started without the
helper's remote-control port, users should close it before login or configure an
isolated login profile so the helper can start a separate browser process.
