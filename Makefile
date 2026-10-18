ARGS ?=

.PHONY: run build test check fmt icons app install clean

ifeq ($(shell uname -s),Darwin)
run:
	PROFILE=debug ./scripts/bundle-macos.sh
	./dist/give.app/Contents/MacOS/give $(ARGS)
else
run:
	cargo run -- $(ARGS)
endif

build:
	cargo build --release

test:
	cargo test

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt

icons:
	./scripts/build-icons.sh

app:
	./scripts/bundle-macos.sh

# Optimized release bundle, replacing /Applications/give.app (quit a running copy first).
install: app
	rm -rf /Applications/give.app
	ditto dist/give.app /Applications/give.app
	@echo 'Installed /Applications/give.app'

clean:
	cargo clean
