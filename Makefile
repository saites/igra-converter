
.PHONY: up down prod \
	clean-volumes clean-web \
	vite

UID := $(shell id -u)
GID := $(shell id -g)
COMPOSE_FILES ?= docker-compose.yaml
COMPOSE_ARGS ?=
COMPOSE = SERVICE_UID=$(UID) SERVICE_GID=$(GID) docker compose $(COMPOSE_ARGS) $(addprefix -f ,$(COMPOSE_FILES))
BUILD_DIRS = dtarget/debug dtarget/release

up: $(BUILD_DIRS)
	$(COMPOSE) up

down:
	$(COMPOSE) down

prod: web/dist

$(BUILD_DIRS):
	mkdir -p $@

# Generate web assets from Vue source.
# This uses the `vite` config from the dev compose file.
WEB_SRC = $(shell find web/ \
 	-path web/dist -prune \
 	-o -path web/node_modules -prune \
 	-o -type f -print)
web/dist: $(WEB_SRC)
	$(COMPOSE) run --rm vite npm run build-only
	rm -rf web/dist


VITE = $(COMPOSE) run --service-ports --rm vite

vite:
	# $(VITE) npm init vue@latest
	# $(VITE) npm install
	$(VITE) npm run dev
	# $(VITE) npm install highlight.js

tailwind:
	$(VITE) npm install -D tailwindcss postcss autoprefixer
	$(VITE) npx tailwindcss init 


clean-web:
	rm -rf web/dist

clean-volumes:
	$(COMPOSE) down -v

