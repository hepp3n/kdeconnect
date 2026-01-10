PREFIX := /usr/local
FLATPAK_PREFIX := /app
APPLET := cosmic-ext-applet-connect
ICON := sc-apps-kdeconnectindicator.svg

build:
	@echo "Building $(APPLET)..."
	cargo build --release

build-offline:
	@echo "Building offline $(APPLET)..."
	cargo --offline fetch --manifest-path Cargo.toml --verbose
	cargo --offline build --release --verbose

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

