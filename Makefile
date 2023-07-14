
.PHONY: up down prod \
	clean-volumes clean-web \
	vite node_modules \
	serve-local

UID := $(shell id -u)
GID := $(shell id -g)
COMPOSE_FILES ?= docker-compose.yaml
COMPOSE_ARGS ?=
COMPOSE = SERVICE_UID=$(UID) SERVICE_GID=$(GID) docker compose $(COMPOSE_ARGS) $(addprefix -f ,$(COMPOSE_FILES))
BUILD_DIRS = dtarget/debug dtarget/release

CARGO_RUN_ARGS ?= --release

data/random-data.dbf:
	cargo run $(CARGO_RUN_ARGS) gen_db ./new-random.dbf $@
serve-local: data/random-data.dbf
	cargo run $(CARGO_RUN_ARGS) serve ./data/random-data.dbf 8080


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

tailwind:
	$(VITE) npm install -D tailwindcss postcss autoprefixer
	$(VITE) npx tailwindcss init 


clean-web:
	rm -rf web/dist

clean-volumes:
	$(COMPOSE) down -v

