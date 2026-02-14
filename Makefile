.PHONY: fmt lint test build contracts-verify ci

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

ci:
	mise run ci
