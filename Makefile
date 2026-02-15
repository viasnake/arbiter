.PHONY: fmt lint test build contracts-verify drift-guard version-check version-bump ci

fmt:
	mise run fmt

lint:
	mise run lint

test:
	mise run test

build:
	mise run build

contracts-verify:
	mise run contracts-verify

drift-guard:
	mise run drift-guard

version-check:
	mise run version-check

version-bump:
	@if [ -z "$(VERSION)" ]; then echo "VERSION is required (e.g. make version-bump VERSION=1.2.1)"; exit 1; fi
	VERSION=$(VERSION) mise run version-bump

ci:
	mise run ci
