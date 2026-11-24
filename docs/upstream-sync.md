# Синхронизация с апстримами

## Откуда что берётся

```
Marvisak/VIctory  main ──●──●──●──(ffe310e)──●──●…        апстрим, GPL-3.0-or-later
                                     \
fxceless/victory-gta6 main            ●──●──●──●──●──●──●  7 линейных коммитов поверх ffe310e
                                                         \
oceansde012/victory-gta6 main                             (наша работа)
```

- `fxceless/victory-gta6` — не сквош поверх произвольной истории, а обычная
  линейная ветка: первый коммит `578d459` ("this fork's changes on top of
  VIctory main (ffe310e)") — сквош всех ранних правок fxceless, дальше 6 обычных
  коммитов (включая PR #1 от ZazaJr24). История Marvisak под ними сохранена
  целиком, поэтому **оба апстрима сливаются в наш main обычным `git merge`**,
  без rebase и cherry-pick.
- Наш `main` на момент создания = `fxceless/main` (`1684989`), то есть уже
  содержит и Marvisak до `ffe310e`.
- Порядок слияния: сначала fxceless (он сам стоит на Marvisak), потом Marvisak
  (то, что ушло в апстрим после точки, на которой сидит fxceless).
- Нашу историю **не переписываем** (никаких rebase main на апстрим): только
  merge, иначе каждое обновление апстрима будет ломать открытые ветки.

## Автоматика (каждые 10 минут)

`.github/workflows/upstream-sync.yml` → `.github/scripts/upstream-sync.sh`:

1. Тянет `main` обоих апстримов.
2. Если там есть коммиты, которых нет в нашем `main`, сливает их в ветку
   `sync/upstream` и держит **один** PR "Sync upstream (Marvisak, fxceless)"
   в `main` (обновляет его, а не плодит новые). В описании PR — список
   пришедших коммитов.
3. Конфликт: слияние останавливается на конфликтующем апстриме, PR получает
   лейбл `sync-conflict`, а вверху ветки `upstream-watch` появляется ⚠️.
   Разрешать руками (или попросить Claude):
   ```
   git fetch origin && git fetch marvisak && git fetch fxceless
   git checkout -B sync/upstream origin/main
   git merge fxceless/main   # затем git merge marvisak/main
   # разрешить, git commit, git push -f origin sync/upstream
   ```
4. Обновляет ветку **`upstream-watch`** (файл `README.md`): открытые и последние
   30 влитых PR обоих апстримов, и сколько коммитов каждого ещё не в нашем main.
   Коммит туда делается только когда список реально изменился.

Если апстримы не двигались, запуск ничего не пушит.

Ограничения GitHub: cron `*/10` — минимум 5 минут, на практике запуски
опаздывают на 5–20 минут под нагрузкой; у форков расписание по умолчанию
выключено (включается во вкладке Actions); после 60 дней без коммитов в репо
GitHub сам отключает расписание. PR, созданный токеном `GITHUB_TOKEN`, не
запускает `CI` — нужно один раз нажать "Re-run" или закрыть/открыть PR вручную.
Публичный репозиторий — минуты Actions бесплатны.

Ручной запуск: Actions → Upstream sync → Run workflow.

## Как не делать работу дважды

Перед любой задачей (код, фикс, оптимизация):

1. Открыть ветку `upstream-watch` и поискать по области задачи (шейдеры, RT,
   тени, barrier, pipeline, fps и т.д.).
2. Поискать в issues/PR обоих апстримов (включая закрытые):
   `https://github.com/Marvisak/VIctory/pulls?q=<слова>` и
   `https://github.com/fxceless/victory-gta6/pulls?q=<слова>`.
3. Если нашлось:
   - влито в апстрим → дождаться sync PR (или влить его сразу);
   - открытый PR → либо ждать, либо забрать коммиты к себе
     (`git fetch marvisak pull/<N>/head:up-<N> && git cherry-pick …`) и записать
     в `docs/fork-tracker.md`, чтобы потом не было дубля при слиянии апстрима;
   - кто-то обсуждает в issue → учесть выводы.
4. Только если ничего нет — делать своё и записать задачу в
   `docs/fork-tracker.md`.

Код из Marvisak и fxceless брать можно (оба GPL-3.0-or-later, как и мы).
**XCL (ivall/xcl, GPL-2.0-only) — только как источник идей, код не копировать.**
