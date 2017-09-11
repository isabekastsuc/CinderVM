CARGO   ?= cargo
GO      ?= go
FUZZ_T  ?= 30
BASELINE?= main

RUST_BINS := cinderc cinder cinder-fuzz
GO_PKGS   := ./cmd/... ./internal/...

.DEFAULT_GOAL := all
.PHONY: all release test verify bench fuzz corpus isa clean install docker help

all:
	$(CARGO) build --workspace
	$(GO) build $(GO_PKGS)
