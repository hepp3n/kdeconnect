PREFIX := /usr/local
FLATPAK_PREFIX := /app
APPLET := cosmic-ext-applet-connect
APPID := dev.heppen.CosmicExtConnect
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
	install -Dm0644 ./resources/icons/$(ICON) $(PREFIX)/share/icons/hicolor/scalable/apps/$(APPID).svg
	install -Dm0644 ./resources/$(APPID).desktop $(PREFIX)/share/applications/$(APPID).desktop
	install -Dm0644 ./resources/$(APPID).metainfo.xml $(PREFIX)/share/metainfo/$(APPID).metainfo.xml

install-flatpak:
	@echo "Installing flatpak version of $(APPLET)..."
	install -Dm0755 ./target/release/$(APPLET) $(FLATPAK_PREFIX)/bin/$(APPLET)
	install -Dm0644 ./resources/icons/$(ICON) $(FLATPAK_PREFIX)/share/icons/hicolor/scalable/apps/$(APPID).svg
	install -Dm0644 ./resources/$(APPID).desktop $(FLATPAK_PREFIX)/share/applications/$(APPID).desktop
	install -Dm0644 ./resources/$(APPID).metainfo.xml $(FLATPAK_PREFIX)/share/metainfo/$(APPID).metainfo.xml

uninstall:
	@echo "Uninstalling $(APPLET)..."
	rm $(PREFIX)/bin/$(APPLET)
	rm $(PREFIX)/share/icons/hicolor/scalable/apps/kdeconnect.svg
	rm $(PREFIX)/share/applications/$(APPID).desktop	
	rm ${PREFIX}/share/metainfo/${APPID}.metainfo.xml

