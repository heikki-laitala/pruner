.PHONY: build release install run test test-quick test-unit test-integration bench lint format check clean index dev-purge dev-test-install

build:
	cargo build

release:
	cargo build --release

install: release
	cargo install --path .

test:
	cargo test --bin pruner --test integration

test-unit:
	cargo test --lib

test-integration:
	cargo test --test integration

test-quick:
	cargo test -- --quiet

bench:
	cargo build --release
	cargo test --test bench -- --nocapture

lint:
	cargo clippy -- -D warnings

format:
	cargo fmt

check: lint test

clean:
	cargo clean
	rm -rf .pruner/

run:
	cargo run -- $(ARGS)

index:
	cargo run -- index . -v

# ============================================================================
# Dev targets — these touch global config files outside this repo!
# ============================================================================

# Remove all pruner artifacts from global Claude/Copilot config and PATH.
# Use this to test a fresh install experience.
dev-purge:
	@echo "⚠️  WARNING: This removes pruner from global config files."
	@echo "   - ~/.claude/skills/pruner/"
	@echo "   - ~/.claude/hooks/pruner-context.sh"
	@echo "   - ~/.claude/settings.json (pruner hooks)"
	@echo "   - ~/.copilot/skills/pruner/"
	@echo "   - ~/.copilot/copilot-instructions.md (pruner section)"
	@echo "   - ~/.local/bin/pruner"
	@echo "   - /opt/homebrew/bin/pruner (if present)"
	@echo "   - ~/.cargo/bin/pruner (if present)"
	@echo ""
	@read -p "Continue? [y/N] " confirm && [ "$$confirm" = "y" ] || (echo "Aborted."; exit 1)
	rm -rf ~/.claude/skills/pruner
	rm -f ~/.claude/hooks/pruner-context.sh
	@if [ -f ~/.claude/settings.json ]; then \
		python3 -c "import json,sys; s=json.load(open(sys.argv[1])); s.pop('hooks',None); json.dump(s,open(sys.argv[1],'w'),indent=2)" ~/.claude/settings.json && \
		echo "Cleaned hooks from ~/.claude/settings.json"; \
	fi
	rm -rf ~/.copilot/skills/pruner
	@if [ -f ~/.copilot/copilot-instructions.md ]; then \
		python3 -c "import sys; t=open(sys.argv[1]).read(); i=t.find('## Pruner'); open(sys.argv[1],'w').write(t[:i].rstrip()+'\n' if i>=0 else t)" ~/.copilot/copilot-instructions.md && \
		echo "Cleaned pruner section from ~/.copilot/copilot-instructions.md"; \
	fi
	rm -f ~/.local/bin/pruner
	rm -f ~/.cargo/bin/pruner
	@if [ -f /opt/homebrew/bin/pruner ]; then \
		echo "Found /opt/homebrew/bin/pruner — removing (may need sudo)"; \
		rm -f /opt/homebrew/bin/pruner || sudo rm -f /opt/homebrew/bin/pruner; \
	fi
	@echo ""
	@echo "Done. Run 'make dev-test-install' to test a fresh install."

# Build, install to ~/.local/bin, and run init --global --hook.
dev-test-install: release
	@echo "⚠️  WARNING: This installs pruner globally and modifies ~/.claude/ config."
	@echo ""
	@read -p "Continue? [y/N] " confirm && [ "$$confirm" = "y" ] || (echo "Aborted."; exit 1)
	ln -sf $(CURDIR)/target/release/pruner /opt/homebrew/bin/pruner
	@echo "Installed symlink -> /opt/homebrew/bin/pruner -> $(CURDIR)/target/release/pruner"
	pruner --version
	@echo ""
	pruner init --global --hook
	@echo ""
	@echo "Done. Open Claude Code in any repo to test the hook."
