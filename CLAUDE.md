# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Проект

MuteOnMinimize — дополнение для KDE Plasma 6. Выбранные пользователем приложения
заглушаются, пока свёрнуты все их окна, и снова звучат, когда окно развернули.

- Репозиторий: github.com/BeleBob/MuteOnMinimize (публичный), лицензия GPL-3.0-or-later.
- Состояние: заготовка, кода нет. Устройство ещё не выбрано (см. «Первая задача»).

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
  - Есть: `kpackagetool6`, `qmllint`, `qmlformat`, `qdbus6`, `kwriteconfig6`, `busctl`, `pactl`, `wpctl`, `cmake`, `extra-cmake-modules`.
  - `plasma-sdk` (`plasmoidviewer`) не установлен. Ставить пакеты только с согласия владельца: нужен sudo.

## Первая задача: выбрать устройство вместе с владельцем

Что известно:
- **Окна.** Свёрнутость окон на Wayland надёжно видит только KWin. Скрипт KWin на JS или QML получает:
  - `workspace.windowList()`, сигналы `workspace.windowAdded` и `workspace.windowRemoved`;
  - у окна: `minimized` и сигнал `minimizedChanged`, `pid`, `resourceClass`, `desktopFileName`.

  Скрипт KWin не запускает процессы и не управляет звуком, но может вызывать D-Bus (`callDBus`).
  Отдельная программа без KWin состояние окон на Wayland не видит.
- **Звук.** У каждого приложения свои потоки: sink-input в pipewire-pulse со свойствами
  `application.process.id`, `application.process.binary` и `application.name`. Проверено на Firefox.
  Заглушить: `pactl set-sink-input-mute <id> 1` (или libpulse), либо `wpctl set-mute <node> 1`.

Варианты устройства:
1. **Скрипт KWin и пользовательский D-Bus-сервис** (Python или Go, systemd `--user`).
   - Скрипт следит за окнами и сообщает сервису о сворачивании, сервис глушит потоки.
   - Плюсы: надёжно, на стабильных API.
   - Минусы: две части и установка сервиса.
2. **Только виджет Plasma на QML.**
   - Окна берутся из `org.kde.taskmanager` (TasksModel: IsMinimized, AppPid, AppId), звук — из `org.kde.plasma.private.volume` (потоки с Muted).
   - Плюсы: без отдельного сервиса.
   - Минусы: `private` API может меняться между версиями Plasma, и всё работает, только пока виджет стоит на панели.

## Что учесть при любом устройстве

- Окна и потоки:
  - Если у приложения несколько окон, глушить только когда свёрнуты все.
  - Звук часто идёт из дочернего процесса (Firefox, Chromium, Electron), поэтому PID окна может не совпасть с `application.process.id`. Сопоставлять по дереву процессов или по `application.process.binary`, проверить на реальных приложениях.
  - Потоки, появившиеся, пока окно свёрнуто, тоже глушить.
- Возврат звука:
  - Включать звук только потокам, которые заглушили мы. То, что пользователь выключил сам, не трогать.
  - После закрытия приложения, перезапуска KWin или Plasma и выхода из сеанса потоки не должны остаться заглушёнными.
  - WirePlumber запоминает свойства потоков (`node.stream.restore-props`). Проверить, не сохранится ли наш mute до следующего запуска приложения.
- Настройки:
  - Список приложений — по `resourceClass` или desktop-файлу.
  - Настраивать в «Параметрах системы» (у скриптов KWin есть страница настроек) или в самом виджете.

## Команды-справка

Заполнить, когда устройство выбрано. Общее для пакетов Plasma и KWin:
- Скрипт KWin:
  - установить или обновить: `kpackagetool6 --type KWin/Script -i <папка>` или `-u <папка>`;
  - включить: `kwriteconfig6 --file kwinrc --group Plugins --key <id>Enabled true`, затем `qdbus6 org.kde.KWin /KWin reconfigure`.
- Виджет: `kpackagetool6 --type Plasma/Applet -i <папка>`.
- Логи скрипта KWin (`print`/`console.log`): `journalctl --user -f | grep -i kwin`.
- Проверка QML: `qmllint`.
