PREFIX := /usr/local
FLATPAK_PREFIX := /app
APPLET := cosmic-ext-applet-connect
ICON := sc-apps-kdeconnectindicator.svg

build:
	@echo "Building $(APPLET)..."
	cargo build --release

build-vendored: vendor-extract
	@echo "Building vendored $(APPLET)..."
	cargo build --release --frozen --offline

check:
	cargo clippy

run: 
	@echo "Running $(APPLET)..."
	./target/release/$(APPLET)

all: build run

install:
	@echo "Installing $(APPLET)..."
	install -Dm0755 ./target/release/$(APPLET) $(PREFIX)/bin/$(APPLET)
	install -Dm0644 ./data/icons/$(ICON) $(PREFIX)/share/icons/hicolor/scalable/apps/kdeconnect.svg
	install -Dm0644 ./data/$(APPLET).desktop $(PREFIX)/share/applications/$(APPLET).desktop

install-flatpak:
	@echo "Installing flatpak version of $(APPLET)..."
	install -Dm0755 ./target/release/$(APPLET) $(FLATPAK_PREFIX)/bin/$(APPLET)
	install -Dm0644 ./data/icons/$(ICON) $(FLATPAK_PREFIX)/share/icons/hicolor/scalable/apps/kdeconnect.svg
	install -Dm0644 ./data/$(APPLET).desktop $(FLATPAK_PREFIX)/share/applications/$(APPLET).desktop

uninstall:
	@echo "Uninstalling $(APPLET)..."
	rm $(PREFIX)/bin/$(APPLET)
	rm $(PREFIX)/share/icons/hicolor/scalable/apps/kdeconnect.svg
	rm $(PREFIX)/share/applications/$(APPLET).desktop	

vendor: SHELL:=/bin/bash
vendor:
	mkdir -p .cargo
	cargo vendor --sync Cargo.toml | head -n -1 > .cargo/config.toml
	echo 'directory = "vendor"' >> .cargo/config.toml
	tar pcf vendor.tar .cargo vendor
	rm -rf .cargo vendor

vendor-extract:
	rm -rf vendor
	tar pxf vendor.tar
