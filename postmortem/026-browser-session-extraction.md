# Shared Browser Session Handoff

## Context

The ytm-radio helper had accumulated two different responsibilities. It owned
YouTube Music-specific account requests and auth-file construction, but it also
owned browser discovery, process lifecycle, Chromium DevTools, Firefox/Zen
WebDriver BiDi, profile preparation, and browser cookie capture.

The latter work is not specific to YouTube Music. Keeping it inside ytm-radio
would force Chirp and later consumers to duplicate a difficult compatibility
boundary. Moving the entire ytm-radio helper instead would be worse: its
Innertube request construction, authenticated stream resolution, response
cache, and provider auth format are intentionally YouTube Music-specific.

## Decision

Extract the browser-facing boundary to `browser-session` and make ytm-radio an
explicit consumer of its Emacs API.

The login handoff is now:

```text
ytm-radio Elisp
  -> browser-session capture
  -> private generic capture file
  -> ytm-radio-helper auth import-capture
  -> private ytm-radio auth.json
```

browser-session owns browser protocol selection, isolated profile lifecycle,
remote-control conflict reporting, cookie capture, and the temporary capture
file. ytm-radio owns the YouTube Music page expression, validates the captured
origin/session identity, and maps the capture into its existing auth format.
The ytm helper no longer has a browser login command or browser implementation.

## Why

This preserves the original safety property for ytm-radio: its browser cookies
and page context pass from one external helper boundary to another and do not
enter durable Elisp state or helper stdout. It also leaves the shared package
free to offer explicit Elisp capture reads to consumers whose architecture
chooses that trade-off.

The split gives each workflow one owner. browser-session can fix a browser or
protocol incompatibility once. ytm-radio can change its provider credential
format without turning it into a generic browser API.

## Consequences

ytm-radio now requires the browser-session Emacs package and an executable
browser-session helper for account login. Its existing helper release installer
continues to install only the YouTube Music helper; browser-session distribution
is an independent concern until it publishes its own release/install workflow.

The package metadata deliberately declares browser-session as a required
package instead of disguising it as an optional fallback. Therefore a ytm-radio
release is gated on browser-session becoming installable from a configured
package archive; package-lint enforces that order. This is preferable to
shipping a nominally installable ytm-radio release whose account-login command
cannot load its required shared API.

Chrome, Firefox, and Zen retain their automatic isolated profile names under
`~/.ytm-radio/`, so existing user configuration and prepared Firefox-family
profiles remain valid. Firefox/Zen preparation remains an explicit command.
