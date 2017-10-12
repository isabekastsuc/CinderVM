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

release:
	$(CARGO) build --release --workspace --locked
	CGO_ENABLED=0 $(GO) build -trimpath -ldflags='-s -w' -o target/release/ ./cmd/cinderd

# `make test` is what CI runs; keep the ordering — the corpus depends on cinderc.
test:
	$(CARGO) test --all-features --workspace
	$(CARGO) run --quiet --bin cinder -- corpus corpus/ --expect corpus/MANIFEST.tsv
	$(GO) test $(GO_PKGS) -race -count=1
