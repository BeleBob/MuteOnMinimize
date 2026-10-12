# Builds and installs both parts of MuteOnMinimize.
#
#   make && sudo make install          system-wide, into /usr
#   make install DESTDIR=$pkgdir       for packaging
#   ./install.sh                       per-user, for development
#
# Paths can be overridden; the unit files get the real binary path.

PREFIX ?= /usr
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
SYSTEMDUSERUNITDIR ?= $(PREFIX)/lib/systemd/user
DBUSSERVICEDIR ?= $(DATADIR)/dbus-1/services
KWINSCRIPTDIR ?= $(DATADIR)/kwin/scripts/muteonminimize

CARGO ?= cargo
CARGOFLAGS ?= --locked
BINARY := service/target/release/muteonminimized

# Translations ship inside the KWin script package, where its settings page
# and the service look for them.
LANGS := $(basename $(notdir $(wildcard po/*.po)))
MO = build/locale/$(1)/LC_MESSAGES/muteonminimize.mo

UNIT := muteonminimized.service
DBUS_SERVICE := io.github.belebob.MuteOnMinimize.service

.PHONY: all build check check-script install uninstall clean

all: build

build: $(foreach lang,$(LANGS),$(call MO,$(lang)))
	$(CARGO) build --release $(CARGOFLAGS) --manifest-path service/Cargo.toml

# Needs node; not part of `check` so packaging does not depend on it.
check-script:
	node kwin-script/tests/main.test.js

build/locale/%/LC_MESSAGES/muteonminimize.mo: po/%.po
	@mkdir -p $(@D)
	msgfmt --check -o $@ $<

check:
	$(CARGO) test --release $(CARGOFLAGS) --manifest-path service/Cargo.toml

install:
	install -Dm755 $(BINARY) $(DESTDIR)$(BINDIR)/muteonminimized
	install -d $(DESTDIR)$(SYSTEMDUSERUNITDIR) $(DESTDIR)$(DBUSSERVICEDIR)
	sed 's|/usr/bin/muteonminimized|$(BINDIR)/muteonminimized|' service/data/$(UNIT) \
		> $(DESTDIR)$(SYSTEMDUSERUNITDIR)/$(UNIT)
	sed 's|/usr/bin/muteonminimized|$(BINDIR)/muteonminimized|' service/data/$(DBUS_SERVICE) \
		> $(DESTDIR)$(DBUSSERVICEDIR)/$(DBUS_SERVICE)
	chmod 644 $(DESTDIR)$(SYSTEMDUSERUNITDIR)/$(UNIT) $(DESTDIR)$(DBUSSERVICEDIR)/$(DBUS_SERVICE)
	install -Dm644 kwin-script/metadata.json $(DESTDIR)$(KWINSCRIPTDIR)/metadata.json
	install -Dm644 kwin-script/contents/code/main.js $(DESTDIR)$(KWINSCRIPTDIR)/contents/code/main.js
	install -Dm644 kwin-script/contents/config/main.xml $(DESTDIR)$(KWINSCRIPTDIR)/contents/config/main.xml
	install -Dm644 kwin-script/contents/ui/config.ui $(DESTDIR)$(KWINSCRIPTDIR)/contents/ui/config.ui
	for lang in $(LANGS); do \
		install -Dm644 build/locale/$$lang/LC_MESSAGES/muteonminimize.mo \
			$(DESTDIR)$(KWINSCRIPTDIR)/contents/locale/$$lang/LC_MESSAGES/muteonminimize.mo; \
	done

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/muteonminimized
	rm -f $(DESTDIR)$(SYSTEMDUSERUNITDIR)/$(UNIT)
	rm -f $(DESTDIR)$(DBUSSERVICEDIR)/$(DBUS_SERVICE)
	rm -rf $(DESTDIR)$(KWINSCRIPTDIR)

clean:
	$(CARGO) clean --manifest-path service/Cargo.toml
	rm -rf build
