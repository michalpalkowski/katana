ifeq ($(OS),Windows_NT)
	UNAME := Windows
else
	UNAME := $(shell uname)
endif

EXPLORER_UI_DIR ?= crates/explorer/ui/src
EXPLORER_UI_DIST ?= crates/explorer/ui/dist

SNOS_OUTPUT ?= tests/snos/snos/build/
FIXTURES_DIR ?= tests/fixtures
DB_FIXTURES_DIR ?= $(FIXTURES_DIR)/db

SNOS_DB_TAR ?= $(DB_FIXTURES_DIR)/snos.tar.gz
SNOS_DB_DIR := $(DB_FIXTURES_DIR)/snos

COMPATIBILITY_DB_TAR ?= $(DB_FIXTURES_DIR)/v1_2_2.tar.gz
COMPATIBILITY_DB_DIR ?= $(DB_FIXTURES_DIR)/v1_2_2

.DEFAULT_GOAL := usage
.SILENT: clean
.PHONY: usage help check-llvm native-deps native-deps-macos native-deps-linux native-deps-windows build-explorer clean

# Virtual targets that map to actual file outputs
.PHONY: test-artifacts snos-artifacts db-compat-artifacts

usage help:
	@echo "Usage:"
	@echo "    build-explorer:            Build the explorer."
	@echo "    test-artifacts:            Prepare tests artifacts (including test database)."
	@echo "    snos-artifacts:            Prepare SNOS tests artifacts."
	@echo "    db-compat-artifacts:       Prepare database compatibility test artifacts."
	@echo "    native-deps-macos:         Install cairo-native dependencies for macOS."
	@echo "    native-deps-linux:         Install cairo-native dependencies for Linux."
	@echo "    native-deps-windows:       Install cairo-native dependencies for Windows."
	@echo "    check-llvm:                Check if LLVM is properly configured."
	@echo "    clean:                     Clean up generated files and artifacts."
	@echo "    help:                      Show this help message."

snos-artifacts: $(SNOS_OUTPUT)
	@echo "SNOS test artifacts prepared successfully."
db-compat-artifacts: $(COMPATIBILITY_DB_DIR)
	@echo "Database compatibility test artifacts prepared successfully."
test-artifacts: $(SNOS_DB_DIR) $(SNOS_OUTPUT) $(COMPATIBILITY_DB_DIR)
	@echo "All test artifacts prepared successfully."

build-explorer:
	@which bun >/dev/null 2>&1 || { echo "Error: bun is required but not installed. Please install bun first."; exit 1; }
	@$(MAKE) $(EXPLORER_UI_DIST)

$(EXPLORER_UI_DIR):
	@echo "Initializing Explorer UI submodule..."
	@git submodule update --init --recursive --force crates/explorer/ui

$(EXPLORER_UI_DIST): $(EXPLORER_UI_DIR)
	@echo "Building Explorer..."
	@cd crates/explorer/ui && \
		bun install && \
		bun run build || { echo "Explorer build failed!"; exit 1; }
	@echo "Explorer build complete."

$(SNOS_OUTPUT): $(SNOS_DB_DIR)
	@echo "Initializing submodules..."
	@git submodule update --init --recursive
	@echo "Setting up SNOS tests..."
	@cd tests/snos/snos && \
		. ./setup-scripts/setup-cairo.sh && \
		. ./setup-scripts/setup-tests.sh || { echo "SNOS setup failed\!"; exit 1; }

$(SNOS_DB_DIR): $(SNOS_DB_TAR)
	@echo "Extracting SNOS test database..."
	@cd $(DB_FIXTURES_DIR) && \
		tar -xzf snos.tar.gz || { echo "Failed to extract SNOS test database\!"; exit 1; }
	@echo "SNOS test database extracted successfully."

$(COMPATIBILITY_DB_DIR): $(COMPATIBILITY_DB_TAR)
	@echo "Extracting backward compatibility test database..."
	@cd $(DB_FIXTURES_DIR) && \
		tar -xzf v1_2_2.tar.gz || { echo "Failed to extract backward compatibility test database\!"; exit 1; }
	@echo "Backward compatibility database extracted successfully."

check-llvm:
ifndef MLIR_SYS_190_PREFIX
	$(error Could not find a suitable LLVM 19 toolchain (mlir), please set MLIR_SYS_190_PREFIX env pointing to the LLVM 19 dir)
endif
ifndef TABLEGEN_190_PREFIX
	$(error Could not find a suitable LLVM 19 toolchain (tablegen), please set TABLEGEN_190_PREFIX env pointing to the LLVM 19 dir)
endif
	@echo "LLVM is correctly set at $(MLIR_SYS_190_PREFIX)."

native-deps:
ifeq ($(UNAME), Darwin)
native-deps: native-deps-macos
else ifeq ($(UNAME), Linux)
native-deps: native-deps-linux
else ifeq ($(UNAME), Windows)
native-deps: native-deps-windows
endif
	@echo "Run  \`source scripts/cairo-native.env.sh\` to setup the needed environment variables for cairo-native."

native-deps-macos:
	@echo "Installing LLVM dependencies for macOS..."
	-brew install llvm@19 --quiet
	@echo "macOS dependencies installed successfully."

native-deps-linux:
	@echo "Installing LLVM dependencies for Linux..."
	sudo apt-get install -y llvm-19 llvm-19-dev llvm-19-runtime clang-19 clang-tools-19 lld-19 libpolly-19-dev libmlir-19-dev mlir-19-tools
	@echo "Linux dependencies installed successfully."

native-deps-windows:
	@echo "Installing LLVM dependencies for Windows..."
	@where choco >nul 2>&1 || { echo "Error: Chocolatey is required but not installed. Please install Chocolatey first: https://chocolatey.org/install"; exit 1; }
	choco install llvm --version 19.1.7 -y
	@echo "Windows dependencies installed successfully."

clean:
	echo "Cleaning up generated files..."
	-rm -rf $(SNOS_DB_DIR) $(COMPATIBILITY_DB_DIR) $(SNOS_OUTPUT) $(EXPLORER_UI_DIST)
	echo "Clean complete."
