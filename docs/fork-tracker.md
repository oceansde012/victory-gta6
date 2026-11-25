# Трекер форка

Что взято из апстримов вне обычного sync и что делаем сами. Обновлять при
каждой задаче. Статусы: `план`, `в работе`, `готово`, `ушло в апстрим`, `отменено`.

## Взято из апстримов досрочно (cherry-pick открытых PR)

| Апстрим PR | Что | Куда у нас | Статус |
|------------|-----|------------|--------|
| [fxceless #818](https://github.com/fxceless/victory-fork/pull/818) (Robbha, `f888b29`) | Учёт коротких проходов загрузки текстур в бюджете `VICTORY_BAKE_TEXTURES_MS` с точностью до микросекунд | `codex/texture-bake-budget` поверх #7 | готово |
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
| 5 | Upstream integration, stage 1: fxceless main `8dd30ab` merged; 108 Marvisak/VIctory commits/PRs cherry-picked, GPU-contract work deferred (see `docs/upstream-integration.md`) | `claude/project-thread-2l2dcf` | 2026-11-25: fxceless issue #126 (same 274-hunk trial merge, staged plan); no published merged branch | в работе |
| 6 | Perf picks round 1: 12 upstream perf/stability PRs + F3 overlay + heap texture budget switches (see `docs/perf-picks.md`) | `claude/project-thread-4f4qka` | 2026-11-25: open PRs of Marvisak (since 11-22) and fxceless searched; tracker gta6-pc-race read | в работе |
| 7 | Убрать блокировку на каждый слот и атомарный счётчик на каждый пропуск при синхронизации дескрипторов | поверх #7 и исправления бюджета текстур | 2026-11-25: проверены upstream-watch, PR и issues обоих апстримов. [fxceless #774](https://github.com/fxceless/victory-fork/pull/774) группировал блокировку по страницам. Здесь история слота защищена уже взятой блокировкой реестра куч, с сохранением обработки границ страниц и табличных слотов. | готово |
| 8 | Исправить состояние CLZERO-цикла при некратной границе | `codex/fix-clzero-loop` | 2026-11-25: fxceless #407 описывает ту же ошибку; используется уже имеющийся у нас обработчик `crash::clzero`, без переноса кода PR | в работе |
| 9 | Синхронизировать первую запись CPU с тестом слияния буфера на WARP | `codex/synchronize-resident-merge-test` | 2026-11-25: в upstream-watch, PR и issues fxceless и Marvisak исправления не найдено | готово |
| 10 | Загружать stencil-плоскость текстур глубины | `codex/stencil-texture-upload` | 2026-11-25: адаптация [Marvisak #2509](https://github.com/Marvisak/VIctory/pull/2509) к потоковой загрузке и уровням по запросу в нашей ветке; эффект в игре не проверен | в работе |
| 11 | Ограничить длину строк текстуры размером staging-буфера | `codex/stencil-texture-upload` | 2026-11-25: проверен код Marvisak в основании #2509: там уже есть ограничение `subresource_len`; адаптировано к нашей ветке с учётом недопустимого смещения и переполнения | в работе |
