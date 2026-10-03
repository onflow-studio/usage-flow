APP := Usage Flow.app
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BUNDLE := release/mac-arm64/$(APP)

.PHONY: start dist install-app release

# Build and launch from source.
start:
	cargo run --release

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
