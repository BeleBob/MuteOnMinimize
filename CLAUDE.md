# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Проект

MuteOnMinimize — дополнение для KDE Plasma 6. Выбранные пользователем приложения
заглушаются, пока свёрнуты все их окна, и снова звучат, когда окно развернули.

- Репозиторий: github.com/BeleBob/MuteOnMinimize (публичный), лицензия GPL-3.0-or-later.
- Состояние: устройство выбрано (см. «Устройство»), кода пока нет.

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
Глушим только когда свёрнуты все окна приложения (без учёта рабочих столов и без задержки).

- **Скрипт KWin (JS)** — `kwin-script/`.
  - Свёрнутость окон на Wayland надёжно видит только KWin: `workspace.windowList()`,
    `windowAdded`/`windowRemoved`, у окна `minimized`/`minimizedChanged`, `pid`, `resourceClass`, `desktopFileName`.
  - Группирует окна по `pid`. PID глушится, если среди его окон есть окно выбранного приложения
    и все окна с этим PID свёрнуты (окна PWA Chromium делят PID с браузером, но у них другой класс).
  - При каждом изменении шлёт сервису через `callDBus` **полное** состояние — список PID для глушения.
    Так перезапуск KWin или сервиса ничего не ломает: следующий вызов приводит всё в порядок.
  - Список приложений (`resourceClass` или desktop-файл) — на странице настроек скрипта
    в «Параметрах системы» → «Скрипты KWin».
- **Сервис (Rust)** — `service/`. systemd `--user` и D-Bus-активация: запускается первым вызовом из скрипта.
  - D-Bus: `zbus`. PipeWire: крейт `pipewire` (нативный API, не pulse).
  - Следит за узлами `Stream/Output/Audio`. Поток глушится, если его `application.process.id`
    равен PID из списка или это потомок такого процесса (дерево из `/proc`).
  - Глушит через `softMute` в `Props` узла, а не через `mute` (почему — см. ниже).
    Следит за `Props` заглушённых узлов и ставит `softMute` заново, если его сбросили.
  - Снимает `softMute` только у узлов, которые заглушил сам; при остановке (SIGTERM) снимает со всех.

## Проверено (октябрь 2026, PipeWire 1.6.8, WirePlumber 0.5.17, KWin 6.7.5)

- **PID окна и потока.** У узла-потока `application.process.id` — PID приложения, а `pipewire.sec.pid`
  у pulse-клиентов указывает на сам pipewire-pulse, его не использовать.
  - Firefox: поток идёт от главного процесса, PID совпадает с PID окна.
  - Игра под Wine/Proton: PID окна совпадает с PID потока; `application.process.binary` = `wine-preloader`,
    поэтому сопоставлять по имени бинарника нельзя.
  - Electron (Discord): поток идёт из дочернего `--utility-sub-type=audio.mojom.AudioService`,
    его родитель — процесс окна. Поэтому нужен поиск по дереву процессов.
  - Не проверено: Chromium, Flatpak (у Flatpak свой PID-namespace, `application.process.id` может не совпасть).
- **WirePlumber запоминает `mute`.** `node.stream.restore-props` сохраняет `mute`, `volume`, `channelVolumes`
  и `channelMap` в `~/.local/state/wireplumber/stream-properties` по ключу `application.name`
  (точнее, по первому из `application.id`, `application.name`, `media.name`, `node.name`).
  Если заглушить поток обычным `mute` и приложение закроют свёрнутым, при следующем запуске
  оно стартует заглушённым. Проверено на тестовом потоке.
- **`softMute` не запоминается.** `pw-cli set-param <node> Props '{ softMute: true }'` глушит звук,
  WirePlumber его не сохраняет, в `pactl` поток остаётся `Mute: no`, `mute` пользователя не меняется.
  Минус: изменение громкости или mute пользователем сбрасывает `softMute`, его нужно ставить заново.
  Ещё минус: в микшере не видно, что поток заглушён нами.

## Требования

- Окна и потоки:
  - Если у приложения несколько окон, глушить только когда свёрнуты все.
  - Звук может идти из дочернего процесса (Electron, Chromium), поэтому сопоставлять по дереву процессов.
  - Потоки, появившиеся, пока окно свёрнуто, тоже глушить.
- Возврат звука:
  - Включать звук только потокам, которые заглушили мы. То, что пользователь выключил сам, не трогать.
  - После закрытия приложения, перезапуска KWin или Plasma и выхода из сеанса потоки не должны остаться заглушёнными.
  - Наше глушение не должно попасть в сохранённое состояние WirePlumber (поэтому `softMute`).
- Настройки:
  - Список приложений — по `resourceClass` или desktop-файлу.
  - Настраивать на странице скрипта KWin в «Параметрах системы».

## Команды-справка

Дополнить командами сборки сервиса и установки, когда они появятся.

- Скрипт KWin:
  - установить или обновить: `kpackagetool6 --type KWin/Script -i <папка>` или `-u <папка>`;
  - включить: `kwriteconfig6 --file kwinrc --group Plugins --key <id>Enabled true`, затем `qdbus6 org.kde.KWin /KWin reconfigure`.
- Логи скрипта KWin (`print`/`console.log`): `journalctl --user -f | grep -i kwin`.
- Узлы и их свойства: `pw-dump`; параметры узла: `pw-cli enum-params <node> Props`.
- Сохранённое состояние потоков WirePlumber: `~/.local/state/wireplumber/stream-properties`.
