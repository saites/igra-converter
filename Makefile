###################################################################################################
#
# - For a production build that will run on this host, use `cargo build --release`. 
# - To run a local API server and a UI development server, use `make serve-local-release` elsewhere,
#   update the Vite configuration file with your host IP, and run `make vite`.
# - To make a production bundle of the web site and API server meant for a VCS running OpenSUSE,
#   use `make bundle` to build in container using `docker` and `compose`.
#
# See README.md for more information, configuration, and options.
#
###################################################################################################

.PHONY: vite serve-local serve-local-release
.PHONY: build bundle deploy 
.PHONY: up down node_modules npm-update
.PHONY: clean-web clean-node clean-all-web clean-volumes clean-all

PACKAGE_NAME ?= converter
RANDOM_DBF ?= data/RANDOM.DBF
export RUST_LOG ?= info,axum=debug,$(PACKAGE_NAME)=debug
export RUST_LOG_STYLE ?= always
CARGO_RUN_ARGS ?=


CONTAINER_DIR = dtarget
WEB_DIR = web
BUNDLE_FILE = bundle.tar
DEBUG_BUILD_DIR = $(CONTAINER_DIR)/debug
RELEASE_BUILD_DIR = $(CONTAINER_DIR)/release
API_SERVER_PATH = $(RELEASE_BUILD_DIR)/$(PACKAGE_NAME)
WEB_APP_DIR = $(WEB_DIR)/converter-app
NODE_MODULES_DIR ?= $(WEB_APP_DIR)/node_modules
WEB_DIST_DIR = $(WEB_APP_DIR)/dist

UID := $(shell id -u)
GID := $(shell id -g)

COMPOSE_FILES ?= docker-compose.yaml
COMPOSE_ARGS ?=

# Command definition used to run `docker compose`.
COMPOSE = SERVICE_UID=$(UID) SERVICE_GID=$(GID) \
		  docker compose $(COMPOSE_ARGS) $(addprefix -f ,$(COMPOSE_FILES))
# Command definition to run Vite commands in a container.
VITE = $(COMPOSE) run --service-ports --rm vite

API_SERVER_SRCS = $(shell find src/ -path '*.rs' -type f -print) \
				  Cargo.toml Cargo.lock
WEB_SRC = $(shell find $(WEB_DIR)/ \
 	-path $(WEB_DIST_DIR) -prune \
 	-o -path $(NODE_MODULES_DIR) -prune \
 	-o -type f -print)

# `make data/RANDOM.DBF` generates a personnel database of random data.
# `make serve-local` and `make serve-local-release` run the API server
# using that random data, creating it if needed.
$(RANDOM_DBF): $(API_SERVER_SRCS) $(wildcard data/*.txt)
	cargo run $(CARGO_RUN_ARGS) gen_db $@
serve-local-release: CARGO_RUN_ARGS += --release
serve-local-release serve-local: $(RANDOM_DBF)
	cargo run $(CARGO_RUN_ARGS) serve $(RANDOM_DBF) 8080


# Build a release version of the application in a container,
# using the same glibc as on the production server.
build: $(API_SERVER_PATH) $(WEB_DIST_DIR)
$(API_SERVER_PATH): $(API_SERVER_SRCS) Dockerfile docker-compose.yaml
	$(COMPOSE) run --rm api-server cargo build --release


# Create a tar file meant to be copied to the production server.
bundle: $(BUNDLE_FILE)
$(BUNDLE_FILE): $(WEB_DIST_DIR) $(API_SERVER_PATH) $(wildcard data/*.txt)
	tar -cf $@ \
		--verbose --show-transformed-names \
		--transform 's,^$(WEB_DIST_DIR)/,web/,' \
		--transform 's,^$(API_SERVER_PATH),new,' \
		$(WEB_DIST_DIR) \
		$(API_SERVER_PATH) \
		$(wildcard data/*.txt)


# Copy the production bundle to the production server and run a deploy script.
deploy: $(BUNDLE_FILE)
	scp $(BUNDLE_FILE) igra:bundle.tar
	ssh igra deploy.sh


# Windows versions of the above, using `cross` and `zip`.
.PHONY: build-win32 build-win64 bundle-win32 bundle-win64
WIN64_TARGET = target/x86_64-pc-windows-gnu/release/converter.exe
WIN32_TARGET = target/i686-pc-windows-gnu/release/converter.exe
WIN64_BUNDLE ?= bundle-win64.zip
WIN32_BUNDLE ?= bundle-win32.zip
define zip-win-bundle =
cp $< bundle/windows/converter.exe
cd bundle/windows && zip -r $@ *
cp bundle/windows/$@ $@
endef

build-win64: $(WIN64_TARGET)
build-win32: $(WIN32_TARGET)
target/%/release/converter.exe: $(API_SERVER_SRCS)
	cross build --target $* --release

bundle-win64: $(WIN64_BUNDLE)
bundle-win32: $(WIN32_BUNDLE)
bundle/windows: $(WEB_DIST_DIR) $(wildcard data/*.txt)
	mkdir -p bundle/windows/data
	cp -r $(WEB_DIST_DIR) bundle/windows/web
	cp $(wordlist 2,$(words $^),$^) bundle/windows/data/
$(WIN64_BUNDLE): $(WIN64_TARGET) bundle/windows
	$(zip-win-bundle)
$(WIN32_BUNDLE): $(WIN32_TARGET) bundle/windows
	$(zip-win-bundle)


# Ensure the to-be-mounted directories exist before running the container.
$(RELEASE_BUILD_DIR) $(DEBUG_BUILD_DIR):
	mkdir -p $@


# Run `docker compose up` (or `down`) with extra args/override files.
up: | $(RELEASE_BUILD_DIR) $(DEBUG_BUILD_DIR)
	$(COMPOSE) up
down:
	$(COMPOSE) down


# Generate web assets from Vue source using the `vite` service.
$(WEB_DIST_DIR): $(WEB_SRC)
	$(COMPOSE) run --rm vite npm run build-only


# Run the local webserver in a container: `make vite`.
# The server configuration needs to point to the host running the Rust server.
vite: | $(NODE_MODULES_DIR)
	$(VITE) npm run dev


# Install dependencies using `make node_modules` without setting `module`
# Install a new npm package using `module="some-module" make node_modules`
module ?=
node_modules: $(NODE_MODULES_DIR)
$(NODE_MODULES_DIR):
	$(VITE) npm install $(module)

npm-update:
	$(VITE) npm update --save

clean-web:
	rm -rf $(WEB_DIST_DIR)

clean-node:
	rm -rf $(NODE_MODULES)

clean-all-web: clean-web clean-node

clean-volumes:
	$(COMPOSE) down -v

clean-all: clean-all-web clean-volumes

