# Convenience entry points; task definitions live in Makefile.toml.
.DEFAULT_GOAL := build

CARGO ?= cargo
ARGS ?=

.PHONY: build install install-chat run gesh check test ci py-build help

build:
	$(CARGO) make -- build --release --locked

install:
	$(CARGO) make install-cli

install-chat:
	$(CARGO) make install-chat

run:
	$(CARGO) make -- run $(ARGS)

gesh:
	uv run --project gesh gesh $(ARGS)

check test ci py-build:
	$(CARGO) make $@

help:
	@printf '%s\n' \
	  'make                       Build the release CLIs (target/release/nblm, gemini-chat)' \
	  'make install               Build and install/update nblm in the Cargo bin directory' \
	  'make install-chat          Build and install/update gemini-chat in the Cargo bin directory' \
	  'make run ARGS="--help"      Rebuild as needed and run the CLI from source' \
	  'make gesh                  Run gesh, the shell for Gemini Enterprise chat and notebooks (needs uv)' \
	  'make check                 Check the Rust core and CLI' \
	  'make test                  Run Rust core and CLI tests' \
	  'make ci                    Check Rust formatting, Clippy, and tests' \
	  'make py-build              Rebuild both editable Python SDKs (nblm and gemini_chat)' \
	  'Requires GNU Make, Rust/Cargo, and cargo-make (cargo install cargo-make).'
