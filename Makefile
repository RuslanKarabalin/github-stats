BINARY_NAME=github-stats
BUILD_DIR=./bin
MAIN_PATH=./cmd/github-stats/main.go

GOCMD=go
GOBUILD=$(GOCMD) build
GOCLEAN=$(GOCMD) clean
GOTEST=$(GOCMD) test
GOGET=$(GOCMD) get
GOMOD=$(GOCMD) mod

LDFLAGS=-ldflags "-s -w"

.PHONY: all build clean test deps install run help

all: deps build

build:
	@echo "Building $(BINARY_NAME)..."
	@mkdir -p $(BUILD_DIR)
	$(GOBUILD) $(LDFLAGS) -o $(BUILD_DIR)/$(BINARY_NAME) $(MAIN_PATH)
	@echo "Build complete: $(BUILD_DIR)/$(BINARY_NAME)"

deps:
	@echo "Installing dependencies..."
	$(GOMOD) download
	$(GOMOD) tidy
	@echo "Dependencies installed"

clean:
	@echo "Cleaning..."
	$(GOCLEAN)
	rm -rf $(BUILD_DIR)
	@echo "Clean complete"

test:
	@echo "Running tests..."
	$(GOTEST) -v ./...

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

run-user:
	@echo "Running $(BINARY_NAME) with user flag..."
	$(BUILD_DIR)/$(BINARY_NAME) --user $(USER)

run-full:
	@echo "Running $(BINARY_NAME) with full scan..."
	$(BUILD_DIR)/$(BINARY_NAME) --user $(USER) --full

fmt:
	@echo "Formatting code..."
	$(GOCMD) fmt ./...
	@echo "Format complete"

lint:
	@echo "Running linter..."
	golangci-lint run ./...

help:
	@echo "Available targets:"
	@echo "  all        - Install dependencies and build (default)"
	@echo "  build      - Build the binary"
	@echo "  deps       - Install dependencies"
	@echo "  clean      - Remove build artifacts"
	@echo "  test       - Run tests"
	@echo "  install    - Install binary to /usr/local/bin"
	@echo "  uninstall  - Remove binary from /usr/local/bin"
	@echo "  run        - Build and run the application"
	@echo "  run-user   - Run with USER env var (make run-user USER=octocat)"
	@echo "  run-full   - Run with full scan (make run-full USER=octocat)"
	@echo "  fmt        - Format code"
	@echo "  lint       - Run linter"
	@echo "  help       - Show this help message"
