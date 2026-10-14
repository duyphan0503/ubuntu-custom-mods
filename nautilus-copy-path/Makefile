.PHONY: all build release install clean

all: release

build:
	cargo build

release:
	cargo build --release

install: release
	mkdir -p $(HOME)/.local/lib $(HOME)/.local/bin $(HOME)/.local/share/nautilus-python/extensions
	cp target/release/libcopypath.so $(HOME)/.local/lib/
	cp target/release/copy-path $(HOME)/.local/bin/
	nautilus -q

clean:
	cargo clean
