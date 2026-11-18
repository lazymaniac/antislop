# Końcowa walidacja zgodności

## Zakres i źródła

To jest raport odczytowej kontroli wskazanych plików w bieżącym checkoutcie po zatwierdzonej naprawie testów. Nie zmieniono kodu, testów ani konfiguracji. Wnioski o implementacji pochodzą z bezpośrednio przeczytanych symboli i testów poniżej; raport zależności nie zastępuje tej kontroli.

Pełny raport poprzedniego zadania i zachowane pomiary znajdują się w:

- Raport zależności: `repair-gate-options-unit-tests`, wynik `e203aeaa-ff3a-44cc-b38e-e132dad65fcb`.
- Trwały raport wykonania: `/Users/sebastian/.local/state/antislop/projects/fcd81498ab4176caf43a61287ab0c59fbc3f6b16b7ff9af2a0ce856efad5fcd2/task-state/302b97ba-d9ff-4c16-8006-39cd1ae9d1e0/g0/run-20261118-options-audit/audit-report.md`.
- Niezmienne dowody MCP: testy jednostkowe `81947596-bcf4-4eb0-84b6-91dd07a2e6cc`; format/check/clippy/review `5f833b0c-1cf6-46d8-8377-ef363de67884`; testy integracyjne wraz z nieudanym pierwszym przebiegiem i retry `46b7cdf4-1286-417c-92b5-34d7e0e20895`.

Dowody komend pochodzą z zatwierdzonego zadania zależnego i są tu podane jako jego pomiary, nie jako komendy uruchomione ponownie w tym zadaniu raportowym.

## R1.1 — rozszerzona karta wypełnia dostępny obszar z marginesem

W `src/lean/tui/graph/model.rs`, `Size::fit` wyznacza docelowy rozmiar przez odjęcie dwóch komórek szerokości i wysokości viewportu. `Size::limit_expanded_width` zachowuje ten limit, a `Size::grow` wylicza rozmiar animowanej karty. W `src/lean/tui/graph/layout/units.rs`, `resize` ponownie oblicza rozmiar, a `fit_expanded_card` odejmuje margines dwóch komórek oraz obramowanie jednostki (po dwie kolumny/wiersze, gdy karta należy do ramki zadania). Zatem ramka zadania pozostawia jedną komórkę z każdej strony; sama karta jest wewnątrz tej ramki.

Zaobserwowane regresje:

- `src/lean/tui/graph/tests.rs::expanded_card_reflows_the_graph_without_overlapping_its_neighbours` sprawdza powiększenie ponad rozmiar zwykłej karty, brak poziomego przewijania i położenie pozostałych elementów.
- `src/lean/tui/graph/tests.rs::expanded_pty_content_grows_with_the_viewport` sprawdza rozmiar viewportu 76×12, kartę 74×10 oraz wzrost obszaru treści z 364 do 490 komórek.
- `src/lean/tui/graph/tests.rs::an_expanded_card_spans_a_narrow_viewport_past_the_lanes_that_end_above_it` sprawdza ramkę w viewportcie szerokości 70: x=1..69 i szerokość grafu 70.
- `tests/lean_tui.rs::expanded_card_content_measurements_cover_compact_near_full_and_wide_terminals` sprawdza geometrię dla terminali 80×24, 120×30 i 160×50; asercje wymagają jednej komórki marginesu ramki oraz `graph.width <= viewport.0`.

Wynik inspekcji: implementacja i testy pokazują wypełnienie dostępnego obszaru z wymaganym marginesem, z uwzględnieniem obramowania ramki zadania.

## R1.2 — resize przelicza układ i ogranicza rysowanie

W `src/lean/tui/render/specialized_screens/graph.rs`, `graph_areas` aktualizuje `graph_viewport` po zmianie rozmiaru i unieważnia `graph_revealed`; ścieżka rysowania pobiera nowy układ z `get_with_expanded`. `fit_graph_offset` ogranicza przewinięcie do nowej geometrii. `reveal_selected_node` ponownie ustawia offset rozszerzonej karty z marginesem ramki. `draw_expanded_card_panels` oblicza pozycję ekranową i przekazuje obszary do `expanded_card_areas`, która przycina rysowanie do viewportu.

Testy w `src/lean/tui/tests.rs`:

- `expanded_task_frame_keeps_one_cell_margin_across_viewport_resizes` zmienia ten sam terminal z 120×40 na 80×24 i 12×5. Dla viewportów co najmniej 40×16 sprawdza dokładnie cztery krawędzie ramki z jedną komórką marginesu oraz wymiary karty `viewport.width - 6` i `viewport.height - 4`. Dla małego viewportu sprawdza, że niepuste prostokąty karty/tytułu/treści po clippingu mieszczą się w viewportcie; sprawdza też, że zmienił się początek ramki po resize i zwykła karta nie zmieniła wymiarów.
- `src/lean/tui/graph/tests.rs::expanded_card_areas_clip_with_a_card_scrolled_past_the_top` sprawdza przycięcie karty i treści przy scrollu ponad górną krawędź.
- `tests/lean_tui.rs::expanded_card_resize_redraws_same_app_with_margins_and_clipping` niezależnie sprawdza ponowne rysowanie tej samej aplikacji przy 120×40, 80×24 i 12×5, margines na większych rozmiarach i granice prostokątów clippingu przy najmniejszym rozmiarze.

Wynik inspekcji: testy sprawdzają dokładny margines tam, gdzie viewport pozwala go zachować, a widoczne, przycięte obszary pozostają w granicach viewportu dla 12×5. Nie twierdzę, że pełna nieprzycięta ramka mieści się w wymiarach, które są zbyt małe.

## Dodatkowa prośba — quality gates bez options

W `src/lean/tasks/agents.rs`, `eligible_for_options` zwraca `task.changes_code() && !task.quality_gate`; `with_options` tworzy i dodaje prerequisite tylko dla tak zakwalifikowanych zadań. `src/lean/core/plan/helpers.rs::finalize_team_plan` nakłada `with_options` tylko przy włączonej polityce i teamie dopuszczającym options, w przeciwnym razie usuwa options. `src/lean/tasks/prompts/plan_review.rs::GENERATED_NODES` mówi, że quality gates nie dostają options i nie wymagają `selected_approach`, a `plan_entry` ujawnia w grafie przeglądu `options_for`, `quality_gate`, `depends_on` i oznaczenie węzłów runtime.

Szczególna kontrola dwóch asercji wskazanych w poprzednim audycie:

- `src/lean/plan_tests.rs::small_change_rigor_adds_only_static_analysis_and_testing_gates` filtruje właścicieli options przez `changes_code() && !quality_gate` (wiersze 390–393), następnie osobno sprawdza, że bramki nie mają przypisanego options task i nie zależą od options task (wiersze 418–440).
- `src/lean/plan_tests.rs::a_team_runs_only_its_own_gates_and_options` buduje `code_writers` z warunkiem `options_for.is_none() && changes_code() && !quality_gate` i porównuje z nimi wszystkie options (wiersze 680–695), a następnie osobno wyklucza bramki jako właścicieli i odbiorców zależności options (wiersze 696–710). Te asercje options dotyczą niebramkowych writerów; nie liczą quality gates jako writerów.

Odpowiadające regresje są także w `src/lean/tasks/tests/options_and_gates.rs::options_cover_code_writers_but_exclude_quality_gates_deterministically`: pętla writerów filtruje niebramkowe zadania dostarczające kod, a osobna pętla sprawdza brak options przypisanego bramkom i brak ich zależności od options. `src/lean/tasks/tests/plans_and_messages.rs::plans_that_write_get_the_required_quality_gates` osobno sprawdza zależności bramek od zbioru options. `refreshing_a_legacy_gate_options_task_removes_it_and_its_edges` sprawdza usunięcie historycznego options task i jego krawędzi po oznaczeniu zadania jako gate.

Wynik inspekcji: warunek produkcyjny, agregacja planu i nazwane testy są zgodne; brak dowodu na przypisywanie options do quality gates w sprawdzonym zakresie.

## Dodatkowa prośba — brak predefined/required context-scout

W `src/lean/tasks/agents.rs` sprawdziłem `PREDEFINED_AGENT_POOL` oraz generowanie wymaganych agentów; nie znalazłem tam `context-scout` ani `WorkKind::Context` jako predefined agenta. W `src/lean/tasks/templates.rs` katalog teamów i ich gate’y nie dodają tego rodzaju agenta. `src/lean/core/plan/helpers.rs::finalize_team_plan` składa team gate’y oraz opcjonalne options; nie dodaje context scout. W przeszukanym `src/lean/tasks/prompts/plan_review.rs` nie ma odwołania do `context-scout`. Test `src/lean/plan_tests.rs::context_scout_is_not_predefined_or_required_and_legacy_context_tasks_round_trip` sprawdza brak agenta w puli i katalogu, brak nazwy w instrukcjach planera/orchestratora, brak automatycznego `WorkKind::Context` w planach wszystkich teamów oraz zachowanie serializacji historycznego zadania typu context.

`src/lean/tasks/prompts/planner.rs` nie istnieje w bieżącym checkoutcie (sprawdzone `test -e`); nie przypisuję temu plikowi żadnej obserwacji. Wniosek o instrukcjach opiera się na istniejącym teście faktycznie generowanych `planner_instructions()` i `orchestrator_instructions()`.

## Wyniki sprawdzeń zachowane w poprzednim zadaniu

Poniżej dokładnie raportuję wyniki z odczytanych niezmiennych dowodów. Są to wyniki poprzedniego przebiegu walidacji po naprawie testów:

- `./cargo.sh test --lib small_change_rigor_adds_only_static_analysis_and_testing_gates -- --nocapture`: exit 0, 1 passed.
- `./cargo.sh test --lib a_team_runs_only_its_own_gates_and_options -- --nocapture`: exit 0, 1 passed.
- `./cargo.sh test --lib`: exit 0, 944 passed, 0 failed, 2 ignored, 0 measured, 0 filtered out.
- `./cargo.sh test --test lean_tui`: exit 0, 113 passed, 0 failed.
- `./cargo.sh test --test lean_core`, pierwszy pełny przebieg: exit 101, 24 passed, 1 failed. Test `a_changed_configuration_waits_for_running_work_and_says_so` zakończył się surowym błędem `the daemon disconnected\n`. Izolowane powtórzenie tego testu: exit 0, 1 passed. Następnie pełny retry `./cargo.sh test --test lean_core`: exit 0, 25 passed, 0 failed.
- `./cargo.sh fmt --check`: exit 0.
- `./cargo.sh check`: exit 0.
- `./cargo.sh clippy --all-targets -- -D warnings`: exit 0.
- `hologram review --root . 3e48a57be27a4d16fb332871b274f288c2efcd28 --json`: exit 0, `count: 0`, `findings: []`.
- `git diff --check`: exit 0.

## Ograniczenia i konkluzja

Raport dependency i zachowane logi nie podają powiązania wyników komend z digestem bieżącego checkoutu; nie weryfikowałem takiego powiązania i nie twierdzę, że testy uruchomiono w tej turze raportowej. Natomiast wymienione wyżej pliki kodu i testów przeczytałem bezpośrednio w bieżącym checkoutcie. Pierwszy wynik `lean_core` z daemon disconnect pozostaje odnotowany mimo udanego retry. Brak wcześniejszego zatwierdzenia audytu nie jest dowodem defektu. Nie stwierdziłem defektu w sprawdzonym zakresie; nie audytowałem pozostałych obszarów repozytorium.

## Stan artefaktu

Ponowna kontrola w tym checkoutcie potwierdziła nierozwiązany stan zgłoszony przez recenzenta: przed korektą `git ls-files --stage -- reports/final-compliance-validation.md` zwracał stage 0 (`100644 60ba469a8c874cba02e32f18fd17491f7e2b5bb5 0`), a status dla ścieżki był pusty. Wykonałem `git rm --cached -- reports/final-compliance-validation.md`, zachowując plik roboczy. Po tej zmianie `git ls-files --stage -- reports/final-compliance-validation.md` nie zwraca wpisu; `git status --short --untracked-files=all -- reports/final-compliance-validation.md` zwraca `D  reports/final-compliance-validation.md` oraz `?? reports/final-compliance-validation.md`; `test -f reports/final-compliance-validation.md` kończy się kodem 0. Artefakt istnieje i jest nieśledzony; usunięcie poprzedniego wpisu z indeksu jest staged. Końcowe polecenia i wynik statusu zapisano jako MCP evidence `db85b69d-8b6f-4277-ad9a-237511dcd228`, z kopią w `/Users/sebastian/.local/state/antislop/projects/fcd81498ab4176caf43a61287ab0c59fbc3f6b16b7ff9af2a0ce856efad5fcd2/task-state/9d404a3c-52e5-46cf-bddf-8c02a105f8b4/g0/run-20261118-final-compliance-round8/artifact-state.txt`. Nie utworzyłem commitu.
