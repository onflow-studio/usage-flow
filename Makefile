VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

.PHONY: start dist install-app release

# Build and launch from source.
start:
	cargo run --release

ifeq ($(shell uname -s),Linux)

ARCH := $(shell uname -m)
PREFIX ?= $(HOME)/.local
BUNDLE := release/usage-flow-$(VERSION)-$(ARCH)-linux

# Package the app into release/.
dist:
	cargo build --release
	rm -rf "$(BUNDLE)"
	mkdir -p "$(BUNDLE)"
	cp target/release/usage-flow build/icon.png build/usage-flow.desktop "$(BUNDLE)/"

# Package and copy to ~/.local: the binary, its icon and its entry among the desktop's apps.
install-app: dist
	install -Dm755 "$(BUNDLE)/usage-flow" "$(PREFIX)/bin/usage-flow"
	install -Dm644 "$(BUNDLE)/icon.png" "$(PREFIX)/share/icons/hicolor/512x512/apps/usage-flow.png"
	mkdir -p "$(PREFIX)/share/applications"
	sed 's|^Exec=.*|Exec="$(PREFIX)/bin/usage-flow"|' "$(BUNDLE)/usage-flow.desktop" > "$(PREFIX)/share/applications/usage-flow.desktop"

# Package a distributable tarball into release/.
release: dist
	tar -C release -czf "$(BUNDLE).tar.gz" "$(notdir $(BUNDLE))"

else

APP := Usage Flow.app
BUNDLE := release/mac-arm64/$(APP)

# Package the app into release/mac-arm64.
dist:
	cargo build --release
	rm -rf "$(BUNDLE)"
	mkdir -p "$(BUNDLE)/Contents/MacOS" "$(BUNDLE)/Contents/Resources"
	cp target/release/usage-flow "$(BUNDLE)/Contents/MacOS/"
	cp build/icon.icns "$(BUNDLE)/Contents/Resources/"
	sed 's/VERSION/$(VERSION)/' build/Info.plist > "$(BUNDLE)/Contents/Info.plist"
	codesign --force --sign - "$(BUNDLE)"

# Package and copy to /Applications/Usage Flow.app.
install-app: dist
	rm -rf "/Applications/$(APP)"
	cp -R "$(BUNDLE)" /Applications/

# Package a distributable zip into release/.
release: dist
	cd release/mac-arm64 && ditto -c -k --keepParent "$(APP)" "../Usage.Flow-$(VERSION)-arm64-mac.zip"

endif
