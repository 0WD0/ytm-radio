EMACS ?= emacs
CARGO ?= cargo
PACKAGE_LINT_PATH ?= $(HOME)/.emacs.d/straight/repos/package-lint
COMPAT_PATH ?= $(HOME)/.emacs.d/straight/repos/compat
+BROWSER_SESSION_PATH ?= ../browser-session
ifeq ($(strip $(BROWSER_SESSION_PATH)),)
BROWSER_SESSION_PATH := ../browser-session
endif

.PHONY: check compile test checkdoc package-lint helper-check helper-test clean

# helper-check runs before test so the Elisp helper-contract tests find a
# freshly built debug helper instead of skipping.
check: compile helper-check test checkdoc package-lint

compile:
	$(EMACS) -Q --batch -L . -L $(BROWSER_SESSION_PATH) -f batch-byte-compile ytm-radio.el test/ytm-radio-test.el

test:
	$(EMACS) -Q --batch -L . -L $(BROWSER_SESSION_PATH) -l ytm-radio.el -l test/ytm-radio-test.el -f ert-run-tests-batch-and-exit

checkdoc:
	$(EMACS) -Q --batch -L . -L $(BROWSER_SESSION_PATH) --eval "(progn (require 'checkdoc) (dolist (file '(\"ytm-radio.el\")) (with-current-buffer (find-file-noselect file) (let ((checkdoc-create-error-function #'error)) (checkdoc-current-buffer t)))))"

package-lint:
	@if [ ! -f "$(PACKAGE_LINT_PATH)/package-lint.el" ]; then echo "package-lint unavailable at $(PACKAGE_LINT_PATH)" >&2; exit 1; fi
	$(EMACS) -Q --batch -L . -L $(BROWSER_SESSION_PATH) -L $(COMPAT_PATH) -L $(PACKAGE_LINT_PATH) -l package-lint --eval "(setq package-lint-batch-fail-on-warnings t)" -f package-lint-batch-and-exit ytm-radio.el

helper-check:
	$(CARGO) fmt --manifest-path helper/Cargo.toml -- --check
	$(CARGO) clippy --manifest-path helper/Cargo.toml --locked -- -D warnings
	$(MAKE) helper-test
	$(CARGO) build --manifest-path helper/Cargo.toml --locked

helper-test:
	$(CARGO) test --manifest-path helper/Cargo.toml --locked

clean:
	rm -f *.elc test/*.elc
	$(CARGO) clean --manifest-path helper/Cargo.toml
