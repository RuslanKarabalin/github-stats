BINARY_NAME=github-stats
BUILD_DIR=./target/release
CARGO=cargo
GH_USER ?=
USER_FLAG=$(if $(GH_USER),--user $(GH_USER))

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
	@test -n "$(GH_USER)" || { echo "Usage: make user GH_USER=octocat"; exit 1; }
	@echo "Running $(BINARY_NAME) for $(GH_USER)..."
	$(BUILD_DIR)/$(BINARY_NAME) $(USER_FLAG)

full: build
	@echo "Running $(BINARY_NAME) with full scan..."
	$(BUILD_DIR)/$(BINARY_NAME) $(USER_FLAG) --full

web: build
	@echo "Running $(BINARY_NAME) with web output..."
	$(BUILD_DIR)/$(BINARY_NAME) $(USER_FLAG) --web

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
	@echo "  user       - Run for a user (make user GH_USER=octocat)"
	@echo "  full       - Run with full scan (GH_USER is optional)"
	@echo "  web        - Serve statistics on localhost (GH_USER is optional)"
	@echo "  fmt        - Format code"
	@echo "  lint       - Run clippy"
	@echo "  help       - Show this help message"
