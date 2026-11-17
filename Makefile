BINARY_NAME=github-stats
BUILD_DIR=./target/release
CARGO=cargo

.PHONY: all build clean test deps install uninstall run user full web fmt lint help

all: deps build

build:
	@echo "Building $(BINARY_NAME)..."
	$(CARGO) build --release
	@echo "Build complete: $(BUILD_DIR)/$(BINARY_NAME)"

deps:
	@echo "Fetching dependencies..."
	$(CARGO) fetch
	@echo "Dependencies fetched"

clean:
	@echo "Cleaning..."
	$(CARGO) clean
	@echo "Clean complete"

test:
	@echo "Running tests..."
	$(CARGO) test

install: build
	@echo "Installing $(BINARY_NAME) to /usr/local/bin..."
	sudo cp $(BUILD_DIR)/$(BINARY_NAME) /usr/local/bin/
	@echo "Installation complete"

uninstall:
	@echo "Uninstalling $(BINARY_NAME)..."
	sudo rm -f /usr/local/bin/$(BINARY_NAME)
	@echo "Uninstall complete"

run: build
	@echo "Running $(BINARY_NAME)..."
	$(BUILD_DIR)/$(BINARY_NAME)

user: build
	@echo "Running $(BINARY_NAME) with user flag..."
	$(BUILD_DIR)/$(BINARY_NAME) --user $(USER)

full: build
	@echo "Running $(BINARY_NAME) with full scan..."
	$(BUILD_DIR)/$(BINARY_NAME) --user $(USER) --full

web: build
	@echo "Running $(BINARY_NAME) with web output..."
	$(BUILD_DIR)/$(BINARY_NAME) --user $(USER) --web

fmt:
	@echo "Formatting code..."
	$(CARGO) fmt
	@echo "Format complete"

lint:
	@echo "Running linter..."
	$(CARGO) clippy --all-targets -- -D warnings

help:
	@echo "Available targets:"
	@echo "  all        - Fetch dependencies and build (default)"
	@echo "  build      - Build the release binary"
	@echo "  deps       - Fetch dependencies"
	@echo "  clean      - Remove build artifacts"
	@echo "  test       - Run tests"
	@echo "  install    - Install binary to /usr/local/bin"
	@echo "  uninstall  - Remove binary from /usr/local/bin"
	@echo "  run        - Build and run the application"
	@echo "  user       - Run for a user (make user USER=octocat)"
	@echo "  full       - Run with full scan (make full USER=octocat)"
	@echo "  web        - Serve statistics on localhost (make web USER=octocat)"
	@echo "  fmt        - Format code"
	@echo "  lint       - Run clippy"
	@echo "  help       - Show this help message"
