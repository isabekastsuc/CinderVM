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

miri:
	MIRIFLAGS='-Zmiri-strict-provenance' $(CARGO) +nightly miri test --lib heap:: cont::

install:
	$(CARGO) install --path . --locked
	$(GO) install ./cmd/cinderd

docker:
	docker build -t cindervm:$(shell git rev-parse --short HEAD) .

clean:
	$(CARGO) clean
	$(GO) clean -cache -testcache
	rm -f corpus/*.cdxb

help:
	@printf '%-12s %s\n' \
	  all      'debug build, both halves' \
	  release  'LTO release build' \
	  test     'unit + corpus + go + cross-language e2e' \
	  verify   'fmt, clippy -D warnings, vet, staticcheck, isa drift' \
	  bench    'criterion, baseline-compared' \
	  fuzz     'bytecode fuzzing (FUZZ_T=seconds)' \
	  miri     'miri over heap:: and cont::' \
	  isa      'regenerate docs/isa.md from src/isa.rs'
