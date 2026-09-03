# For packagers and for anyone installing without nix. `cargo install` copies
# the binary and nothing else — no man page — and whoever ran
# `cargo install --git` has no checkout to run this from either, so both routes
# exist and neither replaces the other.

PREFIX  ?= /usr/local
DESTDIR ?=
CARGO   ?= cargo

BINDIR := $(DESTDIR)$(PREFIX)/bin
MANDIR := $(DESTDIR)$(PREFIX)/share/man/man1

.PHONY: all build install uninstall clean

all: build

build:
	$(CARGO) build --release --locked

install: build
	install -Dm755 target/release/swaykeys $(BINDIR)/swaykeys
	install -Dm644 man/man1/swaykeys.1 $(MANDIR)/swaykeys.1

uninstall:
	rm -f $(BINDIR)/swaykeys $(MANDIR)/swaykeys.1

clean:
	$(CARGO) clean
