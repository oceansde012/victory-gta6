# Трекер форка

Что взято из апстримов вне обычного sync и что делаем сами. Обновлять при
каждой задаче. Статусы: `план`, `в работе`, `готово`, `ушло в апстрим`, `отменено`.

## Проверки инфраструктуры

- Анализатор производительности: точечное разрешение для `tools/perf/analyze_run.py` в PR #8; 25 ноября поиск `policy python allowlist` в обоих апстримах не показал аналогичного исправления. Проверки содержимого сохранены.
- Проверяемая сборка для измерений: `tools/perf/build-verified.ps1` требует чистое дерево и ожидаемый коммит, сохраняет SHA-256 созданного EXE; 25 ноября поиск `verified build manifest` в обоих апстримах не показал аналогичного средства. Игра не запускается.

  В чистой рабочей копии: `powershell -NoProfile -ExecutionPolicy Bypass -File tools/perf/build-verified.ps1 -ExpectedCommit <commit> -TargetDir B:\victory-build`. Манифест сохраняется в target-директории; для запуска нужно отдельно проверить хеш EXE и параметры игры.

## Взято из апстримов досрочно (cherry-pick открытых PR)

| Апстрим PR | Что | Куда у нас | Статус |
|------------|-----|------------|--------|
| fxceless #63 (arminkhaleghirod) | Hash recently changed textures at every check (startup flicker fix) | `claude/project-thread-5e9x96` | в работе |
| fxceless #71 (MartinIndra02) | D3D12 COPY queue for relay card copies | `claude/project-thread-5e9x96` | в работе |
| fxceless #32 (arminkhaleghirod) | RGBA16F stand-in for R9G9B9E5 render targets | `claude/project-thread-5e9x96` | в работе |
| fxceless #82 (Kaivyyy) | Depth bounds test passed to host | `claude/project-thread-5e9x96` | в работе |
| fxceless #83 (ryotdev) | Keep empty-EXEC branches that can reactivate lanes | `claude/project-thread-5e9x96` | в работе |
| fxceless #81 (undeadhuman37) | Descriptor read only as 8 dwords of data is an image | `claude/project-thread-5e9x96` | в работе |
| fxceless #77 (m335671) | `v_fma_mixlo/mixhi` over f16 first half keeps zero | `claude/project-thread-5e9x96` | в работе |

## Наши задачи

| # | Задача | Ветка / PR | Проверено в апстримах | Статус |
|---|--------|------------|-----------------------|--------|
| 1 | Настройка синхронизации с апстримами | `chore/upstream-sync` | — | в работе |
| 2 | Сборка и запуск на ПК (Windows, игра на B:) | — | — | план |
| 3 | FPS в мире (сейчас 2–4) | `claude/project-thread-5e9x96` (AnyXboxX "lot 5" mix, see `docs/fps-mix.md`) | 2026-11-24: AnyXboxX (LooperSalty) runs fxceless `7b0f736` + PRs #63 #71 #32 #82 #83 #81 #77, 5.4 fps / best image on RTX 4070 SUPER; lot 4 (#63 #71 #32 + `VICTORY_RELAY=2`) 6.6 fps. Not on PC yet. | в работе |
| 4 | Тени / RT-освещение (сейчас чёрные) | — | — | план |
| 5 | CBL VIctory (cocainebunnylaboratories/cblvictoryvi) texture/reflection fixes review | cool_fox PR #6 (dab3f08 page-mapping planner); report `reports/cbl-victory-2026-11-27.md` in project files | 2026-11-27: texture fixes are not CBL's own: Vice-Tay #137 (`53a1f16`, from VI-BR-PORT `87dc307` / fxceless #896) and #141 (`7993c1b`, from VI-BR-PORT `5c17c8c` / fxceless #1241). Both missing in cool_fox `f4b48ad`, both change the picture: owner decides | готово (picture fixes waiting for owner) |
