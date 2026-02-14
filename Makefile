.PHONY: fmt lint test build contracts-verify drift-guard ci

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

ci:
	mise run ci
