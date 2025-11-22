PREFIX := /usr/local
APPLET := cosmic-ext-applet-connect
ICON := sc-apps-kdeconnectindicator.svg

build:
	@echo "Building $(APPLET)..."
	cargo build --release

run: 
	@echo "Running $(APPLET)..."
	./target/release/$(APPLET)

all: build run

install:
	@echo "Installing $(APPLET)..."
	install -Dm0755 ./target/release/$(APPLET) $(PREFIX)/bin/$(applet)
	install -Dm0644 ./data/icons/$(ICON) $(PREFIX)/share/icons/hicolor/scalable/apps/kdeconnect.svg
	install -Dm0644 ./data/$(APPLET).desktop $(PREFIX)/share/applications/$(APPLET).desktop

uninstall:
	@echo "Uninstalling $(APPLET)..."
	rm $(PREFIX)/bin/$(APPLET)
	rm $(PREFIX)/share/icons/hicolor/scalable/apps/kdeconnect.svg
	rm $(PREFIX)/share/applications/$(APPLET).desktop	
