.PHONY: fmt lint test build ci

fmt:
	mise run fmt

lint:
	mise run lint

test:
	mise run test

build:
	mise run build

ci:
	mise run ci
