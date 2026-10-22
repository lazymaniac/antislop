# Bramka integracyjna options — 2026-10-22

Wynik: **bramka nie jest zielona**. Niezależnie sprawdzono zintegrowany
HEAD `a28cc16a27dd8164ff1953ee950be48d5a3e26cf`. Mechanika planowania i testy
przepływu options przechodzą. Kontrole core/TUI nie przechodzą w obecnym sandboxie.
Nie uzyskano dowodu semantycznej poprawności decyzji rzeczywistych modeli.
Nie zmieniono implementacji ani testów; ten dokument jest wynikiem zadania.

## Wykonane kontrole

Logi tego niezależnego uruchomienia: `.antislop/integration-gate/` w tym worktree.
Nie korzystano z logów innego worktree jako dowodu wykonania.

| Polecenie | Wynik | Log |
| --- | --- | --- |
| `cargo build` | PASS, kod 0 | `build-confirm.log` |
| `cargo fmt --check` | PASS, kod 0 | `fmt-confirm.log` |
| `cargo check` | PASS, kod 0 | `check-confirm.log` |
| `cargo clippy --all-targets -- -D warnings` | PASS, kod 0 | `clippy-confirm.log` |
| `cargo test --test lean_core --test lean_tui --test lean_options` | FAIL, kod 101; core 13/20; dalsze targety niewykonane przez to polecenie | `tests.log` |
| `cargo test --test lean_tui` | FAIL, kod 101; 101/110 | `lean_tui.log` |
| `cargo test --test lean_options` | PASS, kod 0; 4/4 | `lean_options.log` |
| `cargo test --lib` | FAIL, kod 101; 309/310 | `lib.log` |
| `cargo test --lib lean::agent::tests::only_the_turns_own_process_group_is_reaped -- --exact` | FAIL, kod 101; 0/1 | `reap-retry.log` |
| `cargo test --test lean_options_probe -- --nocapture` | PASS, kod 0; 1/1, obejmuje oczekiwane kontrprzykłady | `probe.log` |

Nie pomijano testów ani nie zmieniano konfiguracji, uprawnień lub sandboxa.
Nie powtarzano całej regresji pozostałych targetów: już wymagane core/TUI
uniemożliwiają zieloną bramkę. Poprzedni raport 597/597 nie jest wynikiem tego
niezależnego uruchomienia. `checks-status.json` i `separate-status.json` zapisują
kody zakończenia kontroli i oddzielnych targetów.

## Porównanie każdego kryterium

Identyfikatory poniżej odpowiadają kryteriom ledger. PASS oznacza dowód wskazanego
zachowania kodu lub kontraktu; nie oznacza uniwersalnej gwarancji zachowania LLM.

| Kryterium | Ocena i dowód |
| --- | --- |
| `ac-options-purpose-plan` | PASS: `src/lean/tasks.rs::with_options` tworzy `WorkKind::Options`; dokumentacja `docs/session-model.md`, sekcja Options before each writing task, opisuje porównanie i wybór. |
| `ac-options-purpose-implementation` | PASS kontraktu: generowany goal nakazuje 2–4 realistyczne warianty i wspólne kryteria; `checklist(Options)` sprawdza alternatywy i dowody. Test integracyjny sprawdza treść promptu. Faktyczne porównanie przez model: luka dowodowa opisana poniżej. |
| `ac-options-purpose-selection` | PASS kontraktu: odrzucenie wariantów naruszających wymagania przed rankingiem, wybór oparty na dowodach i `Selected: <id>`. Faktyczny najlepszy wybór modelu: NIEPOTWIERDZONY. |
| `ac-options-purpose-integration` | PASS mechaniki na atrapach: `orchestrator_plan_is_completed_before_each_writer_runs` oraz test odstępstw wykonują Core, plan, zależności, pliki writera i osobną rolę weryfikatora. Pełny rezultat semantyczny z rzeczywistymi modelami: NIEPOTWIERDZONY. |
| `ac-options-every-writer-coverage` | PASS: integracja iteruje po wszystkich `writes=true` i sprawdza bezpośrednią zależność options oraz approved przed working. |
| `ac-options-every-writer-gates` | PASS: ten sam test dla `required_gates=true/false`, z potwierdzeniem obecności dodatkowych writerów przy włączonych bramkach; test jednostkowy `options_cover_all_writers_including_gates_deterministically`. |
| `ac-options-runtime-auto` | PASS: plan atrapy zawiera wyłącznie writera; runtime dodaje options w `propose` przez `finalize_plan`, przed utrwaleniem i oceną planu. |
| `ac-options-runtime-dependency` | PASS: sprawdzane `depends_on` oraz kolejność zdarzeń approved options przed working writera. |
| `ac-options-runtime-deterministic` | PASS: porównanie wyników normalizacji tego samego wejścia w integracji i testach jednostkowych. |
| `ac-options-runtime-existing` | PASS: `shared_options_are_not_duplicated_and_bind_every_writer_even_with_ambiguous_reports` pozostawia trzy zadania dla shared i dwóch writerów; test jednostkowy sprawdza także kolizje ID. |
| `ac-options-runtime-kind` | PASS: integracja szuka zależności o `WorkKind::Options`; jednostkowy `options_inherit_writer_context_and_precede_it` sprawdza kind. |
| `ac-options-runtime-readonly` | PASS: integracja sprawdza `writes=false`, runtime wybiera `Access::ReadOnly` i projekt jako cwd; sonda potwierdza brak base/result options. |
| `ac-options-binding-delivery` | PASS: pełny raport options znajduje się w przechwyconych promptach writera i jego weryfikatora. |
| `ac-options-binding-evidence` | PASS na atrapach: test odstępstw używa deklaracji naruszenia kontraktu A i pliku `required-format.txt`; identyczna deklaracja przy dowodzie A jest odrzucana, przy dowodzie B akceptowana. |
| `ac-options-binding-implementation` | PASS na atrapach: wariant A zostaje zaakceptowany; B wymaga potwierdzonego odstępstwa. Rzeczywisty writer: NIEPOTWIERDZONY. |
| `ac-options-binding-justification` | PASS kontraktu i testu: `OPTIONS_BINDING`, `OPTIONS_VERIFICATION`; „I prefer B” nie wystarcza atrapowemu weryfikatorowi. |
| `ac-options-binding-no-silent-switch` | PASS na atrapach: B bez deklaracji odstępstwa nie uzyskuje approved, mimo samopotwierdzenia workera. |
| `ac-options-binding-verification` | PASS mechaniki: osobna sesja/rola verifier, read-only w checkoutcie writera, raport options i rzeczywiste pliki dostępne niezależnie od deklaracji workera. Test sprawdza verdict i brak zdarzenia approved przy odrzuceniu. Semantyka rzeczywistego weryfikatora: NIEPOTWIERDZONA. |

Wszystkie jednostkowe testy options, finalizacji i konfiguracji przeszły w `lib.log`;
jedyny błąd biblioteki dotyczy sprzątania procesu.

## Niezgodności i ograniczenia dowodów

1. **Wymagane kontrole nie są zielone.** Core: siedem błędów; TUI: dziewięć;
   biblioteka: jeden. Core zwraca m.in. `the daemon exited (exit status: 1):
   Operation not permitted (os error 1)`, `the orphaned CLI still runs`,
   `the orchestrator never started`. TUI pada m.in. bezpośrednio na
   `UnixListener::bind` (`tests/lean_tui.rs:5907`, `:1368`), a trzy testy edytora
   kończą się timeoutem. `reap` zależy od `ps` (`src/lean/agent.rs:466`);
   próba uruchomienia `ps` wyłącznie dla własnego procesu kończy się
   `PermissionError: [Errno 1] Operation not permitted: 'ps'` (`ps-own.log`).
   To potwierdza ograniczenia wykonania, nie daje dowodu regresji options.
   Potrzebne ponowienie tych samych kontroli w środowisku dopuszczającym lokalne
   sockety i inspekcję własnych procesów. Nie obchodzono ograniczeń.

2. **Najlepszy wybór i niezależna ocena modeli nie są dowiedzione przez atrapy.**
   `tests/common/options_cli.py` zwraca `settings['report']` dla options i zapisuje
   `settings['implementation']` dla writera. Wyrocznia odczytuje pliki i dowody,
   lecz nie wykonuje rzeczywistego porównania wariantów przez model. Co więcej,
   test shared dopuszcza raport bez wyboru i raport z różnymi wyborami.
   Runtime zgodnie z udokumentowanym podejściem A nie parsuje Selected i opiera
   się na ocenie agentów. Brak parsera nie jest sam w sobie naruszeniem
   zatwierdzonego mechanizmu. Do zamknięcia luki potrzebne są rzeczywiste tury
   options → writer → niezależny verifier z zapisanym porównaniem, wyborem,
   diffem i oceną odstępstwa. W tym zadaniu nie uruchamiano usług zewnętrznych;
   nie wolno traktować zielonej atrapy jako takiej atestacji.

3. **Potwierdzona różnica stanu options i writera.** Sonda przechodzi, ponieważ
   oczekuje także błędnych decyzji. W `unmerged-opaque` i `unmerged-wrong`
   options widzi `7\n11`, wybiera lines, podczas gdy W2 oraz pisząca bramka
   otrzymują `{"records":[7,11]}`; niezależne dekodowanie daje null/FAIL.
   `unmerged-accurate` działa dzięki poprawnemu raportowi zależności.
   `src/lean/mod.rs:3200` i następne tworzą writer_base tylko dla writerów;
   options czyta projekt. Goal jawnie ostrzega o tym ograniczeniu.
   Nie potwierdza to gwarancji najlepszego wyboru dla dowolnych raportów.
   Możliwa poprawa wymaga osobnego ustalenia: zapewnić options read-only widok
   złożonych wyników zależności albo dowodowo zweryfikowany kontrakt tych zmian.
   Nie rozszerzano tu architektury.

4. **Kontekst kolejnych tur nie jest ponownie kompletnie doklejany.** Początkowe
   prompty dostają upstream (`src/lean/mod.rs:3345`, `:3464`), zwykły followup i
   poczta polegają na sesji. Gałąź poczty `if first` składa worker_prompt bez
   upstream (`:3369`). Nie jest to dowód utraty kontekstu w sprawnej sesji;
   brak testu kontraktu wyboru po utracie sesji w trakcie poczty. Proponowane
   uzupełnienie: test restartu sesji/poczty z wyborem options i ewentualne
   doklejenie raportów do nowej sesji. Nie przedstawiono tego jako potwierdzonego
   obejścia weryfikacji.

## Pozostałe ustalenia poza poprawkami tego zadania

- Potwierdzono zgłoszoną przez audytora konstrukcję scratch ze `spec.id`
  (`src/lean/mod.rs:3870`) i brak walidacji alfabetu ID w `tasks.rs::validate`.
  Ograniczenie ID i wewnętrzne identyfikatory ścieżek wymagają osobnej poprawki;
  nie wykonywano próby zapisu poza worktree.
- Raporty zależności są wstawiane wprost przez `upstream_note`, a wybór ma moc
  instrukcji dla writera. Podtrzymano zgłoszone ryzyko prompt injection;
  zalecane jawne oznaczenie raportów jako danych i rozdzielenie uprawnień od
  decyzji. Brak dowodu obejścia sandboxa.
- Standardowe clippy jest zielone; istniejące `allow` nie dowodzą braku
  wszystkich ostrzeżeń. Ustalenia static-analyzer pozostają osobnym zakresem.
- Brak konfliktu przeczytanych instrukcji projektu z zadaniem. Nie wykonywano
  skryptów zmieniających konfigurację, nie tworzono commita, nie wysyłano danych
  do usług. Dalsza zielona atestacja wymaga dostępu/wykonania poza obecnymi
  ograniczeniami oraz rzeczywistych wyników modeli; zatrzymano się na tej granicy.
