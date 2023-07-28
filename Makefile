
.PHONY: up down prod \
	clean-volumes clean-web \
	vite node_modules npm-update \
	serve-local serve-local-release

UID := $(shell id -u)
GID := $(shell id -g)
COMPOSE_FILES ?= docker-compose.yaml
COMPOSE_ARGS ?=
COMPOSE = SERVICE_UID=$(UID) SERVICE_GID=$(GID) docker compose $(COMPOSE_ARGS) $(addprefix -f ,$(COMPOSE_FILES))
BUILD_DIRS = dtarget/debug dtarget/release

CARGO_RUN_ARGS ?= 
serve-local-release: CARGO_RUN_ARGS += --release

RANDOM_DBF ?= ./data/RANDOM.DBF
# RANDOM_DBF ?= ./data/tmp/PERSONEL.DBF
$(RANDOM_DBF):
	cargo run $(CARGO_RUN_ARGS) gen_db ../shared/PERSONEL.DBF $@
serve-local-release serve-local: $(RANDOM_DBF) 
	cargo run $(CARGO_RUN_ARGS) serve $(RANDOM_DBF) 8080

build:
	$(COMPOSE) run --rm api-server cargo build --release

up: $(BUILD_DIRS)
	$(COMPOSE) up

down:
	$(COMPOSE) down

prod: web/converter-app/dist

$(BUILD_DIRS):
	mkdir -p $@

# Generate web assets from Vue source.
# This uses the `vite` config from the dev compose file.
WEB_SRC = $(shell find web/ \
 	-path web/dist -prune \
 	-o -path web/node_modules -prune \
 	-o -type f -print)
web/converter-app/dist: $(WEB_SRC)
	$(COMPOSE) run --rm vite npm run build-only


VITE = $(COMPOSE) run --service-ports --rm vite

module ?=
node_modules:
	$(VITE) npm install $(module)

vite: 
	$(VITE) npm run dev

npm-update:
	$(VITE) npm update --save

clean-web:
	rm -rf web/dist

clean-volumes:
	$(COMPOSE) down -v

