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
	$(CARGO) test --test e2e -- --test-threads=1

verify:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets --all-features -- -D warnings
	$(GO) vet $(GO_PKGS)
	$(GO) run honnef.co/go/tools/cmd/staticcheck@latest $(GO_PKGS)
	$(MAKE) isa
	git diff --exit-code -- docs/isa.md

bench:
	$(CARGO) bench --bench dispatch -- --save-baseline $(BASELINE)
	$(CARGO) bench --bench snapshot -- --save-baseline $(BASELINE)

fuzz:
	$(CARGO) run --profile fuzz --bin cinder-fuzz -- --seconds $(FUZZ_T) --corpus corpus/accept

corpus:
	$(CARGO) run --quiet --bin cinder -- corpus corpus/ --expect corpus/MANIFEST.tsv --verbose

# docs/isa.md is generated from the const table in src/isa.rs. `verify` fails on drift.
isa:
	$(CARGO) run --quiet --bin cinderc -- --emit-isa-md > docs/isa.md

