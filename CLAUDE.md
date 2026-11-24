# Правила для Claude в этом репозитории

- Это форк `fxceless/victory-gta6`, который сам стоит на `Marvisak/VIctory`.
  Схема и автосинхронизация: `docs/upstream-sync.md`.
- Перед любой задачей проверить, не сделано ли это уже в апстримах: ветка
  `upstream-watch` + поиск по PR/issues обоих апстримов (шаги в
  `docs/upstream-sync.md`, раздел "Как не делать работу дважды"). Результат
  проверки и задачу записать в `docs/fork-tracker.md`.
- В `main` апстримы попадают только через merge (PR "Sync upstream"),
  историю `main` не переписывать.
- XCL (ivall/xcl, GPL-2.0-only): только идеи, код не копировать.
- Сборка и запуск игры — только на Windows-ПК владельца (игра на диске B:),
  в облачном Linux-контейнере игру не запустить; `cargo test` частично.
- Remotes для локальной работы:
  `git remote add marvisak https://github.com/Marvisak/VIctory`,
  `git remote add fxceless https://github.com/fxceless/victory-gta6`.
