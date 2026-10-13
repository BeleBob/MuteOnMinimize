# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Проект

MuteOnMinimize — дополнение для KDE Plasma 6. Выбранные пользователем приложения
заглушаются, пока свёрнуты все их окна, и снова звучат, когда окно развернули.

- Репозиторий: github.com/BeleBob/MuteOnMinimize (публичный), лицензия GPL-3.0-or-later.
- Состояние: выпущена 0.1.0 (релиз на GitHub с готовым пакетом); изменения после неё — в CHANGELOG.md, «Unreleased».
  Пакеты AUR готовы, но не опубликованы: регистрация в AUR временно закрыта, аккаунта у владельца нет.
- Документация: README.md (ru), README.en.md, docs/ARCHITECTURE.md, CONTRIBUTING.md (en); CI — `.github/workflows/ci.yml`.

## Владелец и правила

- Общаться с владельцем на русском. Комментарии в коде и сообщения коммитов — на английском.
- Перед крупными изменениями — план, ждать подтверждения.
- Работа в ветках, изменения через PR; сливает владелец. Небольшие логичные коммиты.
- Никаких секретов в репозитории.
- Окружение владельца:
  - CachyOS, Plasma 6.7.5 на Wayland, KWin 6.7.5;
  - Qt 6.11, KDE Frameworks 6.30;
  - звук на PipeWire 1.6.8 с pipewire-pulse и WirePlumber.
- Инструменты:
  - Есть: `kpackagetool6`, `qmllint`, `qmlformat`, `qdbus6`, `kwriteconfig6`, `busctl`, `pactl`, `wpctl`, `pw-cli`, `pw-dump`, `cmake`, `extra-cmake-modules`.
  - Rust 1.92 через rustup (`cargo`), `clang`, заголовки `pipewire-0.3` и `spa-0.2` — всё для крейта `pipewire`.
  - `plasma-sdk` (`plasmoidviewer`) не установлен. Ставить пакеты только с согласия владельца: нужен sudo.

## Устройство

Выбрано с владельцем: **скрипт KWin + пользовательский сервис на Rust**, распространение через AUR.
Глушим, когда свёрнуты все окна приложения, а с `MuteInBackground` (по умолчанию включено) — и когда
ни одно его окно не впереди на своём экране (Alt+Tab). Окно на другом мониторе звук не трогает.

- **Скрипт KWin (JS)** — `kwin-script/`.
  - Свёрнутость окон на Wayland надёжно видит только KWin: `workspace.windowList()`,
    `windowAdded`/`windowRemoved`, у окна `minimized`/`minimizedChanged`, `pid`, `resourceClass`, `desktopFileName`.
  - Группирует окна по `pid`. PID глушится, если среди его окон есть окно выбранного приложения
    и ни одно окно с этим PID не «впереди» (окна PWA Chromium делят PID с браузером, но у них другой класс).
    «Впереди» на экране — последнее активированное (`windowActivated`) несвёрнутое окно приложения
    на текущем рабочем столе и в текущей комнате; до первых активаций — порядок `stackingOrder`.
    Без `MuteInBackground` — только «все окна свёрнуты».
  - При каждом изменении шлёт сервису через `callDBus` **полное** состояние — список PID для глушения.
    Так перезапуск KWin или сервиса ничего не ломает: следующий вызов приводит всё в порядок.
  - Список приложений (`resourceClass` или desktop-файл) — на странице настроек скрипта
    в «Параметрах системы» → «Диспетчер окон» → «Сценарии KWin», в меню окна (`registerUserActionsMenu`,
    Alt+F3) и сочетанием Meta+Ctrl+M (`registerShortcut`) для активного окна. Скрипты KWin не умеют
    писать свои настройки, поэтому список сохраняет сервис (`SetApplications` → `kwriteconfig6`).
  - Подтверждение — экранная подсказка Plasma: `org.kde.plasmashell /org/kde/osdService showText(icon, text)`.
  - Переводы: в JS-скриптах KWin нет `i18n`, поэтому строки берутся у сервиса (`Translations`, JSON),
    а сочетание клавиш регистрируется после их получения — его имя видно в настройках.
- **Сервис (Rust)** — `service/`. systemd `--user` и D-Bus-активация: запускается первым вызовом из скрипта.
  - D-Bus: `zbus`. PipeWire: крейт `pipewire` (нативный API, не pulse).
  - Имя на шине занимается до подключения к PipeWire: вызовы до этого копятся в канале.
  - Методы `io.github.belebob.MuteOnMinimize1`: `SetMutedPids(av)`, `SetApplications(s)`, `Translations() → s`.
  - Переводы читает из `.mo` в пакете скрипта (`contents/locale/<lang>/LC_MESSAGES/muteonminimize.mo`)
    по `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`; свой разбор `.mo`, без зависимостей.
  - Следит за узлами `Stream/Output/Audio`. Поток глушится, если его `application.process.id`
    равен PID из списка или это потомок такого процесса (дерево из `/proc`).
  - Flatpak: у потока `application.process.id` — PID внутри песочницы; по `pipewire.access.portal.instance_id`
    из `$XDG_RUNTIME_DIR/.flatpak/<id>/bwrapinfo.json` берётся `pid-namespace`, и в `/proc` ищется процесс
    этого пространства с таким последним числом в `NSpid` (`sandbox.rs`).
  - Фатальна только ошибка ядра PipeWire с `-EPIPE` (потеря связи); остальные (например, «unknown resource»
    после удаления потока) — гонки, их только пишем в лог.
  - Глушит через `softMute` в `Props` узла, а не через `mute` (почему — см. ниже).
    Следит за `Props` заглушённых узлов и ставит `softMute` заново, если его сбросили.
  - Глушение с затуханием за 1 с: таймер цикла PipeWire раз в 20 мс ставит `softVolumes` =
    громкость пользователя × уровень³, на нуле — `softMute`. Возврат звука (и посреди затухания) — сразу:
    восстановить `softMute` и `softVolumes` пользователя (так решил владелец). Новые потоки при заглушённом
    приложении и отпускание по SIGTERM — тоже сразу.
  - SIGTERM/SIGINT блокируются в `main()` до создания потоков (zbus), иначе signalfd PipeWire их не получит.
  - Снимает `softMute` только у узлов, которые заглушил сам; при остановке (SIGTERM) снимает со всех.

## Проверено (октябрь 2026, PipeWire 1.6.8, WirePlumber 0.5.17, KWin 6.7.5)

- **PID окна и потока.** У узла-потока `application.process.id` — PID приложения, а `pipewire.sec.pid`
  у pulse-клиентов указывает на сам pipewire-pulse, его не использовать.
  - Firefox: поток идёт от главного процесса, PID совпадает с PID окна.
  - Игра под Wine/Proton: PID окна совпадает с PID потока; `application.process.binary` = `wine-preloader`,
    поэтому сопоставлять по имени бинарника нельзя.
  - Electron (Discord): поток идёт из дочернего `--utility-sub-type=audio.mojom.AudioService`,
    его родитель — процесс окна. Поэтому нужен поиск по дереву процессов.
  - Flatpak: KWin видит у окна PID на хосте, а поток — PID внутри песочницы (обычно 2), плюс
    `pipewire.access.portal.instance_id`; `pipewire.sec.pid` клиента — снова pipewire-pulse.
    Проверено на окне и звуке одного `gst-launch-1.0` в песочнице Sober.
  - Chromium не проверялся (не установлен): у него, как у Electron, звук из дочернего аудиосервиса.
- **WirePlumber запоминает `mute`.** `node.stream.restore-props` сохраняет `mute`, `volume`, `channelVolumes`
  и `channelMap` в `~/.local/state/wireplumber/stream-properties` по ключу `application.name`
  (точнее, по первому из `application.id`, `application.name`, `media.name`, `node.name`).
  Если заглушить поток обычным `mute` и приложение закроют свёрнутым, при следующем запуске
  оно стартует заглушённым. Проверено на тестовом потоке.
- **`softMute` не запоминается.** `pw-cli set-param <node> Props '{ softMute: true }'` глушит звук,
  WirePlumber его не сохраняет, в `pactl` поток остаётся `Mute: no`, `mute` пользователя не меняется.
  Минус: изменение громкости или mute пользователем сбрасывает `softMute`, его нужно ставить заново.
  Ещё минус: в микшере не видно, что поток заглушён нами.

- **`softVolumes`** заменяет громкость пользователя, а не умножается на неё (обычно оно равно
  `channelVolumes × volume`). WirePlumber его не сохраняет; изменение громкости пользователем его сбрасывает.
  Свойства `volumeRampTime` и др. у узлов-потоков не работают — громкость меняется мгновенно.
  30 быстрых `set_param` дают ровно 30 событий изменения: эхо не склеиваются.
- **Сигналы.** Если поток без блокировки SIGTERM создан раньше `add_signal_local`, процесс умирает
  от сигнала (код 143) и не возвращает звук. Проверять: `grep SigBlk /proc/<pid>/task/*/status`.
- **Win+D.** По умолчанию это «Взглянуть на рабочий стол»: окна прячутся, но `minimized` остаётся `false`.
  Окну приходит `hiddenByShowDesktopChanged`, но само состояние из скрипта не прочитать
  (свойства нет, `hidden` — `false`, D-Bus-свойство `showingDesktop` в тесте тоже `false`).
  У владельца Meta+D переназначен на сценарий KWin `minimizeall` («Сворачивание всех окон»).
- **Переводы страницы настроек.** `kcm_kwin4_genericscripted` берёт домен из `X-KWin-Config-TranslationDomain`
  в metadata.json и каталоги из `contents/locale` пакета. Перевод `.ui` — по msgid без контекста.
- **Скриншоты страницы настроек:** `LANGUAGE=<lang> kcmshell6 kwin/effects/configs/kcm_kwin4_genericscripted
  --args "muteonminimize KWin/Script"`, окно увеличить скриптом KWin (`frameGeometry`), снять `spectacle -b -n -a -o <файл>`.
  Длинный текст у `QCheckBox` не переносится и растягивает страницу — пояснения выносить в `QLabel` с `wordWrap`.
- **Живые тесты с фокусом** ненадёжны, пока владелец работает за ПК: фокус уходит в его окна.
  Логику окон проверять на макете: `make check-script`.

## Требования

- Окна и потоки:
  - Если у приложения несколько окон, глушить только когда свёрнуты (или не впереди) все.
  - Alt+Tab на другое окно того же экрана глушит; уход в окно на другом мониторе — нет.
  - Звук может идти из дочернего процесса (Electron, Chromium), поэтому сопоставлять по дереву процессов.
  - Потоки, появившиеся, пока окно свёрнуто, тоже глушить.
- Возврат звука:
  - Включать звук только потокам, которые заглушили мы. То, что пользователь выключил сам, не трогать.
  - После закрытия приложения, перезапуска KWin или Plasma и выхода из сеанса потоки не должны остаться заглушёнными.
  - Наше глушение не должно попасть в сохранённое состояние WirePlumber (поэтому `softMute`).
- Настройки:
  - Список приложений — по `resourceClass` или desktop-файлу; выбирать и из меню окна, и сочетанием клавиш.
  - Настраивать на странице скрипта KWin в «Параметрах системы».
- Интерфейс на языке системы; переводы в `po/` (сейчас ru, uk, de).

## Команды-справка

- Сборка и тесты сервиса: `make build`, `make check` (или `cargo build`/`cargo test`/`cargo clippy` в `service/`).
- Тест логики скрипта KWin на макете (нужен node): `make check-script`.
- Переводы: после правки текстов в `main.js` (строки в `tr("…")`) или `config.ui` — `po/update.sh`,
  затем перевести новые строки в `po/*.po`; проверка: `msgfmt --check --statistics -o /dev/null po/ru.po`.
- Установка для разработки (без root, в `~/.local` и `~/.config`): `./install.sh`; удаление: `./install.sh --uninstall`.
  Повторный запуск обновляет установленное и перезапускает сервис.
- Системная установка и упаковка: `make install DESTDIR=… PREFIX=/usr`; пути в unit-файлах подставляет `make`.
- Пакет: `packaging/aur/muteonminimize-git/`. Проверить сборку: `makepkg -f` в копии каталога;
  после правки PKGBUILD обновить `.SRCINFO`: `makepkg --printsrcinfo > .SRCINFO`.
- Выпуск версии — по CONTRIBUTING.md («Releases»). Важно: у владельца в makepkg.conf `-march=native` и
  `RUSTFLAGS=-C target-cpu=native`; пакет для релиза собирать с `makepkg --config <копия /etc/makepkg.conf
  с флагами Arch по умолчанию>` и проверять, что в бинарнике нет AVX (`objdump -d … | grep ymm`).
- Сервис:
  - логи: `journalctl --user -u muteonminimized -f`;
  - вызвать вручную: `busctl --user call io.github.belebob.MuteOnMinimize /io/github/belebob/MuteOnMinimize io.github.belebob.MuteOnMinimize1 SetMutedPids av 1 i <pid>`.
    `busctl` сразу отключается от шины, и сервис тут же снимает заглушение — для проверок вызывать из временного скрипта KWin.
- Скрипт KWin:
  - установить или обновить: `kpackagetool6 --type KWin/Script -i <папка>` или `-u <папка>`;
  - включить: `kwriteconfig6 --file kwinrc --group Plugins --key <id>Enabled true`, затем `qdbus6 org.kde.KWin /KWin reconfigure`.
- Логи скрипта KWin (`print`/`console.log`): `journalctl --user -f | grep -i kwin`.
- Настройки скрипта: `kwinrc`, группа `[Script-muteonminimize]`, ключи `Applications`, `MuteInBackground`.
- Сочетания клавиш: `busctl --user call org.kde.kglobalaccel /kglobalaccel org.kde.KGlobalAccel shortcutKeys as 4 kwin <действие> "" ""`;
  вызвать действие: `qdbus6 org.kde.kglobalaccel /component/kwin org.kde.kglobalaccel.Component.invokeShortcut <действие>`.
- Узлы и их свойства: `pw-dump`; параметры узла: `pw-cli enum-params <node> Props`.
- Сохранённое состояние потоков WirePlumber: `~/.local/state/wireplumber/stream-properties`.
